pub mod signer;
pub use signer::EvmSigner;

use crate::chains::types::ChainId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

// keccak256("SwapCreated(bytes32,address,address,address,uint256,bytes20,uint256)")
pub const TOPIC_SWAP_CREATED: &str =
    "0x6095347ebb4084720e12a89ad2c70fc169534308159a6884c677a6ec9a30742c";

// keccak256("SwapClaimed(bytes32,bytes,address)")
pub const TOPIC_SWAP_CLAIMED: &str =
    "0xd14278a0c2b5ef6c1c6418dba05a4ebd99536912416835fde879006db64b8d06";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseSwapOrder {
    pub swap_id: String,
    pub sender: String,
    pub beneficiary: String,
    pub token: String,
    pub amount_usdc: f64,
    pub hash160: [u8; 20],
    pub timelock: u32,
    pub claimed: bool,
    pub refunded: bool,
    #[serde(default)]
    pub tx_hash: Option<String>,
}

pub type EvmSwapOrder = BaseSwapOrder;

/// Generic on-chain watcher for EVM chains (Base, Arbitrum, Ethereum, Polygon, etc.)
pub struct EvmWatcher {
    chain_id: ChainId,
    rpc_url: String,
    contract_addr: String,
    http_client: reqwest::Client,
    tx_events: broadcast::Sender<BaseSwapOrder>,
    last_scanned_order_block: std::sync::atomic::AtomicU64,
    last_scanned_claim_block: std::sync::atomic::AtomicU64,
}

impl EvmWatcher {
    pub fn new(
        chain_id: ChainId,
        rpc_url: String,
        contract_addr: String,
        tx_events: broadcast::Sender<BaseSwapOrder>,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("TRUSWAP-Native/1.0")
            .build()
            .unwrap_or_default();

        Self {
            chain_id,
            rpc_url,
            contract_addr,
            http_client,
            tx_events,
            last_scanned_order_block: std::sync::atomic::AtomicU64::new(0),
            last_scanned_claim_block: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Sends JSON-RPC payload with automatic failover across public Base endpoints
    async fn send_rpc_with_failover(&self, payload: &serde_json::Value) -> Result<serde_json::Value, String> {
        let mut endpoints = vec![self.rpc_url.as_str()];
        let fallbacks = if self.chain_id.as_u64() == 84532 {
            crate::chains::evm::signer::EvmSigner::SEPOLIA_FALLBACK_RPCS
        } else {
            crate::chains::evm::signer::EvmSigner::MAINNET_FALLBACK_RPCS
        };
        for fb in fallbacks {
            if *fb != self.rpc_url {
                endpoints.push(*fb);
            }
        }

        let mut last_err = String::new();
        for ep in endpoints {
            match self.http_client.post(ep).json(payload).send().await {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        if body.get("error").is_none() {
                            return Ok(body);
                        } else {
                            last_err = format!("RPC error from {}: {:?}", ep, body.get("error"));
                        }
                    } else {
                        last_err = format!("Invalid JSON from {}", ep);
                    }
                }
                Err(e) => {
                    last_err = format!("Connection error to {}: {}", ep, e);
                }
            }
        }
        Err(last_err)
    }

    /// Fetches latest block number from EVM RPC
    pub async fn fetch_block_number(&self) -> Result<u64, String> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "eth_blockNumber",
            "params": []
        });
        let body = self.send_rpc_with_failover(&payload).await?;
        let hex_str = body.get("result").and_then(|r| r.as_str()).unwrap_or("0x0");
        let clean = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        u64::from_str_radix(clean, 16).map_err(|e| e.to_string())
    }

    /// Queries eth_getLogs in safe <= 150 block chunks to stay strictly within public Base RPC range limits.
    async fn fetch_logs_in_chunks(&self, from_block: u64, to_block: u64, topic: &str) -> Vec<serde_json::Value> {
        let chunk_size = 150u64;
        let mut curr_from = from_block;
        let mut all_logs = Vec::new();

        while curr_from <= to_block {
            let curr_to = (curr_from + chunk_size - 1).min(to_block);
            let payload = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "eth_getLogs",
                "params": [{
                    "address": self.contract_addr,
                    "fromBlock": format!("0x{:x}", curr_from),
                    "toBlock": format!("0x{:x}", curr_to),
                    "topics": [topic]
                }]
            });

            match self.send_rpc_with_failover(&payload).await {
                Ok(body) => {
                    if let Some(logs) = body.get("result").and_then(|r| r.as_array()) {
                        all_logs.extend(logs.clone());
                    }
                }
                Err(e) => {
                    warn!(
                        "[{}] eth_getLogs chunk [0x{:x}..0x{:x}] failed across all RPCs: {}",
                        self.chain_id.display_name(),
                        curr_from,
                        curr_to,
                        e
                    );
                }
            }

            if curr_to >= to_block {
                break;
            }
            curr_from = curr_to + 1;
        }

        all_logs
    }

    /// Fetches all active swap orders from HTLC contract using chunked log scanning.
    /// Strictly requires at least 3 confirmations on Base L2 before notifying engine, protecting seller against micro-reorgs.
    pub async fn fetch_active_orders(&self) -> Result<Vec<BaseSwapOrder>, String> {
        let tip = self.fetch_block_number().await.unwrap_or(48_000_000);
        let safe_tip = tip.saturating_sub(3); // 3-block confirmation depth on Base L2
        let last = self.last_scanned_order_block.load(std::sync::atomic::Ordering::Relaxed);
        let from_block = if last == 0 {
            // Initial scan: look back ~1800 blocks (~1 hour) in chunks of 150 blocks
            safe_tip.saturating_sub(1800)
        } else {
            // Subsequent polls: re-check last 20 blocks for safety up to safe_tip
            last.saturating_sub(20)
        };
        let to_block = safe_tip;

        let logs = self.fetch_logs_in_chunks(from_block, to_block, TOPIC_SWAP_CREATED).await;
        self.last_scanned_order_block.store(safe_tip, std::sync::atomic::Ordering::Relaxed);

        let mut orders = Vec::new();
        for log in logs {
            if let Some(order) = self.parse_created_log(&log).await {
                if !order.claimed && !order.refunded {
                    orders.push(order);
                }
            }
        }

        Ok(orders)
    }

    /// Fetches all claimed events from HTLC to extract revealed preimages using chunked log scanning.
    pub async fn fetch_claimed_preimages(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        let tip = self.fetch_block_number().await.unwrap_or(48_000_000);
        let last = self.last_scanned_claim_block.load(std::sync::atomic::Ordering::Relaxed);
        let from_block = if last == 0 {
            tip.saturating_sub(1800)
        } else {
            last.saturating_sub(20)
        };
        let to_block = tip;

        let logs = self.fetch_logs_in_chunks(from_block, to_block, TOPIC_SWAP_CLAIMED).await;
        self.last_scanned_claim_block.store(tip, std::sync::atomic::Ordering::Relaxed);

        let mut claims = Vec::new();
        for log in logs {
            if let Some((swap_id, preimage)) = self.parse_claimed_log(&log) {
                claims.push((swap_id, preimage));
            }
        }
        Ok(claims)
    }

    fn parse_claimed_log(&self, log: &serde_json::Value) -> Option<(String, Vec<u8>)> {
        let topics = log.get("topics")?.as_array()?;
        if topics.len() < 2 {
            return None;
        }
        let swap_id = topics.get(1)?.as_str()?.to_string();
        let data = log.get("data")?.as_str()?.strip_prefix("0x")?;
        // ABI encoding for bytes secret:
        // data[0..64]: offset (0x20 = 32)
        // data[64..128]: length (0x20 = 32 bytes)
        // data[128..192]: 32-byte secret
        if data.len() < 192 {
            return None;
        }
        let len_hex = &data[64..128];
        let preimage_len = usize::from_str_radix(len_hex, 16).unwrap_or(0);
        if preimage_len > 0 && data.len() >= 128 + preimage_len * 2 {
            let preimage_hex = &data[128..128 + preimage_len * 2];
            if let Ok(bytes) = hex::decode(preimage_hex) {
                return Some((swap_id, bytes));
            }
        }
        None
    }

    async fn parse_created_log(&self, log: &serde_json::Value) -> Option<BaseSwapOrder> {
        let topics = log.get("topics")?.as_array()?;
        if topics.len() < 4 {
            return None;
        }

        let swap_id = topics.get(1)?.as_str()?.to_string();
        let sender_topic = topics.get(2)?.as_str()?;
        let sender = format!("0x{}", &sender_topic[sender_topic.len().saturating_sub(40)..]);

        let beneficiary_topic = topics.get(3)?.as_str()?;
        let beneficiary = format!("0x{}", &beneficiary_topic[beneficiary_topic.len().saturating_sub(40)..]);

        let data = log.get("data")?.as_str()?.strip_prefix("0x")?;
        if data.len() < 256 {
            return None;
        }

        let token = format!("0x{}", &data[24..64]);
        let amount_hex = &data[64..128];
        let amount_atoms = u64::from_str_radix(amount_hex, 16).unwrap_or(0);
        let amount_usdc = amount_atoms as f64 / 1_000_000.0;

        let hash160_hex = &data[128..168];
        let mut hash160 = [0u8; 20];
        if let Ok(bytes) = hex::decode(hash160_hex) {
            if bytes.len() == 20 {
                hash160.copy_from_slice(&bytes);
            }
        }

        let timelock_hex = &data[192..256];
        let timelock = u32::from_str_radix(timelock_hex, 16).unwrap_or(0);

        let (claimed, refunded) = self.query_swap_status(&swap_id).await.unwrap_or((false, false));
        let tx_hash = log.get("transactionHash").and_then(|h| h.as_str()).map(|s| s.to_string());

        Some(BaseSwapOrder {
            swap_id,
            sender,
            beneficiary,
            token,
            amount_usdc,
            hash160,
            timelock,
            claimed,
            refunded,
            tx_hash,
        })
    }

    /// Queries on-chain status of a specific swapId in HTLC contract.
    pub async fn query_swap_status(&self, swap_id: &str) -> Result<(bool, bool), String> {
        let clean_id = swap_id.strip_prefix("0x").unwrap_or(swap_id);
        let call_data = format!("0xeb84e7f2{:0>64}", clean_id);

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "eth_call",
            "params": [{
                "to": self.contract_addr,
                "data": call_data
            }, "latest"]
        });

        let resp = self
            .http_client
            .post(&self.rpc_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("eth_call failed: {}", e))?;

        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let res_hex = body
            .get("result")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .strip_prefix("0x")
            .unwrap_or("");

        if res_hex.len() >= 512 {
            let claimed_word = &res_hex[384..448];
            let refunded_word = &res_hex[448..512];
            let claimed = claimed_word.ends_with('1');
            let refunded = refunded_word.ends_with('1');
            return Ok((claimed, refunded));
        }

        Ok((false, false))
    }

    /// Background task: Polls for newly created swaps and revealed secrets.
    pub async fn start_background_polling(
        self: Arc<Self>,
        orderbook: Arc<crate::swap::orderbook::Orderbook>,
        swap_engine: Arc<crate::swap::engine::SwapEngine>,
    ) {
        tokio::spawn(async move {
            info!("[{}] On-chain HTLC watcher loop started", self.chain_id.display_name());
            let mut seen_claimed_ids = std::collections::HashSet::<String>::new();
            loop {
                // 1. Scan for newly created swap orders
                match self.fetch_active_orders().await {
                    Ok(orders) => {
                        for order in orders {
                            orderbook.upsert_base_order(order.clone()).await;
                            swap_engine.handle_base_swap_detected(&order).await;
                            let _ = self.tx_events.send(order);
                        }
                    }
                    Err(e) => {
                        warn!("[{}] Watcher order poll error: {}", self.chain_id.display_name(), e);
                    }
                }

                // 2. Scan for revealed secrets / preimages
                match self.fetch_claimed_preimages().await {
                    Ok(claims) => {
                        for (swap_id, preimage) in claims {
                            orderbook.mark_base_swap_claimed(&swap_id).await;
                            let h160 = crate::core::hash160(&preimage);
                            orderbook.mark_base_swap_claimed(&hex::encode(h160)).await;
                            if seen_claimed_ids.insert(swap_id.to_lowercase()) {
                                swap_engine.handle_base_preimage_revealed(&swap_id, &preimage).await;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("[{}] Watcher claim poll error: {}", self.chain_id.display_name(), e);
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(12)).await;
            }
        });
    }
}

/// Convenience alias for Base Sepolia
pub type BaseWatcher = EvmWatcher;

#[cfg(test)]
mod tests {
    use sha3::{Digest, Keccak256};

    fn selector(sig: &str) -> String {
        let hash = Keccak256::digest(sig.as_bytes());
        hex::encode(&hash[..4])
    }

    #[test]
    fn test_print_evm_selectors() {
        println!("createSwap: {}", selector("createSwap(address,address,uint256,bytes20,uint256,bytes32)"));
        println!("claim: {}", selector("claim(bytes32,bytes)"));
        println!("refund: {}", selector("refund(bytes32)"));
        println!("swaps: {}", selector("swaps(bytes32)"));
        println!("approve: {}", selector("approve(address,uint256)"));
        println!("transfer: {}", selector("transfer(address,uint256)"));
        println!("balanceOf: {}", selector("balanceOf(address)"));
    }
}
