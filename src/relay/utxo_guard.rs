use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::relay::state::HotOrderbookState;
use crate::tru_p2p::chain_cache::load_cached_chain;
use crate::tru_p2p::client::TruP2PNode;
use crate::tru_p2p::scanner::parse_serialized_tx;
use crate::tru_p2p::wire::BlockProto;

/// Total fee buffer required for a TRU HTLC lock:
/// 250 atoms surplus in HTLC output (consumed as counterparty's claim tx fee)
/// + 250 atoms network miner fee for the lock transaction itself.
pub const HTLC_TOTAL_FEE_BUFFER_ATOMS: u64 = 500;
pub const HTLC_CLAIM_FEE_ATOMS: u64 = HTLC_TOTAL_FEE_BUFFER_ATOMS;

/// Derives a TRU address from a P2PKH scriptPubKey hex string
pub fn script_pubkey_to_tru_address(spk_hex: &str) -> Option<String> {
    let clean = spk_hex.trim().to_lowercase();
    if clean.starts_with("76a914") && clean.ends_with("88ac") && clean.len() == 50 {
        let pkh_hex = &clean[6..46];
        if let Ok(pkh_bytes) = hex::decode(pkh_hex) {
            if pkh_bytes.len() == 20 {
                let mut pkh = [0u8; 20];
                pkh.copy_from_slice(&pkh_bytes);
                return Some(crate::core::keys::pkh_to_tru_address(&pkh));
            }
        }
    }
    None
}

/// In-memory UTXO and balance validator for TRU network
pub struct UtxoGuard {
    /// In-memory balance index: TRU address -> confirmed balance in atoms
    pub balances: Arc<RwLock<HashMap<String, u64>>>,
    /// In-memory UTXO index: "txid:vout" -> (TRU address, amount_atoms)
    pub utxos: Arc<RwLock<HashMap<String, (String, u64)>>>,
    /// Highest applied block height
    pub tip_height: Arc<std::sync::atomic::AtomicU64>,
}

impl UtxoGuard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            balances: Arc::new(RwLock::new(HashMap::new())),
            utxos: Arc::new(RwLock::new(HashMap::new())),
            tip_height: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// Initializes in-memory state from disk cache (tru.dat)
    pub async fn load_from_cache(&self, cache_path: &Path) {
        if !cache_path.exists() {
            info!("[UTXO-GUARD] Cache file {:?} not found, starting fresh sync", cache_path);
            return;
        }

        info!("[UTXO-GUARD] Loading cached chain from {:?} into RAM...", cache_path);
        match load_cached_chain(cache_path, 0) {
            Ok(Some((blocks, tip_height, tip_hash))) => {
                let count = blocks.len();
                info!(
                    "[UTXO-GUARD] Parsing {} blocks from cache (tip #{} {})...",
                    count, tip_height, tip_hash
                );
                for blk in &blocks {
                    self.apply_block(blk, None).await;
                }
                self.tip_height.store(tip_height, std::sync::atomic::Ordering::SeqCst);
                let total_addrs = self.balances.read().await.len();
                let total_utxos = self.utxos.read().await.len();
                info!(
                    "[UTXO-GUARD] RAM state hydrated: {} active addresses, {} UTXOs indexed",
                    total_addrs, total_utxos
                );
            }
            Ok(None) => {
                warn!("[UTXO-GUARD] Cache file empty or corrupt, starting fresh");
            }
            Err(e) => {
                warn!("[UTXO-GUARD] Failed to load cached chain: {}", e);
            }
        }
    }

    /// Applies a block to the in-memory UTXO set and balance indices
    pub async fn apply_block(
        &self,
        blk: &BlockProto,
        orderbook: Option<&Arc<HotOrderbookState>>,
    ) {
        self.tip_height.fetch_max(blk.height as u64, std::sync::atomic::Ordering::Relaxed);
        let mut affected_addrs = Vec::new();

        for raw_tx in &blk.transactions {
            if let Some(parsed) = parse_serialized_tx(raw_tx) {
                // 1. Process inputs (spending prior outputs)
                if !parsed.is_coinbase {
                    for input in parsed.inputs {
                        let utxo_key = format!("{}:{}", input.prev_txid, input.prev_vout);
                        let removed = {
                            let mut utxos_lock = self.utxos.write().await;
                            utxos_lock.remove(&utxo_key)
                        };

                        if let Some((addr, spent_atoms)) = removed {
                            let mut balances_lock = self.balances.write().await;
                            if let Some(bal) = balances_lock.get_mut(&addr) {
                                *bal = bal.saturating_sub(spent_atoms);
                            }
                            affected_addrs.push(addr);
                        }
                    }
                }

                // 2. Process outputs (creating new UTXOs)
                for out in parsed.outputs {
                    if out.amount_atoms == 0 {
                        continue;
                    }

                    if let Some(addr) = script_pubkey_to_tru_address(&out.script_pubkey) {
                        let utxo_key = format!("{}:{}", parsed.txid, out.vout);
                        {
                            let mut utxos_lock = self.utxos.write().await;
                            utxos_lock.insert(utxo_key, (addr.clone(), out.amount_atoms));
                        }
                        {
                            let mut balances_lock = self.balances.write().await;
                            let entry = balances_lock.entry(addr.clone()).or_insert(0);
                            *entry = entry.saturating_add(out.amount_atoms);
                        }
                    }
                }
            }
        }

        // If orderbook state was provided, prune any orders where TRU balance became insufficient
        if let Some(ob) = orderbook {
            for addr in affected_addrs {
                let current_bal = self.get_balance(&addr).await;
                let active_order_ids = {
                    let tru_idx = ob.tru_maker_index.read().await;
                    tru_idx.get(&addr).cloned().unwrap_or_default()
                };

                for order_id in active_order_ids {
                    let orders_lock = ob.orders.read().await;
                    if let Some(order) = orders_lock.get(&order_id) {
                        if order.side == crate::relay::protocol::RelayOrderSide::SellTru && !order.is_matched {
                            let required_atoms =
                                (order.amount_tru * 100_000_000.0).round() as u64 + HTLC_TOTAL_FEE_BUFFER_ATOMS;
                            if current_bal < required_atoms {
                                drop(orders_lock);
                                info!(
                                    "[UTXO-GUARD] Maker {} TRU balance dropped to {} atoms (needs {}), pruning order {}",
                                    addr, current_bal, required_atoms, order_id
                                );
                                ob.remove_order(&order_id, "TRU UTXO spent or insufficient balance").await;
                            }
                        }
                    }
                }
            }
        }
    }

    /// O(1) in-memory balance verification for a TRU maker selling TRU
    pub async fn verify_tru_balance(
        &self,
        address: &str,
        required_atoms: u64,
    ) -> Result<u64, String> {
        let total_required = required_atoms.saturating_add(HTLC_TOTAL_FEE_BUFFER_ATOMS);
        let balance = self.get_balance(address).await;

        if balance >= total_required {
            Ok(balance)
        } else {
            Err(format!(
                "Insufficient TRU balance: available {:.8} TRU, required {:.8} TRU (+500 atoms fee buffer: 250 claim surplus + 250 tx fee)",
                balance as f64 / 100_000_000.0,
                total_required as f64 / 100_000_000.0
            ))
        }
    }

    /// Fast lookup of current confirmed balance for an address (in atoms)
    pub async fn get_balance(&self, address: &str) -> u64 {
        let addr = if address.len() == 66
            && (address.starts_with("02") || address.starts_with("03"))
        {
            if let Ok(b) = hex::decode(address) {
                if b.len() == 33 {
                    let mut arr = [0u8; 33];
                    arr.copy_from_slice(&b);
                    let pkh = crate::core::keys::hash160(&arr);
                    crate::core::keys::pkh_to_tru_address(&pkh)
                } else {
                    address.to_string()
                }
            } else {
                address.to_string()
            }
        } else {
            address.to_string()
        };
        let balances = self.balances.read().await;
        balances.get(&addr).copied().unwrap_or(0)
    }

    /// Background task that listens for real-time blocks and catches up missing history
    pub fn start_p2p_sync(
        self: Arc<Self>,
        p2p_node: Arc<TruP2PNode>,
        orderbook: Arc<HotOrderbookState>,
    ) -> tokio::task::JoinHandle<()> {
        let guard_sync = self.clone();
        let p2p_sync = p2p_node.clone();
        tokio::spawn(async move {
            info!("[UTXO-GUARD] Background P2P catch-up task active");
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
                let net_height = p2p_sync.current_height();
                let my_tip = guard_sync.tip_height.load(std::sync::atomic::Ordering::Relaxed);
                if net_height > my_tip {
                    let end = std::cmp::min(my_tip + 32, net_height);
                    let batch: Vec<u64> = (my_tip + 1..=end).collect();
                    if !batch.is_empty() {
                        debug!("[UTXO-GUARD] Catching up blocks {}..{} from P2P (net tip: #{})", batch[0], end, net_height);
                        p2p_sync.send_get_blocks_batch(&batch).await;
                    }
                }
            }
        });

        tokio::spawn(async move {
            info!("[UTXO-GUARD] Subscribing to live TRU P2P block stream...");
            let mut block_rx = p2p_node.subscribe_blocks();

            while let Ok(blk) = block_rx.recv().await {
                debug!("[UTXO-GUARD] Received live TRU block #{} ({})", blk.height, blk.hash);
                self.apply_block(&blk, Some(&orderbook)).await;
            }
            warn!("[UTXO-GUARD] Live TRU block stream closed");
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_pubkey_to_address_roundtrip() {
        let kp = crate::core::keys::generate_keypair();
        let spk = crate::core::script::tru_address_to_spk(&kp.address).unwrap();
        let derived_addr = script_pubkey_to_tru_address(&spk).unwrap();
        assert_eq!(derived_addr, kp.address);
    }
}
