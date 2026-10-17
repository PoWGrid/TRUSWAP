pub mod base_watcher;
pub mod client;
pub mod crypto;
pub mod protection;
pub mod protocol;
pub mod server;
pub mod state;
pub mod utxo_guard;

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

use crate::config::{Config, RelayMode};
use crate::relay::base_watcher::BaseWatcher;
use crate::relay::protection::{tune_kernel_limits, ConnectionGuard};
use crate::relay::server::{create_relay_router, RelayServerContext};
use crate::relay::state::HotOrderbookState;
use crate::relay::utxo_guard::UtxoGuard;
use crate::tru_p2p::client::TruP2PNode;

/// Primary entry point for running the high-throughput TRUSwap Relay service
pub async fn run_relay_service(cfg: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Elevate OS limits (1,048,576 file descriptors on Linux)
    tune_kernel_limits();

    let mode_str = match cfg.relay_mode {
        RelayMode::Blazing => "BLAZING (Low-Traffic Dev Profile)".to_string(),
        RelayMode::UltraBlazing => "ULTRABLAZING (Highload 1M Scale Profile)".to_string(),
    };

    info!("========================================================");
    info!("   Starting Native TRUSwap Orderbook Relay Service      ");
    info!("   Mode: {}", mode_str);
    info!("   Binding Address: {}", cfg.bind_addr);
    info!("   Base RPC: {}", cfg.base_rpc_url);
    info!("   Base WSS: {}", cfg.base_wss_url);
    info!("   Max Client Connections: {}", cfg.max_relay_clients);
    info!("========================================================");

    // 2. Initialize in-memory hot orderbook state
    let orderbook = HotOrderbookState::new();

    // 3. Initialize connection guard with per-IP quotas
    let guard = ConnectionGuard::new(cfg.max_relay_clients);

    // 4. Initialize in-memory TRU UTXO guard & load cached chain
    let utxo_guard = UtxoGuard::new();
    let cache_path = cfg.data_dir.join("tru.dat");
    utxo_guard.load_from_cache(&cache_path).await;

    // 5. Start TRU P2P node for real-time live block streams with compressed peers.dat persistence
    let p2p_node = Arc::new(TruP2PNode::new_with_data_dir(cfg.tru_seeds.clone(), Some(cfg.data_dir.clone())));
    let _p2p_handle = p2p_node.clone().start();
    let _utxo_sync_handle = utxo_guard
        .clone()
        .start_p2p_sync(p2p_node.clone(), orderbook.clone());

    // 6. Initialize Base watcher (blazing or ultrablazing)
    let base_watcher = BaseWatcher::new(&cfg);
    let _base_handle = base_watcher.clone().start(orderbook.clone());

    // 7. Assemble server context and create Axum router
    let ctx = RelayServerContext {
        orderbook: orderbook.clone(),
        guard,
        utxo_guard,
        base_watcher,
        mode_str,
    };

    let router = create_relay_router(ctx);

    // 8. Bind TCP listener and serve incoming WebSocket and HTTP requests
    let listener = TcpListener::bind(&cfg.bind_addr).await?;
    info!("[RELAY-READY] Relay listening on ws://{}/orderbook", cfg.bind_addr);

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
