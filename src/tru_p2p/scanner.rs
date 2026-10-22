use super::wire::BlockProto;
use crate::orderbook::Orderbook;
use crate::tru_p2p::client::TruP2PNode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwapStatus {
    Active,       // Unspent and has sufficient time remaining
    ExpiringSoon, // Unspent but less than min_margin hours remaining
    Expired,      // Unspent but timelock has passed
    Claimed,      // Successfully claimed on-chain
    Refunded,     // Refunded on-chain after timeout
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredTruSwap {
    pub txid: String,
    pub vout: u32,
    pub block_height: u32,
    #[serde(default)]
    pub confirmations: u32,
    #[serde(default)]
    pub is_safe: bool,
    pub amount_atoms: u64,
    pub amount_tru: f64,
    pub hash160_hex: String,
    pub timelock: u32,
    pub hours_remaining: f64,
    pub is_spent: bool,
    pub spent_by_txid: Option<String>,
    pub status: SwapStatus,
    pub raw_script: String,
    #[serde(default)]
    pub secret_hex: Option<String>,
}

pub const MAX_REORG_DEPTH: u64 = 100;

#[derive(Clone, Debug, Default)]
pub struct BlockUndo {
    pub height: u32,
    pub vault_utxos_added: Vec<String>,
    pub vault_utxos_spent: Vec<UtxoEntry>,
    pub address_utxos_added: Vec<String>,
    pub address_utxos_spent: Vec<(String, String, u64)>, // (id, addr, atoms)
    pub address_received_deltas: Vec<(String, u64)>,
    pub address_sent_deltas: Vec<(String, u64)>,
    pub swaps_added: Vec<String>,
    pub swaps_modified: Vec<(String, SwapStatus, bool, Option<String>, Option<String>)>,
    pub txids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UtxoEntry {
    pub txid: String,
    pub vout: u32,
    pub amount_atoms: u64,
    pub amount_tru: f64,
    pub block_height: u32,
    pub script_pubkey: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MempoolEntry {
    pub txid: String,
    pub first_seen_sec: u64,
    pub size_bytes: usize,
    pub inputs_count: usize,
    pub outputs_count: usize,
    pub total_out_atoms: u64,
    pub total_out_tru: f64,
    pub target_amount_atoms: u64,
    pub target_amount_tru: f64,
    pub change_amount_atoms: u64,
    pub change_amount_tru: f64,
    pub is_vault_related: bool,
    pub is_htlc: bool,
    pub htlc_hash160: Option<String>,
    pub htlc_timelock: Option<u32>,
    pub outputs: Vec<MempoolOutput>,
    #[serde(default)]
    pub spent_inputs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MempoolOutput {
    pub vout: u32,
    pub amount_atoms: u64,
    pub amount_tru: f64,
    pub script_pubkey: String,
    pub address: Option<String>,
    pub is_vault: bool,
    pub is_htlc: bool,
    #[serde(default)]
    pub is_quantum: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedTruTx {
    pub txid: String,
    pub version: i32,
    pub lock_time: u32,
    pub is_coinbase: bool,
    #[serde(default)]
    pub is_quantum: bool,
    pub block_height: Option<u32>,
    pub block_hash: Option<String>,
    pub confirmations: u32,
    pub timestamp: u64,
    pub inputs: Vec<CachedTruTxIn>,
    pub outputs: Vec<CachedTruTxOut>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedTruTxIn {
    pub prev_txid: String,
    pub prev_vout: u32,
    pub script_sig: String,
    pub is_coinbase: bool,
    #[serde(default)]
    pub is_quantum: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedTruTxOut {
    pub vout: u32,
    pub amount_atoms: u64,
    pub amount_tru: f64,
    pub script_pubkey: String,
    pub address: Option<String>,
    pub display_target: String,
    pub is_htlc: bool,
    pub is_vault: bool,
    #[serde(default)]
    pub is_quantum: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedTruBlock {
    pub height: u32,
    pub hash: String,
    pub prev_hash: String,
    pub merkle_root: String,
    pub timestamp: u64,
    pub formatted_time: String,
    pub version: i32,
    pub bits: u32,
    pub nonce: u64,
    pub tx_count: usize,
    pub size_bytes: usize,
    pub total_amount_tru: f64,
    pub txids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruBlockHeader {
    pub version: i32,
    pub prev_hash: String,
    pub merkle_root: String,
    pub timestamp: u32,
    pub bits: u32,
    pub nonce: u64,
}

pub fn parse_tru_block_header(raw: &str) -> Option<TruBlockHeader> {
    let parts: Vec<&str> = raw.split('|').collect();
    if parts.len() < 5 {
        return None;
    }
    let version = parts[0].parse::<i32>().ok()?;
    let prev_hash = parts[1].to_string();
    let merkle_root = parts[2].to_string();
    let timestamp = parts[3].parse::<u32>().ok()?;
    let bits = parts[4].parse::<u32>().ok()?;
    let nonce = if parts.len() >= 6 {
        parts[5].parse::<u64>().unwrap_or(0)
    } else {
        0
    };
    Some(TruBlockHeader {
        version,
        prev_hash,
        merkle_root,
        timestamp,
        bits,
        nonce,
    })
}

fn hex_to_le32(hex_str: &str) -> Option<[u8; 32]> {
    if hex_str.len() != 64 {
        return None;
    }
    let decoded = hex::decode(hex_str).ok()?;
    if decoded.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = decoded[31 - i];
    }
    Some(out)
}

pub fn build_block_header_80(hdr: &TruBlockHeader) -> Option<[u8; 80]> {
    let mut buf = [0u8; 80];
    buf[0..4].copy_from_slice(&hdr.version.to_le_bytes());
    let prev_bytes = hex_to_le32(&hdr.prev_hash)?;
    buf[4..36].copy_from_slice(&prev_bytes);
    let merkle_bytes = hex_to_le32(&hdr.merkle_root)?;
    buf[36..68].copy_from_slice(&merkle_bytes);
    buf[68..72].copy_from_slice(&hdr.timestamp.to_le_bytes());
    buf[72..76].copy_from_slice(&hdr.bits.to_le_bytes());
    buf[76..80].copy_from_slice(&(hdr.nonce as u32).to_le_bytes());
    Some(buf)
}

pub fn compute_tru_block_hash(hdr: &TruBlockHeader) -> Option<String> {
    let buf = build_block_header_80(hdr)?;
    let h1 = Sha256::digest(&buf);
    let mut h2: [u8; 32] = Sha256::digest(&h1).into();
    let last32 = ((h2[28] as u32) << 24)
        | ((h2[29] as u32) << 16)
        | ((h2[30] as u32) << 8)
        | (h2[31] as u32);
    let injected = last32.wrapping_add(0x21E8);
    h2[28] = ((injected >> 24) & 0xff) as u8;
    h2[29] = ((injected >> 16) & 0xff) as u8;
    h2[30] = ((injected >> 8) & 0xff) as u8;
    h2[31] = (injected & 0xff) as u8;

    Some(hex::encode(h2))
}

pub fn compute_tru_merkle_root(tx_raws: &[String]) -> String {
    if tx_raws.is_empty() {
        let h1 = Sha256::digest(b"");
        let h2 = Sha256::digest(&h1);
        return hex::encode(h2);
    }
    let mut layer: Vec<String> = tx_raws
        .iter()
        .map(|raw| {
            if let Some(pos) = raw.find('|') {
                raw[..pos].to_string()
            } else {
                raw.clone()
            }
        })
        .collect();

    while layer.len() > 1 {
        if layer.len() % 2 != 0 {
            let last = layer.last().unwrap().clone();
            layer.push(last);
        }
        let mut new_layer = Vec::with_capacity(layer.len() / 2);
        for chunk in layer.chunks(2) {
            let combined = format!("{}{}", chunk[0], chunk[1]);
            let h1 = Sha256::digest(combined.as_bytes());
            let h2 = Sha256::digest(&h1);
            new_layer.push(hex::encode(h2));
        }
        layer = new_layer;
    }

    layer[0].clone()
}

#[derive(Clone)]
pub struct BlockScanner {
    p2p_node: Arc<TruP2PNode>,
    orderbook: Arc<Orderbook>,
    all_swaps: Arc<RwLock<HashMap<String, DiscoveredTruSwap>>>, // key: "txid:vout"
    scanned_height: Arc<AtomicU64>,
    verified_height: Arc<AtomicU64>,
    target_height: Arc<AtomicU64>,
    is_scanning: Arc<AtomicBool>,
    min_margin_hours: f64,
    last_verified_hash: Arc<RwLock<String>>,
    recent_block_hashes: Arc<RwLock<BTreeMap<u64, String>>>,
    integrity_violations: Arc<AtomicU64>,
    sync_start_time: Arc<RwLock<Option<Instant>>>,
    sync_duration_secs: Arc<RwLock<f64>>,
    sync_blocks_per_sec: Arc<RwLock<f64>>,
    vault_address: Arc<RwLock<Option<String>>>,
    vault_spk: Arc<RwLock<Option<String>>>,
    vault_utxos: Arc<RwLock<HashMap<String, UtxoEntry>>>, // key: "txid:vout"
    vault_mempool_utxos: Arc<RwLock<HashMap<String, UtxoEntry>>>, // 0-conf in-flight change UTXOs
    vault_balance_atoms: Arc<AtomicU64>,
    mempool: Arc<RwLock<HashMap<String, MempoolEntry>>>,
    mempool_count: Arc<AtomicU64>,
    tx_cache: Arc<RwLock<HashMap<String, CachedTruTx>>>,
    block_cache: Arc<RwLock<BTreeMap<u32, CachedTruBlock>>>,
    address_utxos: Arc<RwLock<HashMap<String, (String, u64)>>>,
    address_balances: Arc<RwLock<HashMap<String, u64>>>,
    address_total_received: Arc<RwLock<HashMap<String, u64>>>,
    address_total_sent: Arc<RwLock<HashMap<String, u64>>>,
    address_tx_map: Arc<RwLock<HashMap<String, Vec<String>>>>,
    block_undo_log: Arc<RwLock<BTreeMap<u32, BlockUndo>>>,
    resync_from_genesis: Arc<AtomicBool>,
    data_dir: Arc<RwLock<Option<PathBuf>>>,
}

impl BlockScanner {
    pub fn new(p2p_node: Arc<TruP2PNode>, orderbook: Arc<Orderbook>) -> Self {
        let (vault_address, vault_spk): (Option<String>, Option<String>) = (None, None);

        Self {
            p2p_node,
            orderbook,
            all_swaps: Arc::new(RwLock::new(HashMap::new())),
            scanned_height: Arc::new(AtomicU64::new(0)),
            verified_height: Arc::new(AtomicU64::new(0)),
            target_height: Arc::new(AtomicU64::new(0)),
            is_scanning: Arc::new(AtomicBool::new(false)),
            min_margin_hours: 2.0, // Default 2 hours margin
            last_verified_hash: Arc::new(RwLock::new(String::new())),
            recent_block_hashes: Arc::new(RwLock::new(BTreeMap::new())),
            integrity_violations: Arc::new(AtomicU64::new(0)),
            sync_start_time: Arc::new(RwLock::new(None)),
            sync_duration_secs: Arc::new(RwLock::new(0.0)),
            sync_blocks_per_sec: Arc::new(RwLock::new(0.0)),
            vault_address: Arc::new(RwLock::new(vault_address)),
            vault_spk: Arc::new(RwLock::new(vault_spk)),
            vault_utxos: Arc::new(RwLock::new(HashMap::new())),
            vault_mempool_utxos: Arc::new(RwLock::new(HashMap::new())),
            vault_balance_atoms: Arc::new(AtomicU64::new(0)),
            mempool: Arc::new(RwLock::new(HashMap::new())),
            mempool_count: Arc::new(AtomicU64::new(0)),
            tx_cache: Arc::new(RwLock::new(HashMap::new())),
            block_cache: Arc::new(RwLock::new(BTreeMap::new())),
            address_utxos: Arc::new(RwLock::new(HashMap::new())),
            address_balances: Arc::new(RwLock::new(HashMap::new())),
            address_total_received: Arc::new(RwLock::new(HashMap::new())),
            address_total_sent: Arc::new(RwLock::new(HashMap::new())),
            address_tx_map: Arc::new(RwLock::new(HashMap::new())),
            block_undo_log: Arc::new(RwLock::new(BTreeMap::new())),
            resync_from_genesis: Arc::new(AtomicBool::new(false)),
            data_dir: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_data_dir(&self, dir: PathBuf) {
        if let Ok(mut lock) = self.data_dir.try_write() {
            *lock = Some(dir);
        }
    }

    pub fn set_resync_from_genesis(&self, val: bool) {
        self.resync_from_genesis.store(val, Ordering::SeqCst);
    }

    pub async fn get_mempool(&self) -> Vec<MempoolEntry> {
        let lock = self.mempool.read().await;
        let mut list: Vec<_> = lock.values().cloned().collect();
        list.sort_by(|a, b| b.first_seen_sec.cmp(&a.first_seen_sec).then_with(|| a.txid.cmp(&b.txid)));
        list
    }

    pub fn mempool_count(&self) -> u64 {
        self.mempool_count.load(Ordering::Relaxed)
    }

    pub async fn set_vault_address(&self, address: &str) {
        if let Some(spk) = crate::crypto::tru_address_to_spk(address) {
            *self.vault_address.write().await = Some(address.to_string());
            *self.vault_spk.write().await = Some(spk);
        }
    }

    pub fn vault_balance_atoms(&self) -> u64 {
        self.vault_balance_atoms.load(Ordering::Relaxed)
    }

    pub fn vault_confirmed_balance_atoms(&self) -> u64 {
        self.vault_balance_atoms.load(Ordering::Relaxed)
    }

    pub async fn vault_unconfirmed_balance_atoms(&self) -> u64 {
        let lock_conf = self.vault_utxos.read().await;
        let lock_mempool = self.vault_mempool_utxos.read().await;
        lock_mempool
            .iter()
            .filter(|(op, _)| !lock_conf.contains_key(*op))
            .map(|(_, u)| u.amount_atoms)
            .sum()
    }

    pub fn vault_balance_tru(&self) -> f64 {
        self.vault_balance_atoms.load(Ordering::Relaxed) as f64 / 100_000_000.0
    }

    pub async fn vault_address(&self) -> Option<String> {
        self.vault_address.read().await.clone()
    }

    /// Returns only on-chain confirmed spendable UTXOs (required by TRU core node policy)
    pub async fn get_vault_spendable_utxos(&self) -> Vec<UtxoEntry> {
        let lock_conf = self.vault_utxos.read().await;
        let mut list: Vec<UtxoEntry> = lock_conf.values().cloned().collect();
        list.sort_by(|a, b| b.block_height.cmp(&a.block_height));
        list
    }

    pub async fn get_vault_utxos(&self) -> Vec<UtxoEntry> {
        let lock_conf = self.vault_utxos.read().await;
        let lock_mempool = self.vault_mempool_utxos.read().await;
        let mut list: Vec<UtxoEntry> = lock_conf.values().cloned().collect();
        for (op, entry) in lock_mempool.iter() {
            if !lock_conf.contains_key(op) {
                list.push(entry.clone());
            }
        }
        // Sort: confirmed first (higher block_height), then unconfirmed change
        list.sort_by(|a, b| b.block_height.cmp(&a.block_height));
        list
    }

    pub async fn add_vault_mempool_utxo(&self, entry: UtxoEntry) {
        let op = format!("{}:{}", entry.txid, entry.vout);
        let mut lock = self.vault_mempool_utxos.write().await;
        lock.insert(op, entry);
    }

    pub async fn mark_spent_outpoints(&self, outpoints: &[String]) {
        {
            let mut utxos = self.vault_utxos.write().await;
            for op in outpoints {
                utxos.remove(op);
            }
            let total_atoms: u64 = utxos.values().map(|u| u.amount_atoms).sum();
            self.vault_balance_atoms.store(total_atoms, Ordering::Relaxed);
        }
        {
            let mut mempool_utxos = self.vault_mempool_utxos.write().await;
            for op in outpoints {
                mempool_utxos.remove(op);
            }
        }
    }


    pub fn scanned_height(&self) -> u64 {
        self.scanned_height.load(Ordering::Relaxed)
    }

    pub fn verified_height(&self) -> u64 {
        self.verified_height.load(Ordering::Relaxed)
    }

    pub fn target_height(&self) -> u64 {
        self.target_height.load(Ordering::Relaxed)
    }

    pub fn update_target_height(&self, height: u64) {
        self.target_height.fetch_max(height, Ordering::Relaxed);
    }

    pub fn is_scanning(&self) -> bool {
        self.is_scanning.load(Ordering::Relaxed)
    }

    pub fn integrity_violations(&self) -> u64 {
        self.integrity_violations.load(Ordering::Relaxed)
    }

    pub async fn last_verified_hash(&self) -> String {
        self.last_verified_hash.read().await.clone()
    }

    pub async fn sync_duration_secs(&self) -> f64 {
        *self.sync_duration_secs.read().await
    }

    pub async fn sync_blocks_per_sec(&self) -> f64 {
        *self.sync_blocks_per_sec.read().await
    }

    pub async fn elapsed_secs(&self) -> f64 {
        if let Some(start) = *self.sync_start_time.read().await {
            start.elapsed().as_secs_f64()
        } else {
            0.0
        }
    }

    pub async fn get_all_swaps(&self) -> Vec<DiscoveredTruSwap> {
        let tip = self.verified_height.load(Ordering::Relaxed);
        let lock = self.all_swaps.read().await;
        let mut list: Vec<_> = lock.values().cloned().map(|mut s| {
            s.confirmations = if s.block_height > 0 && tip >= s.block_height as u64 {
                (tip - s.block_height as u64 + 1) as u32
            } else {
                0
            };
            s.is_safe = s.confirmations >= 6;
            s
        }).collect();
        list.sort_by(|a, b| b.block_height.cmp(&a.block_height));
        list
    }

    pub async fn get_active_swaps(&self) -> Vec<DiscoveredTruSwap> {
        let tip = self.verified_height.load(Ordering::Relaxed);
        let lock = self.all_swaps.read().await;
        let mut list: Vec<_> = lock
            .values()
            .filter(|s| (s.status == SwapStatus::Active || s.status == SwapStatus::ExpiringSoon) && !s.is_spent)
            .cloned()
            .map(|mut s| {
                s.confirmations = if s.block_height > 0 && tip >= s.block_height as u64 {
                    (tip - s.block_height as u64 + 1) as u32
                } else {
                    0
                };
                s.is_safe = s.confirmations >= 6;
                s
            })
            .collect();
        list.sort_by(|a, b| b.block_height.cmp(&a.block_height));
        list
    }

    /// Returns (is_safe, confirmations) for a given hash160.
    /// Safe means >= 6 confirmed blocks on canonical TRU chain!
    pub async fn get_swap_safety(&self, hash160_hex: &str) -> (bool, u32) {
        let clean = hash160_hex.strip_prefix("0x").unwrap_or(hash160_hex).to_lowercase();
        let tip = self.verified_height.load(Ordering::Relaxed);
        let lock = self.all_swaps.read().await;
        for s in lock.values() {
            if s.hash160_hex.to_lowercase() == clean {
                let confs = if s.block_height > 0 && tip >= s.block_height as u64 {
                    (tip - s.block_height as u64 + 1) as u32
                } else {
                    0
                };
                return (confs >= 6, confs);
            }
        }
        (false, 0)
    }

    pub async fn get_swap_txid(&self, hash160_hex: &str) -> Option<String> {
        let clean = hash160_hex.strip_prefix("0x").unwrap_or(hash160_hex).to_lowercase();
        {
            let lock = self.all_swaps.read().await;
            for s in lock.values() {
                if s.hash160_hex.to_lowercase() == clean {
                    return Some(s.txid.clone());
                }
            }
        }
        {
            let mem = self.mempool.read().await;
            for entry in mem.values() {
                if let Some(ref h) = entry.htlc_hash160 {
                    if h.to_lowercase() == clean {
                        return Some(entry.txid.clone());
                    }
                }
            }
        }
        None
    }

    pub async fn get_swap_secret(&self, hash160_hex: &str) -> Option<String> {
        let clean = hash160_hex.strip_prefix("0x").unwrap_or(hash160_hex).to_lowercase();
        {
            let lock = self.all_swaps.read().await;
            for s in lock.values() {
                if s.hash160_hex.to_lowercase() == clean {
                    if let Some(ref sec) = s.secret_hex {
                        return Some(sec.clone());
                    }
                }
            }
        }
        // Fallback: check live mempool transaction inputs in tx_cache for revealed preimage secret
        {
            let cache = self.tx_cache.read().await;
            for tx in cache.values() {
                if tx.confirmations == 0 {
                    for inp in &tx.inputs {
                        if let Ok(sig_bytes) = hex::decode(&inp.script_sig) {
                            if sig_bytes.len() >= 34 && sig_bytes.last() == Some(&0x51) && sig_bytes[sig_bytes.len() - 34] == 0x20 {
                                let cand_secret = &sig_bytes[sig_bytes.len() - 33..sig_bytes.len() - 1];
                                let computed_h160 = crate::core::keys::hash160(cand_secret);
                                if hex::encode(computed_h160).to_lowercase() == clean {
                                    let sec_hex = hex::encode(cand_secret);
                                    return Some(sec_hex);
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Spawns the full background block scanner pipeline
    /// and strict sequential cryptographic chain integrity verification.
    pub fn start(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        // Spawn real-time P2P 0-conf mempool transaction listener
        let mempool_scanner = self.clone();
        let mut tx_rx = self.p2p_node.subscribe_txs();
        tokio::spawn(async move {
            info!("[TRU-MEMPOOL] Real-time 0-conf P2P mempool watcher active");
            while let Ok(tx_proto) = tx_rx.recv().await {
                mempool_scanner.add_mempool_tx(&tx_proto.txid, &tx_proto.raw).await;
            }
        });

        tokio::spawn(async move {
            info!("[TRU-SCANNER] Initializing block scanner and verifier...");

            // Wait until connected to at least 1 peer and know current chain height
            loop {
                let peers = self.p2p_node.peer_count();
                let height = self.p2p_node.current_height();
                if peers > 0 && height > 0 {
                    self.target_height.store(height, Ordering::SeqCst);
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            }

            let cache_path_buf = {
                let lock = self.data_dir.read().await;
                match &*lock {
                    Some(d) => d.join("tru.dat"),
                    None => PathBuf::from(super::chain_cache::DEFAULT_CACHE_PATH),
                }
            };
            let cache_path = cache_path_buf.as_path();
            let mut all_verified_blocks: Vec<BlockProto> = Vec::new();

            // Load and fast-verify cached chain from tru.dat unless full resync is requested
            let skip_cache = self.resync_from_genesis.load(Ordering::Relaxed);
            if skip_cache {
                info!(
                    "[TRU-SCANNER] Full P2P Resync enabled (--resync). Skipping local cache tru.dat and syncing all blocks directly from Genesis (#1) via P2P network..."
                );
            } else if let Ok(Some((cached_blocks, cached_tip, cached_hash))) =
                super::chain_cache::load_cached_chain(cache_path, super::chain_cache::DEFAULT_VERIFY_TAIL_BLOCKS)
            {
                let count = cached_blocks.len();
                info!(
                    "[TRU-PERSISTENCE] Restoring {} cached blocks from {} up to #{}...",
                    count,
                    cache_path.display(),
                    cached_tip
                );

                for blk in &cached_blocks {
                    self.process_block(blk).await;
                }

                // Populate recent window and tip
                {
                    let mut r = self.recent_block_hashes.write().await;
                    for blk in cached_blocks.iter().rev().take(512) {
                        r.insert(blk.height as u64, blk.hash.clone());
                    }
                    let mut tip_lock = self.last_verified_hash.write().await;
                    *tip_lock = cached_hash;
                }

                self.verified_height.store(cached_tip, Ordering::SeqCst);
                self.scanned_height.store(cached_tip, Ordering::SeqCst);
                all_verified_blocks = cached_blocks;
            }

            // Wait briefly for P2P connection to establish initial target tip height
            for _ in 0..25 {
                let cur = self.p2p_node.current_height();
                if cur > 0 {
                    self.target_height.store(cur, Ordering::SeqCst);
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
            }

            let initial_verified = self.verified_height.load(Ordering::Relaxed);
            let target = self.target_height.load(Ordering::Relaxed);

            if initial_verified < target {
                info!(
                    "[TRU-SCANNER] Connected to P2P network. Syncing ledger with chain validation #{} -> #{}...",
                    initial_verified + 1,
                    target
                );
            } else {
                info!(
                    "[TRU-SCANNER] Chain already up-to-date with P2P network tip #{}! Instant startup.",
                    target
                );
            }

            self.is_scanning.store(initial_verified < target, Ordering::SeqCst);
            *self.sync_start_time.write().await = Some(Instant::now());

            let mut block_rx = self.p2p_node.subscribe_blocks();
            let mut requested_height: u64 = initial_verified + 1;
            let mut next_verify_height: u64 = initial_verified + 1;
            let mut pending_requests: HashMap<u64, (Instant, usize)> = HashMap::new(); // height -> (sent_time, retry_offset)
            let mut reorder_buffer: BTreeMap<u64, BlockProto> = BTreeMap::new();

            const MAX_IN_FLIGHT_PER_PEER: usize = 64;
            const MAX_AHEAD: u64 = 1024;

            while next_verify_height <= self.target_height.load(Ordering::Relaxed) {
                // Update target if chain progressed
                let current_tip = self.p2p_node.current_height();
                if current_tip > self.target_height.load(Ordering::Relaxed) {
                    self.target_height.store(current_tip, Ordering::SeqCst);
                }
                let tip = self.target_height.load(Ordering::Relaxed);
                let active_peers = self.p2p_node.peer_count().max(1) as usize;
                let max_in_flight = (active_peers * MAX_IN_FLIGHT_PER_PEER).min(256).max(64);

                // Top up in-flight requests across active peers in batch
                let mut batch = Vec::new();
                while pending_requests.len() + batch.len() < max_in_flight
                    && requested_height <= tip
                    && requested_height < next_verify_height + MAX_AHEAD
                {
                    batch.push(requested_height);
                    requested_height += 1;
                }

                if !batch.is_empty() {
                    let now = Instant::now();
                    for &h in &batch {
                        pending_requests.insert(h, (now, 0));
                    }
                    self.p2p_node.send_get_blocks_batch(&batch).await;
                }

                // Wait for block response or retry interval
                tokio::select! {
                    recv_res = block_rx.recv() => {
                        match recv_res {
                            Ok(blk) => {
                                let h = blk.height as u64;
                                pending_requests.remove(&h);
                                reorder_buffer.insert(h, blk);

                                // Drain reorder buffer strictly sequentially to verify chain continuity
                                while let Some(current_blk) = reorder_buffer.remove(&next_verify_height) {
                                    let mut valid = true;

                                    // Parse header
                                    let hdr = match parse_tru_block_header(&current_blk.header) {
                                        Some(h) => h,
                                        None => {
                                            warn!(
                                                "[TRU-CHAIN-INTEGRITY] Block #{} header parsing failed! Data: {}",
                                                next_verify_height, current_blk.header
                                            );
                                            valid = false;
                                            TruBlockHeader {
                                                version: 1,
                                                prev_hash: String::new(),
                                                merkle_root: String::new(),
                                                timestamp: 0,
                                                bits: 0,
                                                nonce: 0,
                                            }
                                        }
                                    };

                                    // 1. Cryptographic check: computed double-SHA256 (+21E8) matches block hash
                                    if valid {
                                        if let Some(computed_hash) = compute_tru_block_hash(&hdr) {
                                            if computed_hash != current_blk.hash {
                                                warn!(
                                                    "[TRU-CHAIN-INTEGRITY] Block #{} hash mismatch! Computed: {}, Wire: {}",
                                                    next_verify_height, computed_hash, current_blk.hash
                                                );
                                                valid = false;
                                            }
                                        }
                                    }

                                    // 2. Cryptographic check: chain continuity (prevHash == last_verified_hash)
                                    let mut prev_hash_broken = false;
                                    if valid {
                                        let last_hash = self.last_verified_hash.read().await.clone();
                                        if next_verify_height > 1 {
                                            if !last_hash.is_empty() && hdr.prev_hash != last_hash {
                                                warn!(
                                                    "[TRU-CHAIN-INTEGRITY] Block #{} prevHash link broken! Block prevHash: {}, Chain tip: {}",
                                                    next_verify_height, hdr.prev_hash, last_hash
                                                );
                                                valid = false;
                                                prev_hash_broken = true;
                                            }
                                        } else {
                                            debug!("[TRU-CHAIN-INTEGRITY] Genesis block #1 accepted: {}", current_blk.hash);
                                        }
                                    }

                                    // 3. Cryptographic check: Merkle root of all transactions matches header
                                    if valid {
                                        let computed_merkle = compute_tru_merkle_root(&current_blk.transactions);
                                        if computed_merkle != hdr.merkle_root {
                                            warn!(
                                                "[TRU-CHAIN-INTEGRITY] Block #{} Merkle root mismatch! Computed: {}, Header: {}",
                                                next_verify_height, computed_merkle, hdr.merkle_root
                                            );
                                            valid = false;
                                        }
                                    }

                                    if valid {
                                        // Update tip hash and recent block window
                                        {
                                            let mut hash_lock = self.last_verified_hash.write().await;
                                            *hash_lock = current_blk.hash.clone();
                                            let mut r = self.recent_block_hashes.write().await;
                                            r.insert(next_verify_height, current_blk.hash.clone());
                                            if r.len() > 512 {
                                                if let Some(oldest) = r.keys().next().copied() {
                                                    r.remove(&oldest);
                                                }
                                            }
                                        }

                                        // Process HTLC swaps
                                        self.process_block(&current_blk).await;
                                        all_verified_blocks.push(current_blk.clone());

                                        self.scanned_height.store(next_verify_height, Ordering::Relaxed);
                                        self.verified_height.store(next_verify_height, Ordering::Relaxed);

                                        // Update live speed metrics
                                        if let Some(start) = *self.sync_start_time.read().await {
                                            let elapsed = start.elapsed().as_secs_f64();
                                            if elapsed > 0.05 {
                                                let bps = next_verify_height as f64 / elapsed;
                                                *self.sync_blocks_per_sec.write().await = bps;
                                            }
                                        }

                                        if next_verify_height % 2000 == 0 || next_verify_height == tip {
                                            let _ = super::chain_cache::save_cached_chain(cache_path, &all_verified_blocks);
                                            let swaps = self.all_swaps.read().await;
                                            let active = swaps.values().filter(|s| (s.status == SwapStatus::Active || s.status == SwapStatus::ExpiringSoon) && !s.is_spent).count();
                                            let bps = *self.sync_blocks_per_sec.read().await;
                                            info!(
                                                "[TRU-CHAIN-INTEGRITY] Verified: #{}/#{} ({:.1}%) | Speed: {:.0} blk/s | Swaps: {} (Active: {}) | Tip: {}...",
                                                next_verify_height,
                                                tip,
                                                (next_verify_height as f64 / tip as f64) * 100.0,
                                                bps,
                                                swaps.len(),
                                                active,
                                                &current_blk.hash[..16]
                                            );
                                        }

                                        next_verify_height += 1;
                                    } else {
                                        // Validation failed on block from peer!
                                        self.integrity_violations.fetch_add(1, Ordering::Relaxed);
                                        if prev_hash_broken && next_verify_height > 1 && !all_verified_blocks.is_empty() {
                                            let rollback_to = next_verify_height - 2;
                                            warn!(
                                                "[TRU-CHAIN-INTEGRITY] Chain fork encountered in batch sync at #{}. Rolling back tip to #{} to search for common ancestor...",
                                                next_verify_height - 1, rollback_to
                                            );
                                            self.rollback_to_height(rollback_to, &mut all_verified_blocks, cache_path).await;
                                            next_verify_height = rollback_to + 1;
                                            requested_height = next_verify_height;
                                            pending_requests.clear();
                                            reorder_buffer.clear();
                                            let _ = self.p2p_node.send_get_block(next_verify_height).await;
                                            break;
                                        } else {
                                            warn!(
                                                "[TRU-CHAIN-INTEGRITY] Rejecting invalid block #{}. Re-requesting with peer offset...",
                                                next_verify_height
                                            );
                                            pending_requests.insert(next_verify_height, (Instant::now(), 1));
                                            let _ = self.p2p_node.send_get_block_with_offset(next_verify_height, 1).await;
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                debug!("[TRU-SCANNER] Broadcast channel caught up (skipped {} transient frames)", skipped);
                            }
                            Err(_) => {}
                        }
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(250)) => {
                        // High-frequency stall recovery: re-request missing blocks using round-robin peer offset
                        let now = Instant::now();
                        let mut to_retry = Vec::new();

                        // Immediate high priority retry if next_verify_height is stalled > 350ms
                        if !reorder_buffer.contains_key(&next_verify_height) && next_verify_height <= tip {
                            let should_retry = if let Some(&(sent_at, _retry_count)) = pending_requests.get(&next_verify_height) {
                                now.duration_since(sent_at) > Duration::from_millis(350)
                            } else {
                                true
                            };
                            if should_retry {
                                let offset = pending_requests.get(&next_verify_height).map(|x| x.1 + 1).unwrap_or(1);
                                to_retry.push((next_verify_height, offset));
                            }
                        }

                        for (&h, &(sent_at, retry_count)) in pending_requests.iter() {
                            if h != next_verify_height && now.duration_since(sent_at) > Duration::from_millis(1200) {
                                to_retry.push((h, retry_count + 1));
                            }
                        }

                        for (h, next_offset) in to_retry.into_iter().take(32) {
                            pending_requests.insert(h, (now, next_offset));
                            let _ = self.p2p_node.send_get_block_with_offset(h, next_offset).await;
                        }
                    }
                }
            }

            self.is_scanning.store(false, Ordering::SeqCst);
            let _ = super::chain_cache::save_cached_chain(cache_path, &all_verified_blocks);
            let total_swaps = self.all_swaps.read().await.len();
            let active_swaps = self.get_active_swaps().await.len();
            let tip_hash = self.last_verified_hash.read().await.clone();

            if let Some(start) = *self.sync_start_time.read().await {
                let total_dur = start.elapsed().as_secs_f64();
                *self.sync_duration_secs.write().await = total_dur;
                let final_bps = target as f64 / total_dur.max(0.001);
                *self.sync_blocks_per_sec.write().await = final_bps;
                info!(
                    "[TRU-SCANNER] Full historical scan & verification COMPLETE (#1 to #{}). Finished in {:.2}s ({:.0} blk/s). Found {} total swaps ({} active). Tip Hash: {}",
                    self.scanned_height.load(Ordering::Relaxed),
                    total_dur,
                    final_bps,
                    total_swaps,
                    active_swaps,
                    tip_hash
                );
            }

            // Drain any leftover responses from peers buffered during initial sync
            while block_rx.try_recv().is_ok() {}

            // Continue listening for newly mined blocks in real-time with full cryptographic verification & reorg defense
            let mut tip_poll_ticker = tokio::time::interval(tokio::time::Duration::from_millis(250));
            tip_poll_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut watchdog_ticker = tokio::time::interval(tokio::time::Duration::from_millis(1000));
            watchdog_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_advance_time = Instant::now();
            let mut last_seen_verified = self.verified_height.load(Ordering::Relaxed);
            let mut realtime_reorder: BTreeMap<u64, BlockProto> = BTreeMap::new();

            loop {
                tokio::select! {
                    _ = self.p2p_node.tip_notify.notified() => {
                        let net_tip = self.p2p_node.current_height();
                        let cur_verified = self.verified_height.load(Ordering::Relaxed);
                        if net_tip > cur_verified {
                            self.target_height.fetch_max(net_tip, Ordering::Relaxed);
                            let mut missing = Vec::new();
                            for missing_h in (cur_verified + 1)..=net_tip {
                                if !realtime_reorder.contains_key(&missing_h) {
                                    missing.push(missing_h);
                                    if missing.len() >= 64 {
                                        break;
                                    }
                                }
                            }
                            if !missing.is_empty() {
                                self.p2p_node.send_get_blocks_batch(&missing).await;
                            }
                        }
                    }
                    _ = tip_poll_ticker.tick() => {
                        let net_tip = self.p2p_node.current_height();
                        let cur_verified = self.verified_height.load(Ordering::Relaxed);
                        if net_tip > cur_verified {
                            self.target_height.fetch_max(net_tip, Ordering::Relaxed);
                            debug!(
                                "[TRU-SCANNER] Real-time network tip ahead: #{} > #{} (gap: {}). Fetching missing blocks...",
                                net_tip, cur_verified, net_tip - cur_verified
                            );
                            let mut missing = Vec::new();
                            for missing_h in (cur_verified + 1)..=net_tip {
                                if !realtime_reorder.contains_key(&missing_h) {
                                    missing.push(missing_h);
                                    if missing.len() >= 64 {
                                        break;
                                    }
                                }
                            }
                            if !missing.is_empty() {
                                self.p2p_node.send_get_blocks_batch(&missing).await;
                            }
                        }
                    }
                    _ = watchdog_ticker.tick() => {
                        let net_tip = self.p2p_node.current_height();
                        let mut cur_verified = self.verified_height.load(Ordering::Relaxed);
                        if cur_verified > last_seen_verified {
                            last_seen_verified = cur_verified;
                            last_advance_time = Instant::now();
                        }

                        // 1. Proactive Reorg Detection: verify buffered next block against local tip
                        if let Some(next_blk) = realtime_reorder.get(&(cur_verified + 1)) {
                            if let Some(next_hdr) = parse_tru_block_header(&next_blk.header) {
                                let cur_tip = self.last_verified_hash.read().await.clone();
                                if !cur_tip.is_empty() && next_hdr.prev_hash != cur_tip {
                                    warn!(
                                        "[AUTONOMOUS-SYNC] Disconnected tip detected! Tip #{} ({}) != block #{} prev_hash {}. Resolving reorg...",
                                        cur_verified,
                                        &cur_tip[..16.min(cur_tip.len())],
                                        cur_verified + 1,
                                        &next_hdr.prev_hash[..16.min(next_hdr.prev_hash.len())]
                                    );

                                    let ancestor_match = {
                                        let r = self.recent_block_hashes.read().await;
                                        r.iter().find_map(|(&height, hash)| {
                                            if *hash == next_hdr.prev_hash {
                                                Some(height)
                                            } else {
                                                None
                                            }
                                        })
                                    };

                                    if let Some(ancestor_height) = ancestor_match {
                                        let depth = cur_verified.saturating_sub(ancestor_height);
                                        if depth <= MAX_REORG_DEPTH {
                                            info!(
                                                "[AUTONOMOUS-SYNC] Rolling back {} block(s) to common ancestor #{}...",
                                                depth, ancestor_height
                                            );
                                            self.rollback_to_height(ancestor_height, &mut all_verified_blocks, cache_path).await;
                                            cur_verified = ancestor_height;
                                            last_seen_verified = cur_verified;
                                            last_advance_time = Instant::now();
                                            let missing: Vec<u64> = ((ancestor_height + 1)..=(cur_verified + 2)).collect();
                                            self.p2p_node.broadcast_get_blocks(&missing).await;
                                        }
                                    } else if cur_verified > 1 {
                                        let rollback_to = cur_verified - 1;
                                        warn!(
                                            "[AUTONOMOUS-SYNC] Tip block #{} is orphaned. Rolling back to #{} to switch to canonical branch...",
                                            cur_verified, rollback_to
                                        );
                                        self.rollback_to_height(rollback_to, &mut all_verified_blocks, cache_path).await;
                                        cur_verified = rollback_to;
                                        last_seen_verified = cur_verified;
                                        last_advance_time = Instant::now();
                                        self.p2p_node.broadcast_get_blocks(&[rollback_to + 1, rollback_to + 2]).await;
                                    }
                                }
                            }
                        }

                        // 2. Stall Recovery: if network tip is ahead and local chain hasn't advanced in >2s
                        if net_tip > cur_verified && last_advance_time.elapsed() > Duration::from_millis(2000) {
                            debug!(
                                "[AUTONOMOUS-SYNC] Chain stalled at #{} behind network #{} for {:.1}s. Auto-healing...",
                                cur_verified, net_tip, last_advance_time.elapsed().as_secs_f32()
                            );

                            if last_advance_time.elapsed() > Duration::from_millis(5000) {
                                realtime_reorder.clear();
                                if last_advance_time.elapsed() > Duration::from_millis(6000) && cur_verified > 1 {
                                    let rollback_to = cur_verified - 1;
                                    warn!(
                                        "[AUTONOMOUS-SYNC] Deep stall (>6s). Rolling back tip #{} -> #{} to break deadlock...",
                                        cur_verified, rollback_to
                                    );
                                    self.rollback_to_height(rollback_to, &mut all_verified_blocks, cache_path).await;
                                    cur_verified = rollback_to;
                                    last_seen_verified = cur_verified;
                                    last_advance_time = Instant::now();
                                }
                            }

                            let end_h = net_tip.min(cur_verified + 16);
                            let missing: Vec<u64> = ((cur_verified + 1)..=end_h).collect();
                            if !missing.is_empty() {
                                self.p2p_node.broadcast_get_blocks(&missing).await;
                            }
                        }

                        // 3. Proactive P2P Height Refresh
                        if last_advance_time.elapsed() > Duration::from_secs(3) {
                            self.p2p_node.query_height_from_peers().await;
                        }
                    }
                    recv_res = block_rx.recv() => {
                        let blk = match recv_res {
                            Ok(b) => b,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                debug!("[TRU-SCANNER] Realtime broadcast lagged by {} blocks, continuing...", skipped);
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                warn!("[TRU-SCANNER] Realtime block broadcast channel closed!");
                                break;
                            }
                        };
                        let h = blk.height as u64;
                        let mut current_verified = self.verified_height.load(Ordering::Relaxed);

                        // 0. Ignore duplicate blocks that are already verified in our chain
                        {
                            let r = self.recent_block_hashes.read().await;
                            if let Some(existing_hash) = r.get(&h) {
                                if *existing_hash == blk.hash {
                                    continue;
                                }
                            }
                        }

                        // Ignore stale blocks that belong to historical sync or far behind current tip
                        if h < current_verified.saturating_sub(MAX_REORG_DEPTH + 32) {
                            continue;
                        }

                let tip_hash = self.last_verified_hash.read().await.clone();

                // 1. Cryptographic validation of block wire payload
                let Some(hdr) = parse_tru_block_header(&blk.header) else {
                    self.integrity_violations.fetch_add(1, Ordering::Relaxed);
                    warn!("[TRU-CHAIN-INTEGRITY] Malformed header received in block #{}! Rejecting.", h);
                    continue;
                };

                let mut crypto_valid = true;
                if let Some(computed) = compute_tru_block_hash(&hdr) {
                    if computed != blk.hash {
                        crypto_valid = false;
                        warn!(
                            "[TRU-CHAIN-INTEGRITY] Real-time block #{} hash mismatch! Computed: {}, Wire: {}",
                            h, computed, blk.hash
                        );
                    }
                }
                if compute_tru_merkle_root(&blk.transactions) != hdr.merkle_root {
                    crypto_valid = false;
                    warn!(
                        "[TRU-CHAIN-INTEGRITY] Real-time block #{} Merkle root mismatch! Header: {}",
                        h, hdr.merkle_root
                    );
                }

                if !crypto_valid {
                    self.integrity_violations.fetch_add(1, Ordering::Relaxed);
                    continue;
                }

                // 2. Chain continuity check & Reorg Defense
                if h == current_verified + 1 && (hdr.prev_hash == tip_hash || tip_hash.is_empty()) {
                    // Normal contiguous block extension
                    {
                        let mut hash_lock = self.last_verified_hash.write().await;
                        *hash_lock = blk.hash.clone();
                        let mut r = self.recent_block_hashes.write().await;
                        r.insert(h, blk.hash.clone());
                        if r.len() > 512 {
                            if let Some(oldest) = r.keys().next().copied() {
                                r.remove(&oldest);
                            }
                        }
                    }
                    self.process_block(&blk).await;
                    all_verified_blocks.push(blk.clone());
                    if all_verified_blocks.len() % 5 == 0 {
                        let _ = super::chain_cache::save_cached_chain(cache_path, &all_verified_blocks);
                    }
                    self.scanned_height.store(h, Ordering::Relaxed);
                    self.verified_height.store(h, Ordering::Relaxed);
                    self.target_height.fetch_max(h, Ordering::Relaxed);
                    info!(
                        "[TRU-CHAIN-INTEGRITY] Contiguous block #{} accepted & verified. New Tip: {}",
                        h,
                        &blk.hash[..16.min(blk.hash.len())]
                    );

                    current_verified = h;
                    last_seen_verified = current_verified;
                    last_advance_time = Instant::now();
                    while let Some(next_blk) = realtime_reorder.remove(&(current_verified + 1)) {
                        let next_h = next_blk.height as u64;
                        if let Some(next_hdr) = parse_tru_block_header(&next_blk.header) {
                            let cur_tip = self.last_verified_hash.read().await.clone();
                            if next_hdr.prev_hash == cur_tip || cur_tip.is_empty() {
                                {
                                    let mut hash_lock = self.last_verified_hash.write().await;
                                    *hash_lock = next_blk.hash.clone();
                                    let mut r = self.recent_block_hashes.write().await;
                                    r.insert(next_h, next_blk.hash.clone());
                                }
                                self.process_block(&next_blk).await;
                                all_verified_blocks.push(next_blk.clone());
                                self.scanned_height.store(next_h, Ordering::Relaxed);
                                self.verified_height.store(next_h, Ordering::Relaxed);
                                self.target_height.fetch_max(next_h, Ordering::Relaxed);
                                current_verified = next_h;
                                last_seen_verified = current_verified;
                                last_advance_time = Instant::now();
                                info!(
                                    "[TRU-CHAIN-INTEGRITY] Buffered contiguous block #{} accepted & verified.",
                                    next_h
                                );
                            } else {
                                realtime_reorder.insert(next_h, next_blk);
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                } else if hdr.prev_hash != tip_hash {
                    // Fork / Reorganization detected!
                    // Check if parent exists in recent block history
                    let ancestor_match = {
                        let r = self.recent_block_hashes.read().await;
                        r.iter().find_map(|(&height, hash)| {
                            if *hash == hdr.prev_hash {
                                Some(height)
                            } else {
                                None
                            }
                        })
                    };

                    if let Some(ancestor_height) = ancestor_match {
                        let depth = current_verified.saturating_sub(ancestor_height);
                        if depth > MAX_REORG_DEPTH {
                            warn!(
                                "[TRU-REORG-DEFENSE] Discovered fork branching from #{} (depth: {} > MAX_REORG_DEPTH {}). Rejecting deep fork.",
                                ancestor_height, depth, MAX_REORG_DEPTH
                            );
                            continue;
                        }

                        if h >= current_verified {
                            warn!(
                                "[TRU-REORG-DEFENSE] Chain reorg detected! Common ancestor at #{} (hash {}). Reorg depth: {} block(s) <= {}. Rolling back tip to #{}...",
                                ancestor_height,
                                &hdr.prev_hash[..16.min(hdr.prev_hash.len())],
                                depth,
                                MAX_REORG_DEPTH,
                                ancestor_height
                            );

                            self.rollback_to_height(ancestor_height, &mut all_verified_blocks, cache_path).await;
                            current_verified = ancestor_height;

                            // Switch to winning branch block if directly subsequent
                            if h == ancestor_height + 1 {
                                {
                                    let mut hash_lock = self.last_verified_hash.write().await;
                                    *hash_lock = blk.hash.clone();
                                    let mut r = self.recent_block_hashes.write().await;
                                    r.insert(h, blk.hash.clone());
                                }
                                self.process_block(&blk).await;
                                all_verified_blocks.push(blk.clone());
                                let _ = super::chain_cache::save_cached_chain(cache_path, &all_verified_blocks);
                                self.scanned_height.store(h, Ordering::Relaxed);
                                self.verified_height.store(h, Ordering::Relaxed);
                                self.target_height.fetch_max(h, Ordering::Relaxed);
                                current_verified = h;
                                last_seen_verified = current_verified;
                                last_advance_time = Instant::now();
                                info!(
                                    "[TRU-REORG-DEFENSE] Successfully switched to winning branch at #{} (Tip: {})",
                                    h,
                                    &blk.hash[..16.min(blk.hash.len())]
                                );

                                // Drain buffered contiguous blocks from realtime_reorder
                                while let Some(next_blk) = realtime_reorder.remove(&(current_verified + 1)) {
                                    let next_h = next_blk.height as u64;
                                    if let Some(next_hdr) = parse_tru_block_header(&next_blk.header) {
                                        let cur_tip = self.last_verified_hash.read().await.clone();
                                        if next_hdr.prev_hash == cur_tip || cur_tip.is_empty() {
                                            {
                                                let mut hash_lock = self.last_verified_hash.write().await;
                                                *hash_lock = next_blk.hash.clone();
                                                let mut r = self.recent_block_hashes.write().await;
                                                r.insert(next_h, next_blk.hash.clone());
                                            }
                                            self.process_block(&next_blk).await;
                                            all_verified_blocks.push(next_blk.clone());
                                            self.scanned_height.store(next_h, Ordering::Relaxed);
                                            self.verified_height.store(next_h, Ordering::Relaxed);
                                            self.target_height.fetch_max(next_h, Ordering::Relaxed);
                                            current_verified = next_h;
                                            last_seen_verified = current_verified;
                                            last_advance_time = Instant::now();
                                            info!(
                                                "[TRU-CHAIN-INTEGRITY] Buffered contiguous block #{} accepted & verified.",
                                                next_h
                                            );
                                        } else {
                                            realtime_reorder.insert(next_h, next_blk);
                                            break;
                                        }
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                realtime_reorder.insert(h, blk.clone());
                                info!(
                                    "[TRU-REORG-DEFENSE] Winning branch height {} > ancestor + 1, fetching missing fork blocks #{}-#{}...",
                                    h, ancestor_height + 1, h - 1
                                );
                                let missing_blocks: Vec<u64> = ((ancestor_height + 1)..h).collect();
                                if !missing_blocks.is_empty() {
                                    self.p2p_node.send_get_blocks_batch(&missing_blocks).await;
                                }
                            }
                        } else {
                            realtime_reorder.insert(h, blk.clone());
                        }
                    } else {
                        // Parent not in recent_block_hashes: alternative branch deeper than 1 block
                        realtime_reorder.insert(h, blk.clone());
                        if h >= current_verified {
                            let lookback_start = current_verified.saturating_sub(MAX_REORG_DEPTH).max(1);
                            let missing_fork: Vec<u64> = (lookback_start..h)
                                .filter(|ht| !realtime_reorder.contains_key(ht))
                                .take(32)
                                .collect();
                            if !missing_fork.is_empty() {
                                debug!(
                                    "[TRU-REORG-DEFENSE] Seeking common ancestor for block #{} up to {} blocks back. Requesting #{}-#{}...",
                                    h, MAX_REORG_DEPTH, missing_fork.first().unwrap_or(&0), missing_fork.last().unwrap_or(&0)
                                );
                                self.p2p_node.send_get_blocks_batch(&missing_fork).await;
                            }
                        }
                    }
                } else if h > current_verified + 1 {
                    // Gap detected with matching or distant tip: buffer and request missing in batch
                    realtime_reorder.insert(h, blk.clone());
                    let missing_end = h.min(current_verified + 64);
                    let missing: Vec<u64> = ((current_verified + 1)..=missing_end)
                        .filter(|missing_h| !realtime_reorder.contains_key(missing_h))
                        .collect();
                    if !missing.is_empty() {
                        self.p2p_node.send_get_blocks_batch(&missing).await;
                    }
                }
            }
        }
    }
})
    }

    /// Rolls back verified blockchain state and local indexes to target_height.
    /// Safely reverts mutations up to MAX_REORG_DEPTH (100 blocks) using block undo records.
    pub async fn rollback_to_height(
        &self,
        target_height: u64,
        all_verified_blocks: &mut Vec<BlockProto>,
        cache_path: &std::path::Path,
    ) {
        let current = self.verified_height.load(Ordering::Relaxed);
        if target_height >= current {
            return;
        }

        let depth = current - target_height;
        warn!(
            "[TRU-REORG-DEFENSE] Rolling back state by {} block(s) from #{} down to #{} (max permitted: {})...",
            depth, current, target_height, MAX_REORG_DEPTH
        );

        let mut undo_lock = self.block_undo_log.write().await;
        let mut vault_utxos = self.vault_utxos.write().await;
        let mut address_utxos = self.address_utxos.write().await;
        let mut address_balances = self.address_balances.write().await;
        let mut address_received = self.address_total_received.write().await;
        let mut address_sent = self.address_total_sent.write().await;
        let mut address_tx_map = self.address_tx_map.write().await;
        let mut all_swaps = self.all_swaps.write().await;
        let mut tx_cache = self.tx_cache.write().await;
        let mut block_cache = self.block_cache.write().await;

        for h in ((target_height + 1)..=current).rev() {
            let h_u32 = h as u32;
            if let Some(undo) = undo_lock.remove(&h_u32) {
                // 1. Vault UTXOs
                for id in &undo.vault_utxos_added {
                    vault_utxos.remove(id);
                }
                for entry in undo.vault_utxos_spent {
                    let id = format!("{}:{}", entry.txid, entry.vout);
                    vault_utxos.insert(id, entry);
                }

                // 2. Address UTXOs & Balances
                for id in &undo.address_utxos_added {
                    if let Some((addr, atoms)) = address_utxos.remove(id) {
                        if let Some(b) = address_balances.get_mut(&addr) {
                            *b = b.saturating_sub(atoms);
                        }
                    }
                }
                for (id, addr, atoms) in undo.address_utxos_spent {
                    address_utxos.insert(id, (addr.clone(), atoms));
                    let b = address_balances.entry(addr).or_insert(0);
                    *b = b.saturating_add(atoms);
                }
                for (addr, atoms) in undo.address_received_deltas {
                    if let Some(r) = address_received.get_mut(&addr) {
                        *r = r.saturating_sub(atoms);
                    }
                }
                for (addr, atoms) in undo.address_sent_deltas {
                    if let Some(s) = address_sent.get_mut(&addr) {
                        *s = s.saturating_sub(atoms);
                    }
                }

                // 3. Swaps
                for id in &undo.swaps_added {
                    all_swaps.remove(id);
                }
                for (id, prev_status, prev_spent, prev_txid, prev_secret) in undo.swaps_modified {
                    if let Some(s) = all_swaps.get_mut(&id) {
                        s.status = prev_status;
                        s.is_spent = prev_spent;
                        s.spent_by_txid = prev_txid;
                        s.secret_hex = prev_secret;
                    }
                }

                // 4. Caches & Tx Map
                for txid in &undo.txids {
                    tx_cache.remove(txid);
                    for list in address_tx_map.values_mut() {
                        list.retain(|t| t != txid);
                    }
                }
                block_cache.remove(&h_u32);
            } else {
                // Fallback retain if undo record was evicted
                vault_utxos.retain(|_, u| (u.block_height as u64) <= target_height);
                all_swaps.retain(|_, s| (s.block_height as u64) <= target_height);
                block_cache.remove(&h_u32);
            }
        }

        let total_vault: u64 = vault_utxos.values().map(|u| u.amount_atoms).sum();
        self.vault_balance_atoms.store(total_vault, Ordering::Relaxed);

        all_verified_blocks.retain(|b| (b.height as u64) <= target_height);
        let ancestor_hash = all_verified_blocks.last().map(|b| b.hash.clone()).unwrap_or_default();
        {
            let mut hash_lock = self.last_verified_hash.write().await;
            *hash_lock = ancestor_hash;
            let mut r = self.recent_block_hashes.write().await;
            r.retain(|&ht, _| ht <= target_height);
        }
        self.verified_height.store(target_height, Ordering::Relaxed);
        self.scanned_height.store(target_height, Ordering::Relaxed);
        let _ = super::chain_cache::save_cached_chain(cache_path, all_verified_blocks);
    }

    /// Processes all transactions in a block to detect HTLC creations and spends
    pub async fn process_block(&self, blk: &BlockProto) {
        let block_height = blk.height;
        let mut undo = BlockUndo { height: block_height, ..Default::default() };
        let now_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;

        let target_spk = self.vault_spk.read().await.clone();
        let header_opt = parse_tru_block_header(&blk.header);
        let block_time = header_opt.as_ref().map(|h| h.timestamp as u64).unwrap_or(now_sec as u64);
        let block_version = header_opt.as_ref().map(|h| h.version).unwrap_or(1);
        let block_hash = if !blk.hash.is_empty() {
            blk.hash.clone()
        } else if let Some(ref h) = header_opt {
            compute_tru_block_hash(h).unwrap_or_default()
        } else {
            String::new()
        };

        let mut block_txids: Vec<String> = Vec::new();
        let mut block_total_atoms = 0u64;

        for raw_tx in &blk.transactions {
            if let Some(parsed) = parse_any_tx(raw_tx.as_bytes()) {
                block_txids.push(parsed.txid.clone());
                undo.txids.push(parsed.txid.clone());
                for out in &parsed.outputs {
                    block_total_atoms = block_total_atoms.saturating_add(out.amount_atoms);
                }

                // Populate internal transaction details cache
                let cached_inputs: Vec<CachedTruTxIn> = parsed.inputs.iter().map(|in_tx| {
                    let is_cb = parsed.is_coinbase || in_tx.prev_txid.chars().all(|c| c == '0');
                    let is_quantum = in_tx.script_sig.len() >= 2000;
                    CachedTruTxIn {
                        prev_txid: in_tx.prev_txid.clone(),
                        prev_vout: in_tx.prev_vout,
                        script_sig: in_tx.script_sig.clone(),
                        is_coinbase: is_cb,
                        is_quantum,
                    }
                }).collect();

                let mut tx_has_quantum_out = false;
                let cached_outputs: Vec<CachedTruTxOut> = parsed.outputs.iter().map(|out| {
                    let is_htlc = inspect_htlc_script(&out.script_pubkey).is_some();
                    let is_vault = target_spk.as_ref().map(|s| s == &out.script_pubkey).unwrap_or(false);
                    let address = crate::relay::utxo_guard::script_pubkey_to_tru_address(&out.script_pubkey);
                    let is_quantum = address.as_ref().map(|a| a.starts_with("truq1:")).unwrap_or(false);
                    if is_quantum {
                        tx_has_quantum_out = true;
                    }
                    let display_target = address.clone().unwrap_or_else(|| format!("{}:{}", parsed.txid, out.vout));
                    CachedTruTxOut {
                        vout: out.vout,
                        amount_atoms: out.amount_atoms,
                        amount_tru: out.amount_atoms as f64 / 100_000_000.0,
                        script_pubkey: out.script_pubkey.clone(),
                        address,
                        display_target,
                        is_htlc,
                        is_vault,
                        is_quantum,
                    }
                }).collect();

                let is_quantum_tx = tx_has_quantum_out || cached_inputs.iter().any(|i| i.is_quantum);

                let cached_tx = CachedTruTx {
                    txid: parsed.txid.clone(),
                    version: block_version,
                    lock_time: 0,
                    is_coinbase: parsed.is_coinbase,
                    is_quantum: is_quantum_tx,
                    block_height: Some(block_height),
                    block_hash: Some(block_hash.clone()),
                    confirmations: 1,
                    timestamp: block_time,
                    inputs: cached_inputs,
                    outputs: cached_outputs.clone(),
                };

                {
                    let mut cache = self.tx_cache.write().await;
                    cache.insert(parsed.txid.to_lowercase(), cached_tx);
                }

                // Global Address UTXO & Balance Indexing
                for out in &cached_outputs {
                    if let Some(ref addr) = out.address {
                        let utxo_id = format!("{}:{}", parsed.txid, out.vout);
                        {
                            let mut u_lock = self.address_utxos.write().await;
                            u_lock.insert(utxo_id.clone(), (addr.clone(), out.amount_atoms));
                        }
                        undo.address_utxos_added.push(utxo_id);
                        undo.address_received_deltas.push((addr.clone(), out.amount_atoms));
                        {
                            let mut bal_lock = self.address_balances.write().await;
                            let bal = bal_lock.entry(addr.clone()).or_insert(0);
                            *bal = bal.saturating_add(out.amount_atoms);
                        }
                        {
                            let mut rec_lock = self.address_total_received.write().await;
                            let rec = rec_lock.entry(addr.clone()).or_insert(0);
                            *rec = rec.saturating_add(out.amount_atoms);
                        }
                        {
                            let mut tx_map = self.address_tx_map.write().await;
                            let list = tx_map.entry(addr.clone()).or_insert_with(Vec::new);
                            if !list.contains(&parsed.txid) {
                                list.push(parsed.txid.clone());
                            }
                        }
                    }
                }

                // 1. Check outputs for vault deposits and HTLC creation
                for out in &parsed.outputs {
                    if out.amount_atoms == 0 {
                        continue;
                    }

                    // Vault UTXO tracking
                    if let Some(ref my_spk) = target_spk {
                        if &out.script_pubkey == my_spk {
                            let utxo_id = format!("{}:{}", parsed.txid, out.vout);
                            let entry = UtxoEntry {
                                txid: parsed.txid.clone(),
                                vout: out.vout,
                                amount_atoms: out.amount_atoms,
                                amount_tru: out.amount_atoms as f64 / 100_000_000.0,
                                block_height,
                                script_pubkey: out.script_pubkey.clone(),
                            };
                            let mut utxos = self.vault_utxos.write().await;
                            utxos.insert(utxo_id.clone(), entry);
                            undo.vault_utxos_added.push(utxo_id.clone());
                            let mut mempool_utxos = self.vault_mempool_utxos.write().await;
                            mempool_utxos.remove(&utxo_id);
                        }
                    }

                    if let Some((hash160, timelock)) = inspect_htlc_script(&out.script_pubkey) {
                        let id = format!("{}:{}", parsed.txid, out.vout);
                        let net_atoms = if out.amount_atoms > crate::wallet::vault::TruLiteClient::HTLC_CLAIM_FEE_SURPLUS_ATOMS {
                            out.amount_atoms - crate::wallet::vault::TruLiteClient::HTLC_CLAIM_FEE_SURPLUS_ATOMS
                        } else {
                            out.amount_atoms
                        };
                        let amount_tru = net_atoms as f64 / 100_000_000.0;
                        let hours_remaining = if timelock > now_sec {
                            (timelock - now_sec) as f64 / 3600.0
                        } else {
                            0.0
                        };

                        let status = if hours_remaining >= self.min_margin_hours {
                            SwapStatus::Active
                        } else if hours_remaining > 0.0 {
                            SwapStatus::ExpiringSoon
                        } else {
                            SwapStatus::Expired
                        };

                        let tip = self.verified_height.load(Ordering::Relaxed);
                        let confs = if block_height > 0 && tip >= block_height as u64 {
                            (tip - block_height as u64 + 1) as u32
                        } else {
                            1
                        };
                        let is_safe = confs >= 6;

                        let swap = DiscoveredTruSwap {
                            txid: parsed.txid.clone(),
                            vout: out.vout,
                            block_height,
                            confirmations: confs,
                            is_safe,
                            amount_atoms: out.amount_atoms,
                            amount_tru,
                            hash160_hex: hex::encode(hash160),
                            timelock,
                            hours_remaining,
                            is_spent: false,
                            spent_by_txid: None,
                            status: status.clone(),
                            raw_script: out.script_pubkey.clone(),
                            secret_hex: None,
                        };

                        // Insert into swap registry
                        {
                            let mut swaps = self.all_swaps.write().await;
                            swaps.insert(id.clone(), swap.clone());
                        }
                        undo.swaps_added.push(id.clone());

                        // If active and an order exists for this swap hash, mark the order as matched and in-progress!
                        if status == SwapStatus::Active {
                            let hash_hex = hex::encode(hash160);
                            self.orderbook.mark_matched(&hash_hex).await;
                            self.orderbook.record_tru_lock_tx(&hash_hex, &parsed.txid).await;
                        }
                    }
                }

                // 2. Check inputs for vault spends and HTLC spends (Claim or Refund)
                for in_tx in &parsed.inputs {
                    let prev_id = format!("{}:{}", in_tx.prev_txid, in_tx.prev_vout);

                    // Track spend from address UTXO
                    let removed_utxo = {
                        let mut u_lock = self.address_utxos.write().await;
                        u_lock.remove(&prev_id)
                    };
                    if let Some((spent_addr, spent_atoms)) = removed_utxo {
                        undo.address_utxos_spent.push((prev_id.clone(), spent_addr.clone(), spent_atoms));
                        undo.address_sent_deltas.push((spent_addr.clone(), spent_atoms));
                        {
                            let mut bal_lock = self.address_balances.write().await;
                            if let Some(b) = bal_lock.get_mut(&spent_addr) {
                                *b = b.saturating_sub(spent_atoms);
                            }
                        }
                        {
                            let mut sent_lock = self.address_total_sent.write().await;
                            let sent = sent_lock.entry(spent_addr.clone()).or_insert(0);
                            *sent = sent.saturating_add(spent_atoms);
                        }
                        {
                            let mut tx_map = self.address_tx_map.write().await;
                            let list = tx_map.entry(spent_addr).or_insert_with(Vec::new);
                            if !list.contains(&parsed.txid) {
                                list.push(parsed.txid.clone());
                            }
                        }
                    }

                    if target_spk.is_some() {
                        let mut utxos = self.vault_utxos.write().await;
                        if let Some(spent_entry) = utxos.remove(&prev_id) {
                            undo.vault_utxos_spent.push(spent_entry);
                        }
                        let mut mempool_utxos = self.vault_mempool_utxos.write().await;
                        mempool_utxos.remove(&prev_id);
                    }

                    let mut swaps = self.all_swaps.write().await;
                    if let Some(existing) = swaps.get_mut(&prev_id) {
                        undo.swaps_modified.push((
                            prev_id.clone(),
                            existing.status.clone(),
                            existing.is_spent,
                            existing.spent_by_txid.clone(),
                            existing.secret_hex.clone(),
                        ));
                        existing.is_spent = true;
                        existing.spent_by_txid = Some(parsed.txid.clone());

                        // Try extracting revealed secret from script_sig (<sig> <0x20> <secret> 0x51)
                        if let Ok(sig_bytes) = hex::decode(&in_tx.script_sig) {
                            if sig_bytes.len() >= 34 && sig_bytes.last() == Some(&0x51) && sig_bytes[sig_bytes.len() - 34] == 0x20 {
                                let cand_secret = &sig_bytes[sig_bytes.len() - 33..sig_bytes.len() - 1];
                                let computed_h160 = crate::core::keys::hash160(cand_secret);
                                if hex::encode(computed_h160).to_lowercase() == existing.hash160_hex.to_lowercase() {
                                    let sec_hex = hex::encode(cand_secret);
                                    info!(
                                        "[TRU-SCANNER] ⚡ Revealed preimage secret {} extracted from on-chain claim of swap {}",
                                        sec_hex, prev_id
                                    );
                                    existing.secret_hex = Some(sec_hex.clone());
                                    self.orderbook.register_secret(&existing.hash160_hex, &sec_hex).await;
                                }
                            }
                        }

                        // Differentiate Claim vs Refund:
                        // Refund happens after timelock, Claim reveals secret
                        if existing.secret_hex.is_some() || existing.timelock > now_sec {
                            existing.status = SwapStatus::Claimed;
                        } else {
                            existing.status = SwapStatus::Refunded;
                        }
                        debug!(
                            "[TRU-SCANNER] Swap {} marked as {:?} by spend in tx {}",
                            prev_id, existing.status, parsed.txid
                        );
                    }
                }
            }
        }

        if target_spk.is_some() {
            let utxos = self.vault_utxos.read().await;
            let total_atoms: u64 = utxos.values().map(|u| u.amount_atoms).sum();
            self.vault_balance_atoms.store(total_atoms, Ordering::Relaxed);
        }

        // Evict confirmed transactions and conflicting double-spends from live mempool
        {
            let mut mempool = self.mempool.write().await;
            let mut confirmed_txids = std::collections::HashSet::new();
            let mut block_spent_inputs = std::collections::HashSet::new();

            for raw_tx in &blk.transactions {
                let trimmed = raw_tx.trim();
                if let Some(pos) = trimmed.find('|') {
                    let txid = trimmed[..pos].trim();
                    confirmed_txids.insert(txid.to_lowercase());
                }
                if let Some(parsed) = parse_any_tx(trimmed.as_bytes()) {
                    confirmed_txids.insert(parsed.txid.to_lowercase());
                    for inp in parsed.inputs {
                        let outpoint = format!("{}:{}", inp.prev_txid.to_lowercase(), inp.prev_vout);
                        block_spent_inputs.insert(outpoint);
                    }
                }
                if trimmed.len() == 64 {
                    confirmed_txids.insert(trimmed.to_lowercase());
                }
            }

            let before_len = mempool.len();
            mempool.retain(|txid, entry| {
                let tx_lower = txid.to_lowercase();
                if confirmed_txids.contains(&tx_lower) {
                    debug!("[TRU-MEMPOOL] Evicted confirmed tx {} (included in block #{})", txid, block_height);
                    return false;
                }
                for spent in &entry.spent_inputs {
                    if block_spent_inputs.contains(spent) {
                        info!(
                            "[TRU-MEMPOOL] ⚔️ Evicted conflicting tx {} (double-spend: input {} spent in block #{})",
                            txid, spent, block_height
                        );
                        return false;
                    }
                }
                true
            });
            let after_len = mempool.len();
            if before_len != after_len {
                info!(
                    "[TRU-MEMPOOL] Block #{} processed: {} txs evicted (confirmed or double-spent). Active mempool: {}",
                    block_height,
                    before_len - after_len,
                    after_len
                );
            }
            self.mempool_count.store(after_len as u64, Ordering::Relaxed);
        }

        // Cache full block details for Real-Time Blockchain Explorer
        let (prev_hash, merkle_root, bits, nonce) = if let Some(ref h) = header_opt {
            (h.prev_hash.clone(), h.merkle_root.clone(), h.bits, h.nonce)
        } else {
            (String::new(), String::new(), 0, 0)
        };

        let formatted_time = chrono::DateTime::<chrono::Utc>::from(
            UNIX_EPOCH + std::time::Duration::from_secs(block_time)
        ).format("%m/%d/%Y, %I:%M:%S %p UTC").to_string();

        let cached_blk = CachedTruBlock {
            height: block_height,
            hash: block_hash.clone(),
            prev_hash,
            merkle_root,
            timestamp: block_time,
            formatted_time,
            version: block_version,
            bits,
            nonce,
            tx_count: if !block_txids.is_empty() { block_txids.len() } else { blk.transactions.len() },
            size_bytes: blk.transactions.iter().map(|t| t.len()).sum::<usize>() + blk.header.len(),
            total_amount_tru: block_total_atoms as f64 / 100_000_000.0,
            txids: block_txids,
        };

        {
            let mut b_lock = self.block_cache.write().await;
            b_lock.insert(block_height, cached_blk);
        }

        {
            let mut undo_lock = self.block_undo_log.write().await;
            undo_lock.insert(block_height, undo);
            if undo_lock.len() > 384 {
                if let Some(oldest) = undo_lock.keys().next().copied() {
                    undo_lock.remove(&oldest);
                }
            }
        }
    }

    pub async fn get_recent_blocks(&self, limit: usize) -> Vec<CachedTruBlock> {
        let b_lock = self.block_cache.read().await;
        if limit == 0 {
            b_lock.values().rev().cloned().collect()
        } else {
            b_lock.values().rev().take(limit).cloned().collect()
        }
    }

    pub async fn get_block_by_query(&self, query: &str) -> Option<CachedTruBlock> {
        let q = query.trim();
        if let Ok(height) = q.parse::<u32>() {
            let b_lock = self.block_cache.read().await;
            if let Some(blk) = b_lock.get(&height) {
                return Some(blk.clone());
            }
        }
        let q_lower = q.to_lowercase();
        let b_lock = self.block_cache.read().await;
        b_lock.values().find(|b| b.hash.to_lowercase() == q_lower).cloned()
    }

    pub async fn get_explorer_stats(&self) -> serde_json::Value {
        let tip_height = self.verified_height.load(Ordering::Relaxed);
        let tip_hash = self.last_verified_hash.read().await.clone();
        let peers = self.p2p_node.peer_count();
        let mempool_lock = self.mempool.read().await;
        let mempool_count = mempool_lock.len();
        let mempool_atoms: u64 = mempool_lock.values().map(|m| m.total_out_atoms).sum();
        let mempool_tru = mempool_atoms as f64 / 100_000_000.0;
        let b_lock = self.block_cache.read().await;
        let cached_blocks_count = b_lock.len();
        let last_block_time = b_lock.values().last().map(|b| b.timestamp).unwrap_or(0);

        serde_json::json!({
            "tip_height": tip_height,
            "tip_hash": tip_hash,
            "peers": peers,
            "mempool_count": mempool_count,
            "mempool_atoms": mempool_atoms,
            "mempool_tru": mempool_tru,
            "cached_blocks_count": cached_blocks_count,
            "last_block_time": last_block_time,
            "is_scanning": self.is_scanning()
        })
    }

    /// Computes the Median Time Past (MTP) of the last 11 blocks according to consensus rules (BIP 113).
    /// Required for validating OP_CHECKLOCKTIMEVERIFY timelock expiration before broadcasting refund transactions.
    pub async fn get_median_time_past(&self) -> u64 {
        let cache = self.block_cache.read().await;
        if cache.is_empty() {
            return std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(7200);
        }
        let mut timestamps: Vec<u64> = cache.values().rev().take(11).map(|b| b.timestamp).collect();
        if timestamps.is_empty() {
            return 0;
        }
        timestamps.sort_unstable();
        let median_idx = timestamps.len() / 2;
        timestamps[median_idx]
    }

    pub async fn search_explorer(&self, query: &str) -> serde_json::Value {
        let q = query.trim();
        if q.is_empty() {
            return serde_json::json!({ "found": false, "error": "Query empty" });
        }

        // 0. Keyword query: "mempool"
        if q.eq_ignore_ascii_case("mempool") {
            let mempool = self.get_mempool().await;
            let mempool_atoms: u64 = mempool.iter().map(|t| t.total_out_atoms).sum();
            return serde_json::json!({
                "found": true,
                "target_type": "mempool",
                "count": mempool.len(),
                "total_atoms": mempool_atoms,
                "total_tru": mempool_atoms as f64 / 100_000_000.0,
                "transactions": mempool
            });
        }

        // 1. If numeric, search block by height
        if let Ok(height) = q.parse::<u32>() {
            let b_lock = self.block_cache.read().await;
            if let Some(blk) = b_lock.get(&height) {
                return serde_json::json!({
                    "found": true,
                    "target_type": "block",
                    "block": blk
                });
            }
        }

        let q_lower = q.to_lowercase();

        // 2. Search block by hash
        {
            let b_lock = self.block_cache.read().await;
            if let Some(blk) = b_lock.values().find(|b| b.hash.to_lowercase() == q_lower) {
                return serde_json::json!({
                    "found": true,
                    "target_type": "block",
                    "block": blk
                });
            }
        }

        // 3. Search transaction by txid
        let tx_opt = self.get_tx_details(q).await;
        if tx_opt.is_some() {
            return serde_json::json!({
                "found": true,
                "target_type": "tx",
                "tx": tx_opt
            });
        }

        // 4. Search by TRU address (starts with 'T' or 'truq1:')
        if q.starts_with('T') || q.starts_with("truq1:") {
            let balance_atoms = self.address_balances.read().await.get(q).copied().unwrap_or(0);
            let total_received_atoms = self.address_total_received.read().await.get(q).copied().unwrap_or(0);
            let total_sent_atoms = self.address_total_sent.read().await.get(q).copied().unwrap_or(0);

            let utxo_count = {
                let u_lock = self.address_utxos.read().await;
                u_lock.values().filter(|(a, _)| a == q).count()
            };

            let tx_ids = self.address_tx_map.read().await.get(q).cloned().unwrap_or_default();
            let tx_cache = self.tx_cache.read().await;
            let mut matched_txs = Vec::new();
            for txid in tx_ids.iter().rev() {
                if let Some(tx) = tx_cache.get(&txid.to_lowercase()) {
                    matched_txs.push(tx.clone());
                }
            }

            if matched_txs.is_empty() {
                for tx in tx_cache.values() {
                    if tx.outputs.iter().any(|o| o.address.as_deref() == Some(q)) {
                        matched_txs.push(tx.clone());
                    }
                }
            }

            if total_received_atoms > 0 || !matched_txs.is_empty() || (q.len() >= 26 && q.len() <= 35) || q.starts_with("truq1:") {
                return serde_json::json!({
                    "found": true,
                    "target_type": "address",
                    "address": q,
                    "balance_atoms": balance_atoms,
                    "balance_tru": balance_atoms as f64 / 100_000_000.0,
                    "total_received_atoms": total_received_atoms,
                    "total_received_tru": total_received_atoms as f64 / 100_000_000.0,
                    "total_sent_atoms": total_sent_atoms,
                    "total_sent_tru": total_sent_atoms as f64 / 100_000_000.0,
                    "utxo_count": utxo_count,
                    "txs": matched_txs
                });
            }
        }

        serde_json::json!({
            "found": false,
            "query": q,
            "error": "No matching block, transaction, or address found in current ledger index."
        })
    }

    /// Ingests an unconfirmed transaction from the P2P wire into the live mempool
    pub async fn add_mempool_tx(&self, claimed_txid: &str, raw: &[u8]) {
        let now_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let target_spk = self.vault_spk.read().await.clone();

        if let Some(parsed) = parse_any_tx(raw) {
            let txid = if !claimed_txid.is_empty() {
                claimed_txid.to_string()
            } else {
                parsed.txid.clone()
            };

            let mut is_vault_related = false;
            let mut is_htlc = false;
            let mut htlc_hash160 = None;
            let mut htlc_timelock = None;

            let mut mempool_outputs = Vec::new();
            let mut total_out_atoms = 0u64;

            for out in &parsed.outputs {
                total_out_atoms += out.amount_atoms;
                let mut out_is_vault = false;
                let mut out_is_htlc = false;

                if let Some(ref my_spk) = target_spk {
                    if &out.script_pubkey == my_spk {
                        out_is_vault = true;
                        is_vault_related = true;
                    }
                }

                if let Some((h160, tl)) = inspect_htlc_script(&out.script_pubkey) {
                    out_is_htlc = true;
                    is_htlc = true;
                    let h160_hex = hex::encode(h160);
                    htlc_hash160 = Some(h160_hex.clone());
                    htlc_timelock = Some(tl);
                    self.orderbook.record_tru_lock_tx(&h160_hex, &txid).await;
                    self.orderbook.mark_matched(&h160_hex).await;
                    info!(
                        "[TRU-MEMPOOL] ⚡ Detected 0-conf HTLC swap lock {} in mempool (hash160: {})",
                        txid, h160_hex
                    );
                }

                let address = crate::relay::utxo_guard::script_pubkey_to_tru_address(&out.script_pubkey);
                let is_quantum = address.as_ref().map(|a| a.starts_with("truq1:")).unwrap_or(false);

                mempool_outputs.push(MempoolOutput {
                    vout: out.vout,
                    amount_atoms: out.amount_atoms,
                    amount_tru: out.amount_atoms as f64 / 100_000_000.0,
                    script_pubkey: out.script_pubkey.clone(),
                    address,
                    is_vault: out_is_vault,
                    is_htlc: out_is_htlc,
                    is_quantum,
                });
            }

            // Check outputs for incoming vault transfers or change
            for out in &mempool_outputs {
                if out.is_vault {
                    is_vault_related = true;
                    let utxo_entry = UtxoEntry {
                        txid: txid.clone(),
                        vout: out.vout,
                        amount_atoms: out.amount_atoms,
                        amount_tru: out.amount_tru,
                        block_height: 0,
                        script_pubkey: out.script_pubkey.clone(),
                    };
                    let op = format!("{}:{}", txid, out.vout);
                    let mut mempool_utxos = self.vault_mempool_utxos.write().await;
                    mempool_utxos.insert(op, utxo_entry);
                    info!(
                        "[TRU-MEMPOOL] Registered 0-conf spendable change/deposit UTXO {}:{} ({:.8} TRU)",
                        txid, out.vout, out.amount_tru
                    );
                }
            }

            // Also check inputs to see if any spend our vault UTXOs
            if target_spk.is_some() {
                let mut spent_ops = Vec::new();
                for inp in &parsed.inputs {
                    let outpoint = format!("{}:{}", inp.prev_txid, inp.prev_vout);
                    spent_ops.push(outpoint);
                }
                {
                    let mut vault_utxos = self.vault_utxos.write().await;
                    for op in &spent_ops {
                        if vault_utxos.remove(op).is_some() {
                            is_vault_related = true;
                        }
                    }
                    let total_atoms: u64 = vault_utxos.values().map(|u| u.amount_atoms).sum();
                    self.vault_balance_atoms.store(total_atoms, Ordering::Relaxed);
                }
                {
                    let mut mempool_utxos = self.vault_mempool_utxos.write().await;
                    for op in &spent_ops {
                        if mempool_utxos.remove(op).is_some() {
                            is_vault_related = true;
                        }
                    }
                }
            }

            // Check inputs for HTLC claims revealing preimage secrets in 0-conf mempool
            for in_tx in &parsed.inputs {
                if let Ok(sig_bytes) = hex::decode(&in_tx.script_sig) {
                    if sig_bytes.len() >= 34 && sig_bytes.last() == Some(&0x51) && sig_bytes[sig_bytes.len() - 34] == 0x20 {
                        let cand_secret = &sig_bytes[sig_bytes.len() - 33..sig_bytes.len() - 1];
                        let computed_h160 = crate::core::keys::hash160(cand_secret);
                        let h160_hex = hex::encode(computed_h160);
                        let sec_hex = hex::encode(cand_secret);

                        let prev_id = format!("{}:{}", in_tx.prev_txid, in_tx.prev_vout);
                        let prev_id_lower = format!("{}:{}", in_tx.prev_txid.to_lowercase(), in_tx.prev_vout);

                        let mut swaps = self.all_swaps.write().await;
                        if swaps.contains_key(&prev_id) {
                            if let Some(existing) = swaps.get_mut(&prev_id) {
                                existing.secret_hex = Some(sec_hex.clone());
                            }
                        } else if let Some(existing) = swaps.get_mut(&prev_id_lower) {
                            existing.secret_hex = Some(sec_hex.clone());
                        }
                        for s in swaps.values_mut() {
                            if s.hash160_hex.eq_ignore_ascii_case(&h160_hex) {
                                s.secret_hex = Some(sec_hex.clone());
                            }
                        }

                        info!(
                            "[TRU-MEMPOOL] ⚡ Revealed preimage secret {} extracted from 0-conf mempool claim of swap {} (hash160: {})",
                            sec_hex, prev_id, h160_hex
                        );
                        self.orderbook.register_secret(&h160_hex, &sec_hex).await;
                    }
                }
            }

            let (target_atoms, change_atoms) = if is_htlc {
                let htlc_out = mempool_outputs.iter().find(|o| o.is_htlc).map(|o| o.amount_atoms).unwrap_or(total_out_atoms);
                (htlc_out, total_out_atoms.saturating_sub(htlc_out))
            } else if is_vault_related {
                if let Some(vault_out) = mempool_outputs.iter().find(|o| o.is_vault) {
                    (vault_out.amount_atoms, total_out_atoms.saturating_sub(vault_out.amount_atoms))
                } else if mempool_outputs.len() > 1 {
                    (mempool_outputs[0].amount_atoms, mempool_outputs[1].amount_atoms)
                } else {
                    (total_out_atoms, 0)
                }
            } else if mempool_outputs.len() >= 2 {
                (mempool_outputs[0].amount_atoms, mempool_outputs[1].amount_atoms)
            } else {
                (total_out_atoms, 0)
            };

            let spent_inputs: Vec<String> = parsed
                .inputs
                .iter()
                .map(|inp| format!("{}:{}", inp.prev_txid.to_lowercase(), inp.prev_vout))
                .collect();

            let entry = MempoolEntry {
                txid: txid.clone(),
                first_seen_sec: now_sec,
                size_bytes: raw.len(),
                inputs_count: parsed.inputs.len(),
                outputs_count: parsed.outputs.len(),
                total_out_atoms,
                total_out_tru: total_out_atoms as f64 / 100_000_000.0,
                target_amount_atoms: target_atoms,
                target_amount_tru: target_atoms as f64 / 100_000_000.0,
                change_amount_atoms: change_atoms,
                change_amount_tru: change_atoms as f64 / 100_000_000.0,
                is_vault_related,
                is_htlc,
                htlc_hash160,
                htlc_timelock,
                outputs: mempool_outputs,
                spent_inputs: spent_inputs.clone(),
            };

            let mut mempool = self.mempool.write().await;
            let mut replaced_txids = Vec::new();
            for (existing_txid, existing_entry) in mempool.iter() {
                if existing_txid != &txid {
                    if existing_entry.spent_inputs.iter().any(|inp| spent_inputs.contains(inp)) {
                        replaced_txids.push(existing_txid.clone());
                    }
                }
            }
            for old_txid in replaced_txids {
                info!(
                    "[TRU-MEMPOOL] 🔄 Replacing in-mempool tx {} with new conflicting tx {}",
                    old_txid, txid
                );
                mempool.remove(&old_txid);
            }

            mempool.insert(txid.clone(), entry.clone());
            self.mempool_count.store(mempool.len() as u64, Ordering::Relaxed);

            // Populate tx_cache with mempool entry
            let mempool_inputs: Vec<CachedTruTxIn> = parsed.inputs.iter().map(|in_tx| {
                let is_quantum = in_tx.script_sig.len() >= 2000;
                CachedTruTxIn {
                    prev_txid: in_tx.prev_txid.clone(),
                    prev_vout: in_tx.prev_vout,
                    script_sig: in_tx.script_sig.clone(),
                    is_coinbase: parsed.is_coinbase || in_tx.prev_txid.chars().all(|c| c == '0'),
                    is_quantum,
                }
            }).collect();
            let mut mem_has_quantum_out = false;
            let mempool_cached_outputs: Vec<CachedTruTxOut> = entry.outputs.iter().map(|out| {
                let display_target = out.address.clone().unwrap_or_else(|| format!("{}:{}", txid, out.vout));
                if out.is_quantum {
                    mem_has_quantum_out = true;
                }
                CachedTruTxOut {
                    vout: out.vout,
                    amount_atoms: out.amount_atoms,
                    amount_tru: out.amount_tru,
                    script_pubkey: out.script_pubkey.clone(),
                    address: out.address.clone(),
                    display_target,
                    is_htlc: out.is_htlc,
                    is_vault: out.is_vault,
                    is_quantum: out.is_quantum,
                }
            }).collect();
            let is_mem_quantum = mem_has_quantum_out || mempool_inputs.iter().any(|i| i.is_quantum);
            let cached_mem_tx = CachedTruTx {
                txid: txid.clone(),
                version: 1,
                lock_time: htlc_timelock.unwrap_or(0),
                is_coinbase: parsed.is_coinbase,
                is_quantum: is_mem_quantum,
                block_height: None,
                block_hash: None,
                confirmations: 0,
                timestamp: now_sec,
                inputs: mempool_inputs,
                outputs: mempool_cached_outputs,
            };
            {
                let mut cache = self.tx_cache.write().await;
                cache.insert(txid.to_lowercase(), cached_mem_tx);
            }

            info!(
                "[TRU-MEMPOOL] Added in-flight tx: {} (Net: {:.4} TRU, Total: {:.4} TRU, {} in / {} out, vault: {}, htlc: {}) [Total in Mempool: {}]",
                txid,
                target_atoms as f64 / 100_000_000.0,
                total_out_atoms as f64 / 100_000_000.0,
                parsed.inputs.len(),
                parsed.outputs.len(),
                is_vault_related,
                is_htlc,
                mempool.len()
            );
        }
    }

    /// Queries parsed TRU transaction details by txid with dynamic confirmation calculation
    pub async fn get_tx_details(&self, txid: &str) -> Option<CachedTruTx> {
        let clean = txid.trim().to_lowercase();
        let tip = self.verified_height.load(Ordering::Relaxed).max(self.scanned_height.load(Ordering::Relaxed));

        // 1. Check tx_cache
        {
            let cache = self.tx_cache.read().await;
            if let Some(entry) = cache.get(&clean) {
                let mut res = entry.clone();
                if let Some(h) = res.block_height {
                    if tip >= h as u64 {
                        res.confirmations = (tip - h as u64 + 1) as u32;
                    } else {
                        res.confirmations = 1;
                    }
                } else {
                    res.confirmations = 0;
                }
                return Some(res);
            }
        }

        // 2. Check mempool
        {
            let mem = self.mempool.read().await;
            if let Some(entry) = mem.get(&clean) {
                let inputs = entry.spent_inputs.iter().map(|s| {
                    let parts: Vec<&str> = s.split(':').collect();
                    let prev_txid = parts.first().unwrap_or(&"").to_string();
                    let prev_vout = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                    CachedTruTxIn {
                        prev_txid,
                        prev_vout,
                        script_sig: String::new(),
                        is_coinbase: false,
                        is_quantum: false,
                    }
                }).collect();
                let is_quantum_tx = entry.outputs.iter().any(|o| o.is_quantum);
                let outputs = entry.outputs.iter().map(|o| {
                    let display_target = o.address.clone().unwrap_or_else(|| format!("{}:{}", entry.txid, o.vout));
                    CachedTruTxOut {
                        vout: o.vout,
                        amount_atoms: o.amount_atoms,
                        amount_tru: o.amount_tru,
                        script_pubkey: o.script_pubkey.clone(),
                        address: o.address.clone(),
                        display_target,
                        is_htlc: o.is_htlc,
                        is_vault: o.is_vault,
                        is_quantum: o.is_quantum,
                    }
                }).collect();
                return Some(CachedTruTx {
                    txid: entry.txid.clone(),
                    version: 1,
                    lock_time: entry.htlc_timelock.unwrap_or(0),
                    is_coinbase: false,
                    is_quantum: is_quantum_tx,
                    block_height: None,
                    block_hash: None,
                    confirmations: 0,
                    timestamp: entry.first_seen_sec,
                    inputs,
                    outputs,
                });
            }
        }

        // 3. Check all_swaps
        {
            let swaps = self.all_swaps.read().await;
            let matching_swaps: Vec<DiscoveredTruSwap> = swaps.values()
                .filter(|s| s.txid.eq_ignore_ascii_case(&clean))
                .cloned()
                .collect();
            if !matching_swaps.is_empty() {
                let first = &matching_swaps[0];
                let block_h = first.block_height;
                let confs = if block_h > 0 && tip >= block_h as u64 {
                    (tip - block_h as u64 + 1) as u32
                } else {
                    first.confirmations.max(1)
                };
                let block_hash = self.recent_block_hashes.read().await.get(&(block_h as u64)).cloned();
                let outputs = matching_swaps.iter().map(|s| {
                    CachedTruTxOut {
                        vout: s.vout,
                        amount_atoms: s.amount_atoms,
                        amount_tru: s.amount_tru,
                        script_pubkey: s.raw_script.clone(),
                        address: None,
                        display_target: format!("{}:{}", s.txid, s.vout),
                        is_htlc: true,
                        is_vault: false,
                        is_quantum: false,
                    }
                }).collect();
                return Some(CachedTruTx {
                    txid: first.txid.clone(),
                    version: 1,
                    lock_time: first.timelock,
                    is_coinbase: false,
                    is_quantum: false,
                    block_height: Some(block_h),
                    block_hash,
                    confirmations: confs,
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                    inputs: vec![],
                    outputs,
                });
            }
        }

        // 4. Check vault_utxos
        {
            let utxos = self.vault_utxos.read().await;
            let matching_utxos: Vec<UtxoEntry> = utxos.values()
                .filter(|u| u.txid.eq_ignore_ascii_case(&clean))
                .cloned()
                .collect();
            if !matching_utxos.is_empty() {
                let first = &matching_utxos[0];
                let block_h = first.block_height;
                let confs = if block_h > 0 && tip >= block_h as u64 {
                    (tip - block_h as u64 + 1) as u32
                } else {
                    1
                };
                let block_hash = self.recent_block_hashes.read().await.get(&(block_h as u64)).cloned();
                let outputs = matching_utxos.iter().map(|u| {
                    CachedTruTxOut {
                        vout: u.vout,
                        amount_atoms: u.amount_atoms,
                        amount_tru: u.amount_tru,
                        script_pubkey: u.script_pubkey.clone(),
                        address: None,
                        display_target: format!("{}:{}", u.txid, u.vout),
                        is_htlc: false,
                        is_vault: true,
                        is_quantum: false,
                    }
                }).collect();
                return Some(CachedTruTx {
                    txid: first.txid.clone(),
                    version: 1,
                    lock_time: 0,
                    is_coinbase: false,
                    is_quantum: false,
                    block_height: Some(block_h),
                    block_hash,
                    confirmations: confs,
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                    inputs: vec![],
                    outputs,
                });
            }
        }

        None
    }
}

pub struct ParsedTx {
    pub txid: String,
    pub is_coinbase: bool,
    pub inputs: Vec<ParsedTxIn>,
    pub outputs: Vec<ParsedTxOut>,
}

pub struct ParsedTxIn {
    pub prev_txid: String,
    pub prev_vout: u32,
    pub script_sig: String,
}

pub struct ParsedTxOut {
    pub vout: u32,
    pub amount_atoms: u64,
    pub script_pubkey: String,
}

/// Parses TRU core pipe-delimited serialized transaction format:
/// `txid|isCoinbase|vin_count|vout_count|vin0_txid|vin0_vout|vin0_scriptsig|vin0_pubkey|vin0_seq|...|vout0_amt|vout0_spk|...`
pub fn parse_serialized_tx(raw: &str) -> Option<ParsedTx> {
    let parts: Vec<&str> = raw.split('|').collect();
    if parts.len() < 4 {
        return None;
    }

    let txid = parts[0].to_string();
    let is_coinbase = parts[1] == "1";
    let vin_count: usize = parts[2].parse().ok()?;
    let vout_count: usize = parts[3].parse().ok()?;

    let mut cursor = 4;
    let mut inputs = Vec::with_capacity(vin_count);
    for _ in 0..vin_count {
        if cursor + 5 > parts.len() {
            return None;
        }
        let prev_txid = parts[cursor].to_string();
        let prev_vout: u32 = parts[cursor + 1].parse().unwrap_or(0);
        let script_sig = parts[cursor + 2].to_string();
        inputs.push(ParsedTxIn {
            prev_txid,
            prev_vout,
            script_sig,
        });
        cursor += 5;
    }

    let mut outputs = Vec::with_capacity(vout_count);
    for vout_idx in 0..vout_count {
        if cursor + 2 > parts.len() {
            break;
        }
        let amount_atoms: u64 = parts[cursor].parse().unwrap_or(0);
        let script_pubkey = parts[cursor + 1].to_string();
        outputs.push(ParsedTxOut {
            vout: vout_idx as u32,
            amount_atoms,
            script_pubkey,
        });
        cursor += 2;
    }

    Some(ParsedTx {
        txid,
        is_coinbase,
        inputs,
        outputs,
    })
}

/// Reads a Bitcoin/TRU VarInt from byte buffer
pub fn read_var_int(buf: &[u8]) -> Option<(u64, usize)> {
    if buf.is_empty() {
        return None;
    }
    match buf[0] {
        0xff => {
            if buf.len() < 9 {
                return None;
            }
            let val = u64::from_le_bytes(buf[1..9].try_into().ok()?);
            Some((val, 9))
        }
        0xfe => {
            if buf.len() < 5 {
                return None;
            }
            let val = u32::from_le_bytes(buf[1..5].try_into().ok()?);
            Some((val as u64, 5))
        }
        0xfd => {
            if buf.len() < 3 {
                return None;
            }
            let val = u16::from_le_bytes(buf[1..3].try_into().ok()?);
            Some((val as u64, 3))
        }
        val => Some((val as u64, 1)),
    }
}

/// Parses raw binary serialized TRU transaction
pub fn parse_binary_tx(raw: &[u8]) -> Option<ParsedTx> {
    if raw.len() < 10 {
        return None;
    }
    let mut cursor = 0;
    if cursor + 4 > raw.len() {
        return None;
    }
    let _version = u32::from_le_bytes(raw[cursor..cursor + 4].try_into().ok()?);
    cursor += 4;

    let (vin_count, read) = read_var_int(&raw[cursor..])?;
    cursor += read;
    let mut inputs = Vec::new();
    for _ in 0..vin_count {
        if cursor + 32 + 4 > raw.len() {
            return None;
        }
        let prev_txid = hex::encode(&raw[cursor..cursor + 32]);
        cursor += 32;
        let prev_vout = u32::from_le_bytes(raw[cursor..cursor + 4].try_into().ok()?);
        cursor += 4;
        let (script_len, read) = read_var_int(&raw[cursor..])?;
        cursor += read;
        if cursor + (script_len as usize) > raw.len() {
            return None;
        }
        let script_sig = hex::encode(&raw[cursor..cursor + (script_len as usize)]);
        cursor += script_len as usize;
        if cursor + 4 > raw.len() {
            return None;
        }
        let _seq = u32::from_le_bytes(raw[cursor..cursor + 4].try_into().ok()?);
        cursor += 4;
        inputs.push(ParsedTxIn {
            prev_txid,
            prev_vout,
            script_sig,
        });
    }

    let (vout_count, read) = read_var_int(&raw[cursor..])?;
    cursor += read;
    let mut outputs = Vec::new();
    for vout_idx in 0..vout_count {
        if cursor + 8 > raw.len() {
            return None;
        }
        let amount_atoms = u64::from_le_bytes(raw[cursor..cursor + 8].try_into().ok()?);
        cursor += 8;
        let (spk_len, read) = read_var_int(&raw[cursor..])?;
        cursor += read;
        if cursor + (spk_len as usize) > raw.len() {
            return None;
        }
        let script_pubkey = hex::encode(&raw[cursor..cursor + (spk_len as usize)]);
        cursor += spk_len as usize;
        outputs.push(ParsedTxOut {
            vout: vout_idx as u32,
            amount_atoms,
            script_pubkey,
        });
    }

    let h1 = Sha256::digest(raw);
    let h2 = Sha256::digest(&h1);
    let txid = hex::encode(h2);

    Some(ParsedTx {
        txid,
        is_coinbase: false,
        inputs,
        outputs,
    })
}

/// Unified parser supporting pipe-delimited format, hex strings, and canonical binary wire transactions
pub fn parse_any_tx(raw: &[u8]) -> Option<ParsedTx> {
    if let Ok(text) = std::str::from_utf8(raw) {
        if text.contains('|') {
            return parse_serialized_tx(text);
        }
        if let Ok(bin) = hex::decode(text.trim()) {
            if let Some(parsed) = parse_binary_tx(&bin) {
                return Some(parsed);
            }
        }
    }
    parse_binary_tx(raw)
}

/// Inspects a scriptPubKey to detect if it matches an HTLC pattern.
/// Returns `Some((hash160, timelock))` if detected.
pub fn inspect_htlc_script(script_hex_or_asm: &str) -> Option<([u8; 20], u32)> {
    // Ignore OP_RETURN (0x6a) unspendable metadata
    if script_hex_or_asm.starts_with("6a") || script_hex_or_asm.starts_with("OP_RETURN") {
        return None;
    }

    // 1. Text ASM script format
    if script_hex_or_asm.contains("OP_CHECKLOCKTIMEVERIFY") {
        let tokens: Vec<&str> = script_hex_or_asm.split_whitespace().collect();
        for (i, &tok) in tokens.iter().enumerate() {
            if tok == "OP_CHECKLOCKTIMEVERIFY" && i > 0 {
                let timelock_str = tokens[i - 1];
                let timelock = if timelock_str.starts_with("0x") {
                    u32::from_str_radix(&timelock_str[2..], 16).unwrap_or(0)
                } else {
                    timelock_str.parse::<u32>().unwrap_or(0)
                };

                // Find hash160 token following OP_HASH160
                let mut hash160 = [0u8; 20];
                for (j, &h_tok) in tokens.iter().enumerate() {
                    if h_tok == "OP_HASH160" && j + 1 < tokens.len() {
                        let clean_hash = tokens[j + 1].trim_start_matches("0x");
                        if let Ok(bytes) = hex::decode(clean_hash) {
                            if bytes.len() == 20 {
                                hash160.copy_from_slice(&bytes);
                            }
                        }
                        break;
                    }
                }
                if hash160 != [0u8; 20] {
                    return Some((hash160, timelock));
                }
            }
        }
    }

    // 2. Binary Hex format (Canonical 103-byte TRU HTLC or standard CLTV)
    // 63a914<20-bytes>8821<33-bytes>ac6704<4-bytes-timelock-le>b17521<33-bytes>ac68
    let bytes = hex::decode(script_hex_or_asm).ok()?;
    if bytes.len() >= 40 && bytes.contains(&0xb1) { // 0xb1 = OP_CHECKLOCKTIMEVERIFY
        for i in 4..bytes.len() {
            if bytes[i] == 0xb1 && i >= 4 {
                // Preceding 4 bytes are timelock (LE)
                let timelock = u32::from_le_bytes([
                    bytes[i - 4],
                    bytes[i - 3],
                    bytes[i - 2],
                    bytes[i - 1],
                ]);

                // Look for 0xa9 0x14 (OP_HASH160 PUSH20)
                let mut hash160 = [0u8; 20];
                let mut found_hash = false;
                for j in 0..i.saturating_sub(21) {
                    if bytes[j] == 0xa9 && bytes[j + 1] == 0x14 && j + 22 <= bytes.len() {
                        hash160.copy_from_slice(&bytes[j + 2..j + 22]);
                        found_hash = true;
                        break;
                    }
                }
                if found_hash && hash160 != [0u8; 20] {
                    return Some((hash160, timelock));
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_serialized_tx_coinbase() {
        let raw = "daca4ceae9b5f0ee1bd30eaa93522666bcf37560b6a7310657eb1f7cac7ec139|1|1|1|0000000000000000000000000000000000000000000000000000000000000000|0|47454e455349533a313734353938323432373a30||4294967295|5000000000|76a9142004155a5d7120f6d3c792a9c881544f86ae4a7c88ac";
        let parsed = parse_serialized_tx(raw).expect("Must parse valid coinbase");
        assert_eq!(parsed.txid, "daca4ceae9b5f0ee1bd30eaa93522666bcf37560b6a7310657eb1f7cac7ec139");
        assert!(parsed.is_coinbase);
        assert_eq!(parsed.inputs.len(), 1);
        assert_eq!(parsed.outputs.len(), 1);
        assert_eq!(parsed.outputs[0].amount_atoms, 5_000_000_000);
    }

    #[test]
    fn test_inspect_canonical_htlc_script() {
        let hash160 = [0x55u8; 20];
        let claim_pubkey = [0x02u8; 33];
        let refund_pubkey = [0x03u8; 33];
        let timelock = 1750000000u32;

        let script = crate::tru_node::TruLiteClient::build_canonical_htlc_script(
            &hash160,
            &claim_pubkey,
            &refund_pubkey,
            timelock,
        );

        let script_hex = hex::encode(&script);
        let inspected = inspect_htlc_script(&script_hex).expect("Should detect canonical HTLC");
        assert_eq!(inspected.0, hash160);
        assert_eq!(inspected.1, timelock);
    }

    #[test]
    fn test_genesis_block_header_and_hash() {
        let raw_hdr = "1|0000000000000000000000000000000000000000000000000000000000000000|daca4ceae9b5f0ee1bd30eaa93522666bcf37560b6a7310657eb1f7cac7ec139|1745982427|503382015|46045855";
        let hdr = parse_tru_block_header(raw_hdr).expect("Should parse genesis header");
        assert_eq!(hdr.version, 1);
        assert_eq!(hdr.prev_hash, "0000000000000000000000000000000000000000000000000000000000000000");
        assert_eq!(hdr.merkle_root, "daca4ceae9b5f0ee1bd30eaa93522666bcf37560b6a7310657eb1f7cac7ec139");
        assert_eq!(hdr.timestamp, 1745982427);
        assert_eq!(hdr.bits, 0x1e00ffff);
        assert_eq!(hdr.nonce, 46045855);

        let hash = compute_tru_block_hash(&hdr).expect("Should compute hash");
        assert_eq!(hash, "b62fba2600030d97a06916b17694bec8d97ca14c1db682b2e57b424bd6000000");
    }

    #[test]
    fn test_compute_tru_merkle_root() {
        let txs = vec!["daca4ceae9b5f0ee1bd30eaa93522666bcf37560b6a7310657eb1f7cac7ec139|1|1|1|...".to_string()];
        let root = compute_tru_merkle_root(&txs);
        assert_eq!(root, "daca4ceae9b5f0ee1bd30eaa93522666bcf37560b6a7310657eb1f7cac7ec139");
    }

    #[tokio::test]
    async fn test_smart_mempool_double_spend_eviction() {
        let p2p = Arc::new(crate::tru_p2p::TruP2PNode::new(vec![]));
        let orderbook = Arc::new(crate::swap::Orderbook::new());
        let scanner = Arc::new(BlockScanner::new(p2p, orderbook));

        // Create tx1 spending input A:0
        let tx1_hex = "0100000001aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0000000000ffffffff0100e1f505000000001976a914111111111111111111111111111111111111111188ac00000000";
        let tx1_bin = hex::decode(tx1_hex).unwrap();
        scanner.add_mempool_tx("tx1_hash", &tx1_bin).await;

        // Create tx2 spending different input B:0
        let tx2_hex = "0100000001bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb0000000000ffffffff0100e1f505000000001976a914222222222222222222222222222222222222222288ac00000000";
        let tx2_bin = hex::decode(tx2_hex).unwrap();
        scanner.add_mempool_tx("tx2_hash", &tx2_bin).await;

        assert_eq!(scanner.get_mempool().await.len(), 2);

        // Simulate a block that spends input A:0 via a completely different transaction tx3 (mined instead of tx1)
        let tx3_hex = "0100000001aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa0000000000ffffffff0100e1f505000000001976a914333333333333333333333333333333333333333388ac00000000";
        let blk = BlockProto {
            hash: "mock_block_hash".to_string(),
            header: "1|0|0|0|0|0".to_string(),
            transactions: vec![tx3_hex.to_string()],
            chain_work: 1,
            height: 100,
            token_metadata: String::new(),
        };

        scanner.process_block(&blk).await;

        // tx1 MUST be evicted because its input A:0 was spent in the block (double-spend)!
        // tx2 MUST remain in mempool because its input B:0 is unspent!
        let remaining = scanner.get_mempool().await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].txid, "tx2_hash");
    }
}
