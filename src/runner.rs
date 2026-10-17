use std::sync::Arc;
use tokio::sync::broadcast;
use crate::config::Config;
use crate::tui::TuiManager;
use crate::{api, chains, relay, swap, tru_p2p, wallet};

pub async fn run_node(config: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut listener_opt = None;
    let current_addr = config.bind_addr.clone();

    match tokio::net::TcpListener::bind(&current_addr).await {
        Ok(l) => {
            listener_opt = Some(l);
        }
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if let Ok(mut socket_addr) = current_addr.parse::<std::net::SocketAddr>() {
                for offset in 1..=10 {
                    let next_port = socket_addr.port() + offset;
                    socket_addr.set_port(next_port);
                    if let Ok(l) = tokio::net::TcpListener::bind(socket_addr).await {
                        listener_opt = Some(l);
                        break;
                    }

                }
            }
            if listener_opt.is_none() {
                return Err(Box::new(e));
            }
        }
        Err(e) => return Err(Box::new(e)),
    }
    let listener = listener_opt.unwrap();
    run_node_with_listener(config, listener).await
}

pub async fn run_node_with_listener(
    config: Config,
    listener: tokio::net::TcpListener,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if config.is_relay {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "info,truswap=info".to_string()),
            )
            .try_init();
        relay::run_relay_service(config).await?;
        return Ok(());
    }

    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "info,truswap=info".to_string()),
            )
            .try_init();
    }

    let local_addr = listener.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| config.bind_addr.clone());
    let session_token = hex::encode(rand::random::<[u8; 32]>());
    let ui_url = format!("http://{}", local_addr);

    let orderbook = Arc::new(swap::orderbook::Orderbook::new());

    // Initialize TRU P2P Client with compressed peers.dat persistence
    let tru_p2p = Arc::new(tru_p2p::TruP2PNode::new_with_data_dir(config.tru_seeds.clone(), Some(config.data_dir.clone())));
    tru_p2p.clone().start();

    // Encrypted and compressed binary storage for TRU wallet and atomic swap preimages
    let vault = Arc::new(tokio::sync::Mutex::new(
        wallet::keystore::EncryptedVault::open(&config.data_dir).expect("Failed to initialize encrypted binary vault"),
    ));

    // TRU Client
    let tru_client = Arc::new(wallet::vault::TruLiteClient::new(
        tru_p2p.clone(),
        vault.clone(),
    ));

    // Get initial vault address from encrypted binary vault
    let initial_vault = tru_client.get_or_create_swap_address().await;

    // Initialize TUI in alternate screen buffer
    let tui = Arc::new(TuiManager::new(
        ui_url.clone(),
        config.base_htlc_contract.clone(),
        initial_vault.clone(),
        config.network_name().to_string(),
    ));

    tui.log("TRUSWAP initializing...".to_string()).await;
    tui.log(format!("TRU P2P: seeds {:?}", config.tru_seeds)).await;
    tui.log(format!("{} HTLC: {}", config.network_name(), config.base_htlc_contract)).await;
    tui.log(format!("Vault initialized at {:?}", config.data_dir.join("vault.dat"))).await;

    let (tx_events, mut rx_events) = broadcast::channel(100);

    let watcher = Arc::new(chains::evm::EvmWatcher::new(
        config.chain_id(),
        config.base_rpc_url.clone(),
        config.base_htlc_contract.clone(),
        tx_events.clone(),
    ));

    // Initialize Autonomous Swap Engine
    let swap_engine = Arc::new(swap::engine::SwapEngine::new(
        vault.clone(),
        tru_client.clone(),
        orderbook.clone(),
        tui.clone(),
    ));

    // Initialize Headless EVM Signer for Base Wallet
    let base_priv_opt = {
        let mut v = vault.lock().await;
        v.get_base_priv_key().ok()
    };
    if let Some(base_priv) = base_priv_opt {
        match crate::chains::evm::EvmSigner::new(
            base_priv,
            config.chain_id().as_u64(),
            config.base_rpc_url.clone(),
            config.base_htlc_contract.clone(),
            config.base_usdc_contract.clone(),
        ) {
            Ok(signer) => {
                let evm_signer = Arc::new(signer);
                swap_engine.set_evm_signer(evm_signer.clone()).await;
                tui.log(format!("{} Vault active: {}", config.network_name(), evm_signer.address())).await;
            }
            Err(e) => {
                tui.log(format!("⚠️ Failed to initialize {} Vault: {}", config.network_name(), e)).await;
            }
        }
    }

    // Spawn watcher background polling loop (orders + claimed preimages)
    watcher.start_background_polling(orderbook.clone(), swap_engine.clone()).await;
    tui.log(format!("{} on-chain HTLC watcher loop started (Auto-Claim enabled)", config.network_name())).await;

    // Start background safety monitor loop for timeouts & refunds
    swap_engine.clone().start_background_safety_loop().await;

    // UI events broadcast channel for web frontend showcase
    let (ui_events, _) = tokio::sync::broadcast::channel::<serde_json::Value>(512);
    tui.set_ui_events(ui_events.clone()).await;

    // Start background TRUSWAP Orderbook Relay client
    let relay_ws_url = config.relay_node_url.clone();
    tui.log(format!("Orderbook Relay node: {}", relay_ws_url)).await;
    let relay_client = crate::relay::client::RelayClient::start_background(
        relay_ws_url,
        orderbook.clone(),
        swap_engine.clone(),
        tui.clone(),
        ui_events.clone(),
    );

    // TUI event subscriber
    let tui_ev = tui.clone();
    let orderbook_ev = orderbook.clone();
    tokio::spawn(async move {
        let mut seen_swaps = std::collections::HashSet::new();
        while let Ok(order) = rx_events.recv().await {
            let active = orderbook_ev.get_active_orders().await;
            tui_ev.update_orders_count(active.len(), 0).await;
            if seen_swaps.insert(order.swap_id.clone()) {
                tui_ev
                    .log(format!(
                        "Base HTLC: {:.2} USDC locked (ID: 0x{}...)",
                        order.amount_usdc,
                        &order.swap_id.strip_prefix("0x").unwrap_or(&order.swap_id)[..8]
                    ))
                    .await;
            }
        }
    });

    // Start historical & real-time block scanner (continuous cryptographic SPV verification)
    let block_scanner = Arc::new(tru_p2p::BlockScanner::new(
        tru_client.p2p_node.clone(),
        orderbook.clone(),
    ));
    block_scanner.set_data_dir(config.data_dir.clone());
    if config.resync_from_genesis {
        block_scanner.set_resync_from_genesis(true);
    }
    block_scanner.clone().start();
    block_scanner.set_vault_address(&initial_vault).await;
    swap_engine.set_block_scanner(block_scanner.clone()).await;
    tui.log("TRU SPV Scanner started (Continuous chain verification)".to_string()).await;

    // Node state background updater
    let tui_node = tui.clone();
    let tru_node_bg = tru_client.clone();
    let scanner_bg = block_scanner.clone();
    tokio::spawn(async move {
        loop {
            let verified = scanner_bg.verified_height();
            let block = if verified > 0 {
                verified as u32
            } else {
                tru_node_bg.get_block_count().await
            };
            let balance = scanner_bg.vault_balance_atoms();
            tru_node_bg.set_balance_atoms(balance);
            let addr = if let Some(a) = scanner_bg.vault_address().await {
                a
            } else {
                tru_node_bg.get_or_create_swap_address().await
            };
            let p2p_peers = tru_node_bg.p2p_node.peer_count();
            tui_node.update_tru_state(block, balance, addr, p2p_peers).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        }
    });

    // TUI periodic render loop (every 500ms)
    let tui_render = tui.clone();
    tokio::spawn(async move {
        loop {
            tui_render.render().await;
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    });

    let config_arc = Arc::new(config.clone());
    let app_state = api::routes::AppState {
        config: config_arc,
        session_token,
        orderbook,
        tru_client,
        tx_events,
        tui: tui.clone(),
        vault,
        swap_engine,
        block_scanner,
        ui_events,
        relay_client,
    };

    let app = api::routes::create_router(app_state);
    tui.log(format!("Web UI listening on http://{}", local_addr)).await;

    // Listen for Ctrl+C for graceful terminal restoration
    tokio::select! {
        res = axum::serve(listener, app) => {
            if let Err(e) = res {
                TuiManager::restore_terminal();
                eprintln!("Server error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            TuiManager::restore_terminal();
            println!("\nTRUSWAP stopped cleanly.");
        }
    }

    TuiManager::restore_terminal();
    Ok(())
}
