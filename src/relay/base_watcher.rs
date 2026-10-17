use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::Value;
use tokio::sync::{RwLock, Semaphore};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::config::{Config, RelayMode};
use crate::relay::protocol::RelayOrderSide;
use crate::relay::state::HotOrderbookState;

pub struct BaseWatcher {
    http_client: Client,
    rpc_url: String,
    wss_url: String,
    usdc_contract: String,
    min_eth_gas_wei: u128,
    relay_mode: RelayMode,
    is_testnet: bool,
    balance_cache: Arc<RwLock<HashMap<String, (Instant, u128, f64, u128)>>>,
    rpc_semaphore: Arc<Semaphore>,
    last_gas_price_wei: Arc<RwLock<u128>>,
}

impl BaseWatcher {
    pub fn new(cfg: &Config) -> Arc<Self> {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("TRUSWAP-Relay/1.0")
            .build()
            .unwrap_or_default();

        // 1 ETH = 10^18 Wei
        let min_eth_gas_wei = (cfg.min_eth_gas * 1e18) as u128;

        Arc::new(Self {
            http_client,
            rpc_url: cfg.base_rpc_url.clone(),
            wss_url: cfg.base_wss_url.clone(),
            usdc_contract: cfg.base_usdc_contract.clone(),
            min_eth_gas_wei,
            relay_mode: cfg.relay_mode,
            is_testnet: cfg.is_testnet,
            balance_cache: Arc::new(RwLock::new(HashMap::new())),
            rpc_semaphore: Arc::new(Semaphore::new(6)),
            last_gas_price_wei: Arc::new(RwLock::new(20_000_000)), // 0.02 Gwei default
        })
    }

    /// Dispatches a JSON-RPC payload trying primary RPC and fallback endpoints
    async fn send_rpc(&self, payload: &Value) -> Result<Value, String> {
        let mut fallbacks = vec![self.rpc_url.as_str()];
        let defaults = if self.is_testnet {
            crate::config::BASE_SEPOLIA_FALLBACK_RPCS
        } else {
            crate::config::BASE_MAINNET_FALLBACK_RPCS
        };
        for fb in defaults {
            if *fb != self.rpc_url.as_str() {
                fallbacks.push(*fb);
            }
        }

        let mut last_err = String::new();
        for ep in fallbacks {
            match self.http_client.post(ep).json(payload).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<Value>().await {
                            return Ok(body);
                        }
                    } else {
                        last_err = format!("HTTP error {} on {}", resp.status(), ep);
                    }
                }
                Err(e) => {
                    last_err = format!("Connection error on {}: {}", ep, e);
                }
            }
        }
        Err(format!("All Base RPC endpoints failed: {}", last_err))
    }

    /// Fetches native ETH balance of an address in Wei
    pub async fn get_eth_balance(&self, address: &str) -> Result<u128, String> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getBalance",
            "params": [address, "latest"]
        });

        let body = self.send_rpc(&payload).await?;
        let hex_str = body
            .get("result")
            .and_then(|r| r.as_str())
            .ok_or_else(|| "Missing result in eth_getBalance".to_string())?;

        let clean = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        u128::from_str_radix(clean, 16).map_err(|e| format!("Invalid hex balance: {}", e))
    }

    /// Fetches ERC20 USDC balance of an address (6 decimals)
    pub async fn get_usdc_balance(&self, address: &str) -> Result<f64, String> {
        let clean_addr = address.strip_prefix("0x").unwrap_or(address);
        let call_data = format!("0x70a08231{:0>64}", clean_addr.to_lowercase());

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "eth_call",
            "params": [{
                "to": self.usdc_contract,
                "data": call_data
            }, "latest"]
        });

        let body = self.send_rpc(&payload).await?;
        let hex_str = body
            .get("result")
            .and_then(|r| r.as_str())
            .ok_or_else(|| "Missing result in eth_call balanceOf".to_string())?;

        let clean = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        let units = u128::from_str_radix(clean, 16).unwrap_or(0);
        Ok(units as f64 / 1_000_000.0)
    }

    /// Fetches native ETH balance (in Wei), ERC20 USDC balance (6 decimals), and dynamic network gas fee threshold (in Wei)
    /// in a single batched JSON-RPC HTTP request with short-lived memory caching and rate limit protection.
    pub async fn get_balances_batch(&self, address: &str) -> Result<(u128, f64, u128), String> {
        let clean_addr = address.strip_prefix("0x").unwrap_or(address).to_lowercase();

        // 1. Check TTL cache (valid for 3 seconds)
        {
            let cache = self.balance_cache.read().await;
            if let Some((instant, eth_wei, usdc_bal, min_gas_wei)) = cache.get(&clean_addr) {
                if instant.elapsed() < Duration::from_secs(3) {
                    return Ok((*eth_wei, *usdc_bal, *min_gas_wei));
                }
            }
        }

        // 2. Acquire permit to prevent RPC rate limiting
        let _permit = self.rpc_semaphore.acquire().await.map_err(|e| e.to_string())?;

        let call_data = format!("0x70a08231{:0>64}", clean_addr);
        let batch_payload = serde_json::json!([
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "eth_getBalance",
                "params": [format!("0x{}", clean_addr), "latest"]
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "eth_call",
                "params": [{
                    "to": self.usdc_contract,
                    "data": call_data
                }, "latest"]
            },
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "eth_gasPrice",
                "params": []
            }
        ]);

        let body = self.send_rpc(&batch_payload).await?;

        let items = body.as_array().ok_or_else(|| "Batch RPC response is not an array".to_string())?;
        let mut eth_res: Option<u128> = None;
        let mut usdc_res: Option<f64> = None;
        let mut gas_price_res: Option<u128> = None;

        for item in items {
            let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(err) = item.get("error") {
                return Err(format!("RPC error for id {}: {}", id, err));
            }
            if let Some(hex_str) = item.get("result").and_then(|r| r.as_str()) {
                let clean = hex_str.strip_prefix("0x").unwrap_or(hex_str);
                if id == 1 {
                    let wei = u128::from_str_radix(clean, 16).unwrap_or(0);
                    eth_res = Some(wei);
                } else if id == 2 {
                    let units = u128::from_str_radix(clean, 16).unwrap_or(0);
                    usdc_res = Some(units as f64 / 1_000_000.0);
                } else if id == 3 {
                    let gp = u128::from_str_radix(clean, 16).unwrap_or(20_000_000); // 0.02 Gwei default
                    gas_price_res = Some(gp);
                }
            }
        }

        let eth_wei = eth_res.ok_or_else(|| "Missing eth_getBalance in batch response".to_string())?;
        let usdc_bal = usdc_res.ok_or_else(|| "Missing USDC balanceOf in batch response".to_string())?;
        let gas_price_wei = gas_price_res.unwrap_or(20_000_000); // 0.02 Gwei fallback

        *self.last_gas_price_wei.write().await = gas_price_wei;

        // Dynamically compute minimum required gas on Base:
        // L2 Execution fee (240k gas) + L1 Data blob fee (~10k Gwei / ~0.000010 ETH) with 25% safety margin
        let l2_cost = 240_000u128 * gas_price_wei;
        let l1_cost = 10_000_000_000_000u128; // ~0.000010 ETH typical L1 blob overhead on Base
        let min_gas_wei = (l2_cost + l1_cost) * 125 / 100;

        // 3. Update cache
        {
            let mut cache = self.balance_cache.write().await;
            cache.insert(clean_addr, (Instant::now(), eth_wei, usdc_bal, min_gas_wei));
        }

        Ok((eth_wei, usdc_bal, min_gas_wei))
    }

    /// Dynamically computes minimum required gas to claim USDC on Base:
    /// L2 Execution fee (~140k gas) + L1 Data blob fee (~5k Gwei / ~0.000005 ETH) with 25% safety margin
    pub async fn get_claim_gas_threshold(&self) -> u128 {
        let gp = *self.last_gas_price_wei.read().await;
        let l2_cost = 140_000u128 * gp;
        let l1_cost = 5_000_000_000_000u128; // ~0.000005 ETH typical L1 blob overhead for claim
        (l2_cost + l1_cost) * 125 / 100
    }

    /// Checks if a maker has sufficient ETH for gas and USDC for any active BUY_TRU orders
    pub async fn verify_maker_balances(
        &self,
        orderbook: &HotOrderbookState,
        maker_base_address: &str,
    ) {
        let order_ids = orderbook.get_orders_by_base_maker(maker_base_address).await;
        if order_ids.is_empty() {
            return;
        }

        // Check if this maker has any active unmatched BUY_TRU orders (locking Base USDC)
        let orders_map = orderbook.orders.read().await;
        let mut max_usdc_needed = 0.0f64;
        let mut has_buy_tru = false;

        for id in &order_ids {
            if let Some(order) = orders_map.get(id) {
                if order.side == RelayOrderSide::BuyTru && !order.is_matched {
                    has_buy_tru = true;
                    max_usdc_needed += order.amount_usdc;
                }
            }
        }
        drop(orders_map);

        // If maker has no active unmatched BUY_TRU orders, Base balance checks do not apply
        if !has_buy_tru {
            return;
        }

        // 1. Verify ETH gas balance for BUY_TRU orders
        match self.get_eth_balance(maker_base_address).await {
            Ok(wei) => {
                if wei < self.min_eth_gas_wei {
                    info!(
                        "[BASE-WATCHER] Maker {} ETH balance ({} Wei) below gas threshold ({} Wei), pruning {} buy orders",
                        maker_base_address, wei, self.min_eth_gas_wei, order_ids.len()
                    );
                    for id in &order_ids {
                        let orders_lock = orderbook.orders.read().await;
                        let is_unmatched_buy = orders_lock
                            .get(id)
                            .map(|o| o.side == RelayOrderSide::BuyTru && !o.is_matched)
                            .unwrap_or(false);
                        drop(orders_lock);
                        if is_unmatched_buy {
                            orderbook.remove_order(id, "Insufficient ETH for gas on Base").await;
                        }
                    }
                    return;
                }
            }
            Err(e) => {
                debug!("[BASE-WATCHER] Could not check ETH balance for {}: {}", maker_base_address, e);
            }
        }

        // 2. Verify USDC balance for BUY_TRU orders
        match self.get_usdc_balance(maker_base_address).await {
            Ok(usdc_available) => {
                if usdc_available < max_usdc_needed {
                    info!(
                        "[BASE-WATCHER] Maker {} USDC balance ({:.2}) below required ({:.2}), pruning orders",
                        maker_base_address, usdc_available, max_usdc_needed
                    );
                    for id in &order_ids {
                        let orders_lock = orderbook.orders.read().await;
                        let is_unmatched_buy = orders_lock
                            .get(id)
                            .map(|o| o.side == RelayOrderSide::BuyTru && !o.is_matched)
                            .unwrap_or(false);
                        drop(orders_lock);
                        if is_unmatched_buy {
                            orderbook.remove_order(id, "Insufficient USDC balance on Base").await;
                        }
                    }
                }
            }
            Err(e) => {
                debug!("[BASE-WATCHER] Could not check USDC balance for {}: {}", maker_base_address, e);
            }
        }
    }

    /// Fetches block details with transactions by block number hex
    async fn fetch_block_transactions(&self, block_hex: &str) -> Result<Vec<(String, String)>, String> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "eth_getBlockByNumber",
            "params": [block_hex, true]
        });

        let body = self.send_rpc(&payload).await.map_err(|e| format!("Failed to fetch block: {}", e))?;
        let mut tx_participants = Vec::new();

        if let Some(txs) = body.get("result").and_then(|r| r.get("transactions")).and_then(|t| t.as_array()) {
            for tx in txs {
                let from = tx.get("from").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                let to = tx.get("to").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                if !from.is_empty() {
                    tx_participants.push((from, to));
                }
            }
        }
        Ok(tx_participants)
    }

    /// Starts the Base watcher service based on the configured RelayMode
    pub fn start(self: Arc<Self>, orderbook: Arc<HotOrderbookState>) -> tokio::task::JoinHandle<()> {
        match self.relay_mode {
            RelayMode::Blazing => self.start_blazing_polling(orderbook),
            RelayMode::UltraBlazing => self.start_ultrablazing_streaming(orderbook),
        }
    }

    /// Lightweight dev profile: batch polls active makers every 5-10s
    fn start_blazing_polling(
        self: Arc<Self>,
        orderbook: Arc<HotOrderbookState>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!("[BASE-WATCHER] Starting --blazing polling loop (5s interval)");
            let mut interval = tokio::time::interval(Duration::from_secs(5));

            loop {
                interval.tick().await;

                let active_makers: Vec<String> = {
                    let set = orderbook.active_evm_addresses.read().await;
                    set.iter().cloned().collect()
                };

                for maker in active_makers {
                    self.verify_maker_balances(&orderbook, &maker).await;
                }
            }
        })
    }

    /// Highload production profile: streams blocks via WebSocket with O(1) inverted RAM check
    fn start_ultrablazing_streaming(
        self: Arc<Self>,
        orderbook: Arc<HotOrderbookState>,
    ) -> tokio::task::JoinHandle<()> {
        let self_sweep = Arc::clone(&self);
        let ob_sweep = Arc::clone(&orderbook);

        // Background sweep every 30s to catch edge-case transfers
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let active_makers: Vec<String> = {
                    let set = ob_sweep.active_evm_addresses.read().await;
                    set.iter().cloned().collect()
                };
                for maker in active_makers {
                    self_sweep.verify_maker_balances(&ob_sweep, &maker).await;
                }
            }
        });

        tokio::spawn(async move {
            info!("[BASE-WATCHER] Starting --ultrablazing real-time WSS stream to {}", self.wss_url);

            loop {
                match connect_async(&self.wss_url).await {
                    Ok((ws_stream, _)) => {
                        info!("[BASE-WATCHER] Connected to Base WebSocket. Subscribing to newHeads...");
                        let (mut write, mut read) = ws_stream.split();

                        let sub_payload = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "eth_subscribe",
                            "params": ["newHeads"]
                        });

                        if let Err(e) = write.send(Message::Text(sub_payload.to_string().into())).await {
                            error!("[BASE-WATCHER] Failed to send newHeads subscription: {}", e);
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            continue;
                        }

                        while let Some(msg_result) = read.next().await {
                            match msg_result {
                                Ok(Message::Text(text)) => {
                                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                        if let Some(params) = v.get("params") {
                                            if let Some(result) = params.get("result") {
                                                if let Some(block_hex) = result.get("number").and_then(|n| n.as_str()) {
                                                    debug!("[BASE-WATCHER] New Base block header: {}", block_hex);

                                                    // Fetch full block transactions
                                                    match self.fetch_block_transactions(block_hex).await {
                                                        Ok(participants) => {
                                                            let active_set = orderbook.active_evm_addresses.read().await;
                                                            let mut matched_makers = Vec::new();

                                                            // O(1) in-memory check for every transaction in the block
                                                            for (from, to) in participants {
                                                                if active_set.contains(&from) {
                                                                    matched_makers.push(from);
                                                                }
                                                                if active_set.contains(&to) {
                                                                    matched_makers.push(to);
                                                                }
                                                            }
                                                            drop(active_set);

                                                            // Verify matched makers
                                                            for maker in matched_makers {
                                                                info!("[BASE-WATCHER] O(1) Match: Active maker {} active in block {}", maker, block_hex);
                                                                self.verify_maker_balances(&orderbook, &maker).await;
                                                            }
                                                        }
                                                        Err(e) => {
                                                            debug!("[BASE-WATCHER] Failed to inspect block {}: {}", block_hex, e);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(Message::Ping(payload)) => {
                                    let _ = write.send(Message::Pong(payload)).await;
                                }
                                Ok(Message::Close(_)) => {
                                    warn!("[BASE-WATCHER] WebSocket closed by remote server, reconnecting...");
                                    break;
                                }
                                Err(e) => {
                                    warn!("[BASE-WATCHER] WebSocket error: {}, reconnecting...", e);
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        warn!("[BASE-WATCHER] Failed to connect to Base WSS ({}): {}. Retrying in 5s...", self.wss_url, e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        })
    }
}
