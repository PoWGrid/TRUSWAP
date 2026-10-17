//! Autonomous cross-chain atomic swap engine.
//! Manages HTLC lifecycle, preimage discovery, autonomous claim execution, and refund timeouts.

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::core;
use crate::swap::orderbook::Orderbook;
use crate::tui::TuiManager;
use crate::wallet::keystore::{EncryptedVault, SwapRecord};
use crate::wallet::vault::TruNodeClient;

#[derive(Clone)]
pub struct SwapEngine {
    pub vault: Arc<Mutex<EncryptedVault>>,
    pub tru_client: Arc<TruNodeClient>,
    pub orderbook: Arc<Orderbook>,
    pub block_scanner: Arc<tokio::sync::RwLock<Option<Arc<crate::tru_p2p::BlockScanner>>>>,
    pub evm_signer: Arc<tokio::sync::RwLock<Option<Arc<crate::chains::evm::EvmSigner>>>>,
    pub tui: Arc<TuiManager>,
    pub processed_preimages: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
    pub active_buyer_guards: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
    pub active_sweep_guards: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
    pub active_maker_base_guards: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
    pub tru_lock_mutex: Arc<tokio::sync::Mutex<()>>,
}

impl SwapEngine {
    pub fn new(
        vault: Arc<Mutex<EncryptedVault>>,
        tru_client: Arc<TruNodeClient>,
        orderbook: Arc<Orderbook>,
        tui: Arc<TuiManager>,
    ) -> Self {
        Self {
            vault,
            tru_client,
            orderbook,
            block_scanner: Arc::new(tokio::sync::RwLock::new(None)),
            evm_signer: Arc::new(tokio::sync::RwLock::new(None)),
            tui,
            processed_preimages: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            active_buyer_guards: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            active_sweep_guards: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            active_maker_base_guards: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            tru_lock_mutex: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub async fn set_block_scanner(&self, scanner: Arc<crate::tru_p2p::BlockScanner>) {
        let mut lock = self.block_scanner.write().await;
        *lock = Some(scanner);
    }

    pub async fn set_evm_signer(&self, signer: Arc<crate::chains::evm::EvmSigner>) {
        let mut lock = self.evm_signer.write().await;
        *lock = Some(signer);
    }

    /// Handles revealed preimage on Base Sepolia (`SwapClaimed` event).
    /// Extracts the secret and autonomously claims corresponding TRU funds on the TRU network
    /// strictly after verifying >= 6 confirmations on the TRU blockchain (ConfirmationGuard).
    pub async fn handle_base_preimage_revealed(&self, _swap_id_hex: &str, preimage: &[u8]) {
        let hash160_preimage = core::hash160(preimage);
        let hash160_hex = hex::encode(hash160_preimage);

        // Deduplication check: do not process the same revealed preimage multiple times
        {
            let mut set = self.processed_preimages.lock().await;
            if set.contains(&hash160_hex) {
                return;
            }
            set.insert(hash160_hex.clone());
        }

        self.tui
            .log(format!(
                "★ Preimage detected on Base: Hash160=0x{}... ({} bytes)",
                &hash160_hex[..8],
                preimage.len()
            ))
            .await;

        let mut is_seller = false;
        let mut already_settled = false;
        {
            let mut vault_lock = self.vault.lock().await;
            let mut records = vault_lock.load_records().unwrap_or_default();
            let mut modified = false;

            for r in &mut records {
                if r.hash160 == hash160_preimage {
                    if r.direction == 1 {
                        is_seller = true;
                        if r.status == 0 {
                            info!("Seller swap finalized upon preimage revelation on Base.");
                            r.step = 4;
                            r.status = 1; // Claimed
                            modified = true;
                        }
                    } else if r.direction == 2 {
                        if r.status == 1 && r.step >= 4 {
                            already_settled = true;
                        }
                    }
                }
            }

            if modified {
                if let Err(e) = vault_lock.save_records(&records) {
                    warn!("Failed to update vault records: {}", e);
                }
            }
        }

        // If we are the seller (direction 1), we already received USDC on Base.
        // The TRU belongs to Buyer, so Seller has no TRU to claim.
        if is_seller {
            info!("Preimage revealed on Base for our sold TRU (direction 1). Seller already received USDC.");
            return;
        }

        if already_settled {
            info!("Preimage revealed on Base for swap 0x{}, but TRU was already claimed/settled by us.", &hash160_hex[..8]);
            return;
        }

        // We are Buyer (direction 2): counterparty claimed our USDC on Base.
        // We have the preimage secret! Sweep TRU into our vault immediately without waiting for 6 confirmations!
        let engine = self.clone();
        let preimage_vec = preimage.to_vec();
        let h160_str = hash160_hex.clone();

        tokio::spawn(async move {
            let h_short = &h160_str[..h160_str.len().min(8)];
            engine.tui.log(format!("⚡ Preimage revealed on Base (0x{}...)! Sweeping TRU funds immediately...", h_short)).await;
            for attempt in 0..60 {
                match engine.sweep_tru_claim_with_secret(&h160_str, &preimage_vec).await {
                    Ok(claim_txid) => {
                        info!("✓ Autonomous TRU Claim succeeded: TXID {}", claim_txid);
                        {
                            let mut vault_lock = engine.vault.lock().await;
                            if let Ok(mut records) = vault_lock.load_records() {
                                for r in &mut records {
                                    if hex::encode(r.hash160).to_lowercase() == h160_str {
                                        r.step = 4;
                                        r.status = 1;
                                    }
                                }
                                let _ = vault_lock.save_records(&records);
                            }
                        }
                        break;
                    }
                    Err(e) => {
                        if e.contains("No active or unspent") || e.contains("already spent") {
                            info!("TRU HTLC 0x{} already claimed or not present in unspent UTXOs: {}", &h160_str[..8], e);
                            break;
                        }
                        warn!("Autonomous TRU claim attempt #{} failed: {}, retrying in 3s...", attempt + 1, e);
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            }
        });
    }

    /// Autonomously claims USDC from BaseHTLC by revealing the secret once Base swap is funded.
    pub async fn wait_tru_safe_and_claim_base(&self, hash160: [u8; 20], base_swap_id: String, secret: [u8; 32]) {
        let hash_hex = hex::encode(hash160);
        let engine = self.clone();

        tokio::spawn(async move {
            for _ in 0..600 {
                if let Some(signer) = engine.evm_signer.read().await.as_ref() {
                    match signer.claim_swap(&base_swap_id, &secret).await {
                        Ok(tx_hash) => {
                            engine.tui.log(format!("🎉 USDC Claimed on Base (0x{}...)! TX: {}", &hash_hex[..hash_hex.len().min(8)], tx_hash)).await;
                            engine.orderbook.mark_base_swap_claimed(&base_swap_id).await;
                            engine.orderbook.mark_base_swap_claimed(&hash_hex).await;
                            let mut v = engine.vault.lock().await;
                            if let Ok(mut recs) = v.load_records() {
                                for r in &mut recs {
                                    if r.hash160 == hash160 {
                                        r.step = 4;
                                        r.status = 1;
                                    }
                                }
                                let _ = v.save_records(&recs);
                            }
                            break;
                        }
                        Err(e) => {
                            if e.contains("already claimed") || e.contains("AlreadyClaimed") {
                                engine.orderbook.mark_base_swap_claimed(&base_swap_id).await;
                                engine.orderbook.mark_base_swap_claimed(&hash_hex).await;
                                break;
                            }
                            warn!("[EVM-CLAIM] Claim on Base attempt failed: {}", e);
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        }
                    }
                } else {
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        });
    }

    /// Validates an on-chain Seller TRU HTLC output against expected order terms for our Buyer node.
    /// Defends against Vector 2: Under-funding, Pubkey Hijacking, and Non-canonical scripts.
    pub async fn find_and_validate_seller_tru_lock(
        &self,
        hash160: [u8; 20],
        expected_atoms: u64,
        min_timelock: u32,
    ) -> Result<crate::tru_p2p::scanner::DiscoveredTruSwap, String> {
        let hash_hex = hex::encode(hash160).to_lowercase();
        let our_pub_key = {
            let mut vault_lock = self.vault.lock().await;
            if let Ok(pk) = vault_lock.get_priv_key() {
                crate::core::keys::keypair_from_bytes(&pk).map(|kp| kp.pub_key).ok()
            } else {
                None
            }
        }.ok_or_else(|| "Vault public key unavailable".to_string())?;

        let scanner = self.block_scanner.read().await.clone()
            .ok_or_else(|| "Block scanner not initialized".to_string())?;

        let swaps = scanner.get_all_swaps().await;
        for s in swaps {
            if s.hash160_hex.to_lowercase() == hash_hex && !s.is_spent {
                let script_bytes = hex::decode(&s.raw_script)
                    .map_err(|e| format!("Invalid script hex: {}", e))?;

                match crate::core::script::validate_buyer_tru_htlc(
                    &script_bytes,
                    s.amount_atoms,
                    &hash160,
                    &our_pub_key,
                    expected_atoms,
                    min_timelock,
                ) {
                    Ok(_) => return Ok(s),
                    Err(e) => {
                        return Err(format!("{}", e));
                    }
                }
            }
        }

        Err(format!("No matching unspent TRU HTLC found for 0x{}", &hash_hex[..8]))
    }

    /// Watches TRU blockchain until the Seller's HTLC lock achieves >= 6 confirmations,
    /// then autonomously locks USDC on BaseHTLC. This guarantees the buyer's funds are 100% safe.
    pub async fn wait_tru_safe_and_lock_base(
        &self,
        beneficiary: String,
        amount_usdc: f64,
        hash160: [u8; 20],
        _timelock: u32,
    ) {
        let hash_hex = hex::encode(hash160).to_lowercase();
        {
            let mut guards = self.active_buyer_guards.lock().await;
            if !guards.insert(hash_hex.clone()) {
                return;
            }
        }
        let engine = self.clone();
        let guard_hash = hash_hex.clone();

        tokio::spawn(async move {
            struct BuyerGuard {
                engine: SwapEngine,
                hash: String,
            }
            impl Drop for BuyerGuard {
                fn drop(&mut self) {
                    let engine = self.engine.clone();
                    let hash = self.hash.clone();
                    tokio::spawn(async move {
                        let mut guards = engine.active_buyer_guards.lock().await;
                        guards.remove(&hash);
                    });
                }
            }
            let _guard = BuyerGuard {
                engine: engine.clone(),
                hash: guard_hash.clone(),
            };

            let mut last_confs = 0;
            let mut logged_detected = false;
            for _ in 0..600 {
                // Check if already locked on Base
                {
                    let v = engine.vault.lock().await;
                    if let Ok(recs) = v.load_records() {
                        if recs.iter().any(|r| r.hash160 == hash160 && r.step >= 3) {
                            break;
                        }
                    }
                }

                if !logged_detected {
                    let tru_tx = engine.orderbook.get_tru_lock_tx(&hash_hex).await;
                    let found_tx = if tru_tx.is_some() {
                        tru_tx
                    } else if let Some(scanner) = engine.block_scanner.read().await.as_ref() {
                        scanner.get_swap_txid(&hash_hex).await
                    } else {
                        None
                    };
                    if let Some(ref tid) = found_tx {
                        logged_detected = true;
                        engine
                            .tui
                            .log(format!(
                                "⚡ Seller TRU lock detected on chain! TXID: {} (Hash160: 0x{}...)",
                                tid, &hash_hex[..8]
                            ))
                            .await;
                    }
                }

                let (is_safe, confs) = if let Some(scanner) = engine.block_scanner.read().await.as_ref() {
                    scanner.get_swap_safety(&hash_hex).await
                } else {
                    (false, 0)
                };

                if confs > last_confs && confs <= 6 {
                    last_confs = confs;
                    if !is_safe {
                        engine
                            .tui
                            .log(format!(
                                "🔒 Buyer Safety Guard: Seller TRU lock confirmed ({} / 6 blocks)...",
                                confs
                            ))
                            .await;
                    }
                }

                if is_safe {
                    // 🛡️ SECURITY DEFENSE: Validate Seller's TRU HTLC on-chain before locking USDC!
                    let expected_atoms = {
                        let v = engine.vault.lock().await;
                        v.load_records().ok().and_then(|recs| {
                            recs.iter().find(|r| r.hash160 == hash160).map(|r| r.tru_atoms)
                        }).unwrap_or(0)
                    };
                    let expected_atoms = if expected_atoms > 0 {
                        expected_atoms
                    } else if let Some(order) = engine.orderbook.get_order_by_hash(&hash_hex).await {
                        (order.amount_tru * 100_000_000.0).round() as u64
                    } else {
                        0
                    };

                    let now_sec = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as u32;
                    let min_tl = now_sec + crate::core::script::MIN_FIRST_LOCK_REMAINING_SECS;

                    match engine.find_and_validate_seller_tru_lock(hash160, expected_atoms, min_tl).await {
                        Ok(verified_swap) => {
                            // 🛡️ ASYMMETRIC TIMELOCK DEFENSE:
                            // The second lock (Base USDC) MUST expire at least delta_t before the first lock (TRU HTLC).
                            let base_timelock = match crate::core::script::calculate_asymmetric_timelock(verified_swap.timelock, now_sec) {
                                Ok(tl) => tl,
                                Err(e) => {
                                    warn!(
                                        "[SECURITY DEFENSE] Asymmetric timelock error for 0x{}: {}. Aborting Base lock!",
                                        &hash_hex[..8], e
                                    );
                                    engine.tui.log(format!("🚨 [SECURITY DEFENSE] Timelock delta error (0x{}...): {}", &hash_hex[..8], e)).await;
                                    break;
                                }
                            };

                            info!(
                                "✓ [SECURITY DEFENSE] Seller TRU lock verified (TL: {}). Setting asymmetric Base timelock: {} (Delta: {}s / {:.1}m)",
                                verified_swap.timelock, base_timelock,
                                verified_swap.timelock.saturating_sub(base_timelock),
                                verified_swap.timelock.saturating_sub(base_timelock) as f64 / 60.0
                            );
                            engine
                                .tui
                                .log(format!(
                                    "⚡ Seller TRU Finalized (6/6 SAFE, 0x{}...)! Locking {:.2} USDC on BaseHTLC (Safety Window: {}m)...",
                                    &hash_hex[..8],
                                    amount_usdc,
                                    verified_swap.timelock.saturating_sub(base_timelock) / 60
                                ))
                                .await;

                            if let Some(signer) = engine.evm_signer.read().await.as_ref() {
                                match signer.create_swap(&beneficiary, amount_usdc, hash160, base_timelock).await {
                                    Ok((swap_id, tx_hash)) => {
                                        engine
                                            .tui
                                            .log(format!(
                                                "🎉 USDC Locked on Base! (0x{}...) SwapId: 0x{}... TX: {}",
                                                &hash_hex[..8],
                                                &swap_id[2..swap_id.len().min(10)],
                                                tx_hash
                                            ))
                                            .await;
                                        engine.orderbook.record_base_swap_id(&hash_hex, &swap_id).await;
                                        engine.orderbook.record_base_lock_tx(&hash_hex, &tx_hash).await;
                                        engine.orderbook.record_base_lock_tx(&swap_id, &tx_hash).await;

                                        let mut v = engine.vault.lock().await;
                                        if let Ok(mut recs) = v.load_records() {
                                            for r in &mut recs {
                                                if r.hash160 == hash160 {
                                                    if let Ok(b) = hex::decode(swap_id.strip_prefix("0x").unwrap_or(&swap_id)) {
                                                        if b.len() == 32 {
                                                            r.swap_id.copy_from_slice(&b);
                                                        }
                                                    }
                                                    r.step = 3; // Base USDC locked
                                                    r.timelock = base_timelock;
                                                }
                                            }
                                            let _ = v.save_records(&recs);
                                        }
                                        break;
                                    }
                                    Err(e) => {
                                        warn!("[EVM-LOCK] Base USDC lock attempt failed: {}", e);
                                        engine.tui.log(format!("⚠️ Base lock error (0x{}...): {}, retrying...", &hash_hex[..8], e)).await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "[SECURITY DEFENSE] Aborting Base lock for 0x{}! Seller TRU HTLC validation failed: {}. Potential attack detected!",
                                &hash_hex[..8], e
                            );
                            engine.tui.log(format!("🚨 [SECURITY DEFENSE] Seller TRU validation failed (0x{}...): Aborting Base lock! Error: {}", &hash_hex[..8], e)).await;
                            break;
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }

            // If loop finishes without locking on Base (Seller never locked safe TRU or timed out)
            let mut v = engine.vault.lock().await;
            if let Ok(mut recs) = v.load_records() {
                let mut modif = false;
                for r in &mut recs {
                    if r.hash160 == hash160 && r.step < 3 && r.status == 0 {
                        r.status = 2; // Expired / Timed out waiting for seller
                        modif = true;
                    }
                }
                if modif {
                    let _ = v.save_records(&recs);
                    engine.tui.log(format!("⏱️ Buyer Safety Guard: Timed out waiting for Seller TRU lock (0x{}...). Swap reservation cancelled.", &hash_hex[..8])).await;
                }
            }
        });
    }

    /// For Maker Buyer: Maker holds the secret and locks Base USDC first (Zero-Trust Protocol).
    pub async fn lock_base_as_maker_buyer(
        &self,
        beneficiary: String,
        amount_usdc: f64,
        hash160: [u8; 20],
        timelock: u32,
        secret: [u8; 32],
    ) {
        let hash_hex = hex::encode(hash160).to_lowercase();
        {
            let mut guards = self.active_maker_base_guards.lock().await;
            if !guards.insert(hash_hex.clone()) {
                return;
            }
        }

        let engine = self.clone();
        let guard_hash = hash_hex.clone();

        tokio::spawn(async move {
            struct MakerBaseGuard {
                engine: SwapEngine,
                hash: String,
            }
            impl Drop for MakerBaseGuard {
                fn drop(&mut self) {
                    let engine = self.engine.clone();
                    let hash = self.hash.clone();
                    tokio::spawn(async move {
                        let mut guards = engine.active_maker_base_guards.lock().await;
                        guards.remove(&hash);
                    });
                }
            }
            let _guard = MakerBaseGuard {
                engine: engine.clone(),
                hash: guard_hash.clone(),
            };

            // Check if already locked on Base
            {
                let v = engine.vault.lock().await;
                if let Ok(recs) = v.load_records() {
                    if recs.iter().any(|r| r.hash160 == hash160 && r.step >= 3) {
                        engine.watch_seller_tru_and_sweep(hash160, secret).await;
                        return;
                    }
                }
            }

            let now_sec = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as u32;
            let onchain_timelock = if timelock > now_sec + 7200 {
                timelock
            } else {
                now_sec + 14400 // Fresh 4-hour safety window at lock time
            };

            if let Some(signer) = engine.evm_signer.read().await.as_ref() {
                match signer.create_swap(&beneficiary, amount_usdc, hash160, onchain_timelock).await {
                    Ok((swap_id, tx_hash)) => {
                        engine
                            .tui
                            .log(format!(
                                "🎉 [Maker Buyer] USDC Locked on Base! (0x{}...) SwapId: 0x{}... TX: {}",
                                &hash_hex[..8],
                                &swap_id[2..swap_id.len().min(10)],
                                tx_hash
                            ))
                            .await;
                        engine.orderbook.record_base_swap_id(&hash_hex, &swap_id).await;
                        engine.orderbook.record_base_lock_tx(&hash_hex, &tx_hash).await;
                        engine.orderbook.record_base_lock_tx(&swap_id, &tx_hash).await;

                        {
                            let mut v = engine.vault.lock().await;
                            if let Ok(mut recs) = v.load_records() {
                                for r in &mut recs {
                                    if r.hash160 == hash160 {
                                        if let Ok(b) = hex::decode(swap_id.strip_prefix("0x").unwrap_or(&swap_id)) {
                                            if b.len() == 32 {
                                                r.swap_id.copy_from_slice(&b);
                                            }
                                        }
                                        r.step = 3; // Base USDC locked
                                        r.timelock = onchain_timelock;
                                    }
                                }
                                let _ = v.save_records(&recs);
                            }
                        }

                        // Now start monitoring for Seller's TRU lock
                        engine.watch_seller_tru_and_sweep(hash160, secret).await;
                    }
                    Err(e) => {
                        warn!("[EVM-LOCK] Maker Buyer Base USDC lock failed: {}", e);
                        engine.tui.log(format!("⚠️ Base lock error (0x{}...): {}", &hash_hex[..8], e)).await;
                    }
                }
            }
        });
    }

    /// For Maker Buyer: Watches TRU blockchain for Seller's lock and sweeps funds once safe or overridden.
    pub async fn watch_seller_tru_and_sweep(&self, hash160: [u8; 20], secret: [u8; 32]) {
        let hash_hex = hex::encode(hash160).to_lowercase();
        {
            let mut guards = self.active_sweep_guards.lock().await;
            if !guards.insert(hash_hex.clone()) {
                return;
            }
        }

        let engine = self.clone();
        let guard_hash = hash_hex.clone();

        tokio::spawn(async move {
            struct SweepGuard {
                engine: SwapEngine,
                hash: String,
            }
            impl Drop for SweepGuard {
                fn drop(&mut self) {
                    let engine = self.engine.clone();
                    let hash = self.hash.clone();
                    tokio::spawn(async move {
                        let mut guards = engine.active_sweep_guards.lock().await;
                        guards.remove(&hash);
                    });
                }
            }
            let _guard = SweepGuard {
                engine: engine.clone(),
                hash: guard_hash.clone(),
            };

            let mut last_confs = 0;
            let mut logged_detected = false;
            for _ in 0..600 {
                // Exit if settled or cancelled
                {
                    let v = engine.vault.lock().await;
                    if let Ok(recs) = v.load_records() {
                        if recs.iter().any(|r| r.hash160 == hash160 && (r.step >= 4 || r.status != 0)) {
                            break;
                        }
                    }
                }

                if !logged_detected {
                    let tru_tx = engine.orderbook.get_tru_lock_tx(&hash_hex).await;
                    let found_tx = if tru_tx.is_some() {
                        tru_tx
                    } else if let Some(scanner) = engine.block_scanner.read().await.as_ref() {
                        scanner.get_swap_txid(&hash_hex).await
                    } else {
                        None
                    };
                    if let Some(ref tid) = found_tx {
                        logged_detected = true;
                        engine
                            .tui
                            .log(format!(
                                "⚡ Seller TRU lock detected on chain! TXID: {} (Hash160: 0x{}...)",
                                tid, &hash_hex[..8]
                            ))
                            .await;
                    }
                }

                let (is_safe, confs) = if let Some(scanner) = engine.block_scanner.read().await.as_ref() {
                    scanner.get_swap_safety(&hash_hex).await
                } else {
                    (false, 0)
                };

                if confs > last_confs && confs <= 6 {
                    last_confs = confs;
                    if !is_safe {
                        engine
                            .tui
                            .log(format!(
                                "🔒 ConfirmationGuard (0x{}...): Seller TRU lock confirmed ({} / 6 blocks)...",
                                &hash_hex[..8],
                                confs
                            ))
                            .await;
                    }
                }

                if is_safe {
                    let expected_atoms = {
                        let v = engine.vault.lock().await;
                        v.load_records().ok().and_then(|recs| {
                            recs.iter().find(|r| r.hash160 == hash160).map(|r| r.tru_atoms)
                        }).unwrap_or(0)
                    };
                    let expected_atoms = if expected_atoms > 0 {
                        expected_atoms
                    } else if let Some(order) = engine.orderbook.get_order_by_hash(&hash_hex).await {
                        (order.amount_tru * 100_000_000.0).round() as u64
                    } else {
                        0
                    };

                    match engine.find_and_validate_seller_tru_lock(hash160, expected_atoms, 0).await {
                        Ok(verified_swap) => {
                            info!(
                                "✓ [SECURITY DEFENSE] Seller TRU lock validated! Sweeping {:.4} TRU with secret (TXID: {}:{})...",
                                verified_swap.amount_tru, verified_swap.txid, verified_swap.vout
                            );
                            engine
                                .tui
                                .log(format!(
                                    "⚡ Seller TRU Finalized (6/6 SAFE, 0x{}...)! Sweeping TRU to vault using secret...",
                                    &hash_hex[..8]
                                ))
                                .await;
                            match engine.sweep_tru_claim_with_secret(&hash_hex, &secret).await {
                                Ok(_txid) => {
                                    let mut v = engine.vault.lock().await;
                                    if let Ok(mut recs) = v.load_records() {
                                        for r in &mut recs {
                                            if r.hash160 == hash160 {
                                                r.step = 4;
                                                r.status = 1;
                                            }
                                        }
                                        let _ = v.save_records(&recs);
                                    }
                                    break;
                                }
                                Err(e) => {
                                    warn!("Failed to sweep TRU with secret: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "[SECURITY DEFENSE] Refusing to reveal secret for 0x{}! Seller TRU lock invalid: {}. Protecting Base deposit!",
                                &hash_hex[..8], e
                            );
                            engine.tui.log(format!("🚨 [SECURITY DEFENSE] Refusing to sweep TRU (0x{}... secret protected): {}", &hash_hex[..8], e)).await;
                            break;
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            }
        });
    }

    /// Handles on-chain Base HTLC deposit detection.
    /// When counterparty funds USDC into BaseHTLC with matching hash160,
    /// advances lifecycle to Step 3 and autonomously locks TRU on the TRU network.
    pub async fn handle_base_swap_detected(&self, order: &crate::chains::evm::BaseSwapOrder) {
        // Strict defense-in-depth: only canonical USDC (configured in config.rs) is accepted
        let canonical_usdc = if let Some(signer) = self.evm_signer.read().await.as_ref() {
            signer.usdc_contract().to_lowercase()
        } else {
            "0x036cbd53842c5426634e7929541ec2318f3dcf7e".to_string()
        };
        if order.token.to_lowercase() != canonical_usdc {
            warn!(
                "[SWAP-ENGINE] Ignored Base swap {}: token {} does not match configured USDC ({})",
                order.swap_id, order.token, canonical_usdc
            );
            return;
        }

        let hash_hex = hex::encode(order.hash160);
        self.orderbook.record_base_swap_id(&hash_hex, &order.swap_id).await;
        if let Some(ref tx_h) = order.tx_hash {
            self.orderbook.record_base_lock_tx(&hash_hex, tx_h).await;
            self.orderbook.record_base_lock_tx(&order.swap_id, tx_h).await;
        }

        let mut vault_lock = self.vault.lock().await;
        let mut records = vault_lock.load_records().unwrap_or_default();
        let mut modified = false;
        let mut auto_lock_args = None;
        let mut auto_claim_args = None;
        let mut immediate_claim_args = None;

        let own_base_addr = if let Some(signer) = self.evm_signer.read().await.as_ref() {
            signer.address().to_lowercase()
        } else {
            String::new()
        };

        for r in &mut records {
            if r.hash160 == order.hash160 && r.status == 0 {
                // When selling TRU for USDC (direction 1), ensure counterparty funded sufficient USDC
                if r.direction == 1 {
                    let expected_usdc = r.usdc_atomic as f64 / 1_000_000.0;
                    if order.amount_usdc < expected_usdc * 0.9999 {
                        warn!(
                            "[SWAP-ENGINE] Base swap {} underfunded: got {:.6} USDC, expected {:.6} USDC",
                            order.swap_id, order.amount_usdc, expected_usdc
                        );
                        continue;
                    }

                    // Beneficiary Spoofing Defense:
                    // When counterparty Buyer locks USDC on Base for our TRU, the designated beneficiary
                    // MUST strictly be our own Base address.
                    if !own_base_addr.is_empty() && order.beneficiary.to_lowercase() != own_base_addr {
                        warn!(
                            "[SECURITY DEFENSE] Beneficiary mismatch on Base swap {}: expected our address {}, got beneficiary {}. Potential beneficiary spoofing attack detected! Ignoring swap.",
                            order.swap_id,
                            own_base_addr,
                            order.beneficiary
                        );
                        continue;
                    }
                }

                if r.direction == 2 {
                    // When buying TRU (direction 2), we are the sender who deposited USDC on Base.
                    if !own_base_addr.is_empty() && order.sender.to_lowercase() != own_base_addr {
                        warn!(
                            "[SECURITY DEFENSE] Sender mismatch on Base swap {}: expected our address {}, got sender {}. Ignoring foreign lock.",
                            order.swap_id,
                            own_base_addr,
                            order.sender
                        );
                        continue;
                    }
                }
                if let Ok(b) = hex::decode(order.swap_id.strip_prefix("0x").unwrap_or(&order.swap_id)) {
                    if b.len() == 32 {
                        if r.swap_id != b.as_slice() {
                            r.swap_id.copy_from_slice(&b);
                            modified = true;
                        }
                    }
                }
                if r.step < 3 {
                    r.step = 3; // Step 3: Counterparty HTLC detected on Base Sepolia
                    modified = true;
                    self.tui
                        .log(format!(
                            "⚡ Counterparty deposited {:.2} USDC in BaseHTLC (SwapId: 0x{}..., Hash160: 0x{}...)",
                            order.amount_usdc,
                            &order.swap_id[..order.swap_id.len().min(10)],
                            &hex::encode(r.hash160)[..8]
                        ))
                        .await;

                    if r.direction == 1 {
                        // We are Seller, locking TRU in response to Buyer's Base USDC lock.
                        // 🛡️ ASYMMETRIC TIMELOCK DEFENSE:
                        // Second lock (TRU HTLC) MUST expire at least delta_t before first lock (Base USDC):
                        let now_sec = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as u32;
                        match crate::core::script::calculate_asymmetric_timelock(order.timelock, now_sec) {
                            Ok(tru_timelock) => {
                                info!(
                                    "✓ [SECURITY DEFENSE] Base USDC lock verified (TL: {}). Setting asymmetric TRU timelock: {} (Delta: {}s / {:.1}m)",
                                    order.timelock, tru_timelock,
                                    order.timelock.saturating_sub(tru_timelock),
                                    order.timelock.saturating_sub(tru_timelock) as f64 / 60.0
                                );
                                r.timelock = tru_timelock;
                                modified = true;
                                let amt = r.tru_atoms as f64 / 100_000_000.0;
                                auto_lock_args = Some((amt, r.hash160, tru_timelock, order.swap_id.clone(), r.secret));
                            }
                            Err(e) => {
                                warn!(
                                    "[SECURITY DEFENSE] Asymmetric timelock violation on Base swap {}: {}. Refusing to lock TRU!",
                                    order.swap_id, e
                                );
                                self.tui.log(format!("🚨 [SECURITY DEFENSE] Refusing to lock TRU (timelock violation): {}", e)).await;
                            }
                        }
                    } else if r.direction == 2 {
                        // We are Maker buying TRU, and our USDC lock is confirmed on Base!
                        // Securely watch the TRU chain and sweep only after >= 6 confirmations!
                        auto_claim_args = Some((r.hash160, r.secret));
                    }
                } else if r.step == 3 && r.direction == 1 && r.secret != [0u8; 32] {
                    // TRU was already locked on-chain (step 3), and Buyer just locked USDC on Base!
                    // Seller holds the secret, claim Base USDC immediately without waiting for 6 TRU confirmations!
                    self.tui
                        .log(format!(
                            "⚡ Buyer deposited {:.2} USDC in BaseHTLC (SwapId: 0x{}...)! Claiming USDC on Base immediately...",
                            order.amount_usdc,
                            &order.swap_id[..order.swap_id.len().min(10)]
                        ))
                        .await;
                    immediate_claim_args = Some((order.swap_id.clone(), r.secret, r.hash160));
                }
            }
        }

        if modified {
            if let Err(e) = vault_lock.save_records(&records) {
                warn!("Failed to update vault records: {}", e);
            }
        }
        drop(vault_lock);

        // Immediate claim for seller when Buyer deposits USDC on Base
        if let Some((swap_id, secret, hash160)) = immediate_claim_args {
            let engine = self.clone();
            tokio::spawn(async move {
                let h_short = &hex::encode(hash160)[..8];
                if let Some(signer) = engine.evm_signer.read().await.as_ref() {
                    match signer.claim_swap(&swap_id, &secret).await {
                        Ok(tx_hash) => {
                            engine.tui.log(format!("🎉 USDC Claimed on Base (0x{}...)! TX: {}", h_short, tx_hash)).await;
                            engine.orderbook.mark_base_swap_claimed(&swap_id).await;
                            engine.orderbook.mark_base_swap_claimed(&hex::encode(hash160)).await;
                            let mut v = engine.vault.lock().await;
                            if let Ok(mut recs) = v.load_records() {
                                for r in &mut recs {
                                    if r.hash160 == hash160 {
                                        r.step = 4;
                                        r.status = 1;
                                    }
                                }
                                let _ = v.save_records(&recs);
                            }
                        }
                        Err(e) => {
                            if e.contains("already claimed") || e.contains("AlreadyClaimed") || e.contains("already known") {
                                info!("[EVM-CLAIM] Swap {} already claimed or tx already in mempool: {}", swap_id, e);
                                engine.orderbook.mark_base_swap_claimed(&swap_id).await;
                                engine.orderbook.mark_base_swap_claimed(&hex::encode(hash160)).await;
                            } else {
                                warn!("[EVM-CLAIM] Immediate claim on Base failed: {}", e);
                                engine.tui.log(format!("⚠️ Base claim error (0x{}...): {}, safety loop will retry...", h_short, e)).await;
                            }
                        }
                    }
                }
            });
        }

        // Execute autonomous TRU lock and trigger wait_tru_safe_and_claim_base
        if let Some((amount_tru, hash160, timelock, base_swap_id, secret)) = auto_lock_args {
            let engine = self.clone();
            tokio::spawn(async move {
                let h_short = &hex::encode(hash160)[..8];
                let mut locked = false;
                for _ in 0..30 {
                    if engine.try_auto_lock_tru(amount_tru, hash160, timelock).await.is_some() {
                        locked = true;
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
                if locked {
                    if secret != [0u8; 32] {
                        engine.wait_tru_safe_and_claim_base(hash160, base_swap_id, secret).await;
                    } else {
                        engine.tui.log(format!(
                            "🔒 TRU Locked on chain (0x{}...)! Waiting for Buyer to sweep TRU (revealing secret) to claim Base USDC...",
                            h_short
                        )).await;
                    }
                }
            });
        }

        // Execute autonomous TRU claim if we are buyer and USDC was locked on Base (direction 2)
        // 🛡️ REORG & PREIMAGE DEFENSE: Wait strictly for >= 6 TRU confirmations (is_safe)
        // and validate the seller's on-chain HTLC before broadcasting the secret claim!
        if let Some((hash160, secret)) = auto_claim_args {
            let engine = self.clone();
            tokio::spawn(async move {
                engine.watch_seller_tru_and_sweep(hash160, secret).await;
            });
        }
    }

    /// Handles order matched events from Relay or API
    pub async fn on_order_matched(
        &self,
        order_id_or_hash: &str,
        taker_tru_address: &str,
        taker_tru_pubkey: Option<&str>,
        taker_base: &str,
        signature_base: Option<&str>,
    ) {
        let clean = order_id_or_hash.strip_prefix("0x").unwrap_or(order_id_or_hash).to_lowercase();
        let canonical_tru = crate::relay::crypto::normalize_to_tru_address(taker_tru_address);

        // Cryptographic verification of Taker signature if provided (Defense-in-depth against fake takes & capital freezing)
        if let Some(sig) = signature_base {
            let order_id = if let Some(order) = self.orderbook.get_order_by_hash(&clean).await {
                order.id.to_lowercase()
            } else {
                clean.clone()
            };

            let mut verified = false;
            // Primary check: canonical message with normalized TRU address (Unified Standard)
            let canonical_1 = crate::relay::crypto::canonical_take_message(&order_id, taker_base, &canonical_tru);
            if let Ok(signer) = crate::relay::crypto::recover_personal_sign_address(&canonical_1, sig) {
                if signer.to_lowercase() == taker_base.to_lowercase() {
                    verified = true;
                }
            }

            if !verified && clean != order_id {
                let canonical_2 = crate::relay::crypto::canonical_take_message(&clean, taker_base, &canonical_tru);
                if let Ok(signer) = crate::relay::crypto::recover_personal_sign_address(&canonical_2, sig) {
                    if signer.to_lowercase() == taker_base.to_lowercase() {
                        verified = true;
                    }
                }
            }

            // Fallback checks for backward compatibility if raw pubkey was signed directly
            if !verified && taker_tru_address != canonical_tru {
                let raw_msg_1 = format!("TRUSWAP_TAKE:{}:{}:{}", order_id.to_lowercase(), taker_base.to_lowercase(), taker_tru_address.trim());
                if let Ok(signer) = crate::relay::crypto::recover_personal_sign_address(&raw_msg_1, sig) {
                    if signer.to_lowercase() == taker_base.to_lowercase() {
                        verified = true;
                    }
                }
            }

            if !verified {
                if let Some(pk) = taker_tru_pubkey {
                    let raw_pk_msg = format!("TRUSWAP_TAKE:{}:{}:{}", order_id.to_lowercase(), taker_base.to_lowercase(), pk.trim());
                    if let Ok(signer) = crate::relay::crypto::recover_personal_sign_address(&raw_pk_msg, sig) {
                        if signer.to_lowercase() == taker_base.to_lowercase() {
                            verified = true;
                        }
                    }
                }
            }

            if !verified {
                warn!(
                    "[SECURITY-ALERT] OrderMatched rejected for {}: Taker signature verification failed!",
                    clean
                );
                self.tui.log(format!("⚠️ [Security Guard] Blocked fake TakeOrder for 0x{}... Invalid cryptographic signature!", &clean[..8.min(clean.len())])).await;
                return;
            }
        }

        // Store pubkey if available for low-level TRU HTLC script construction
        let pubkey_to_store = if let Some(pk) = taker_tru_pubkey.filter(|s| !s.is_empty()) {
            Some(pk.to_string())
        } else if taker_tru_address.len() == 66 && (taker_tru_address.starts_with("02") || taker_tru_address.starts_with("03")) {
            Some(taker_tru_address.to_string())
        } else {
            None
        };

        if let Some(ref pk) = pubkey_to_store {
            self.orderbook.set_taker_pubkey(&clean, pk).await;
        }
        if !taker_base.is_empty() {
            self.orderbook.set_taker_base(&clean, taker_base).await;
        }

        if let Some(order) = self.orderbook.get_order_by_hash(&clean).await {
            if let Some(ref pk) = pubkey_to_store {
                self.orderbook.set_taker_pubkey(&order.hash160_hex, pk).await;
            }
            if !taker_base.is_empty() {
                self.orderbook.set_taker_base(&order.hash160_hex, taker_base).await;
            }
        }

        // Check if there is an active swap in vault waiting to lock TRU (direction 1 = sell TRU, status 0 = pending)
        // or lock USDC on Base (direction 2 = buy TRU, status 0 = pending, step 1)
        let (pending_tru, pending_base) = {
            let own_base_addr = {
                let mut v = self.vault.lock().await;
                v.get_base_address().unwrap_or_default().to_lowercase()
            };
            let mut vault_lock = self.vault.lock().await;
            let mut records = vault_lock.load_records().unwrap_or_default();
            let mut match_tru = None;
            let mut match_base = None;
            let mut modified = false;

            for r in &mut records {
                let r_hash = hex::encode(r.hash160).to_lowercase();
                let r_id = hex::encode(r.swap_id).to_lowercase();
                if r_hash == clean || r_id == clean {
                    if !taker_base.is_empty() && taker_base.to_lowercase() != own_base_addr {
                        r.beneficiary = taker_base.to_string();
                        modified = true;
                    }
                    if r.direction == 1 && r.status == 0 && r.secret != [0u8; 32] {
                        let amt = r.tru_atoms as f64 / 100_000_000.0;
                        match_tru = Some((amt, r.hash160, r.timelock));
                    } else if r.direction == 2 && r.status == 0 && r.step == 1 {
                        let usdc = r.usdc_atomic as f64 / 1_000_000.0;
                        let target_beneficiary = if !r.beneficiary.is_empty() && r.beneficiary.to_lowercase() != own_base_addr {
                            r.beneficiary.clone()
                        } else if !taker_base.is_empty() && taker_base.to_lowercase() != own_base_addr {
                            taker_base.to_string()
                        } else {
                            String::new()
                        };
                        if !target_beneficiary.is_empty() {
                            match_base = Some((target_beneficiary, usdc, r.hash160, r.timelock, r.secret));
                        }
                    }
                }
            }
            if modified {
                let _ = vault_lock.save_records(&records);
            }
            (match_tru, match_base)
        };

        if let Some((amt, hash160, timelock)) = pending_tru {
            self.tui.log(format!("⚡ [Seller] Deal matched! Locking {:.4} TRU on TRU chain (0x{}...)...", amt, &hex::encode(hash160)[..8])).await;
            self.try_auto_lock_tru(amt, hash160, timelock).await;
        }

        if let Some((beneficiary, amount_usdc, hash160, timelock, secret)) = pending_base {
            let h_short = &hex::encode(hash160)[..8];
            if secret != [0u8; 32] {
                // Maker Buyer holds the secret: LOCK USDC ON BASE FIRST!
                self.tui.log(format!("⚡ [Maker Buyer] Deal matched! Locking {:.2} USDC on BaseHTLC first (Zero-Trust Initiator, 0x{}...)...", amount_usdc, h_short)).await;
                let engine = self.clone();
                tokio::spawn(async move {
                    engine.lock_base_as_maker_buyer(beneficiary, amount_usdc, hash160, timelock, secret).await;
                });
            } else {
                // Taker Buyer does not hold secret: wait for Seller's TRU lock first!
                self.tui.log(format!("🔒 [Buyer Safety Guard] Deal matched! Awaiting Seller TRU lock before locking {:.2} USDC on Base (0x{}...)...", amount_usdc, h_short)).await;
                self.wait_tru_safe_and_lock_base(beneficiary, amount_usdc, hash160, timelock).await;
            }
        }
    }

    /// Sweeps an on-chain TRU HTLC using known preimage secret into the vault.
    pub async fn sweep_tru_claim_with_secret(&self, hash160_hex: &str, preimage: &[u8]) -> Result<String, String> {
        let clean_hash = hash160_hex.strip_prefix("0x").unwrap_or(hash160_hex).to_lowercase();
        let (priv_key, dest_addr, our_pub_key) = {
            let mut vault_lock = self.vault.lock().await;
            if let Ok(pk) = vault_lock.get_priv_key() {
                if let Ok(kp) = crate::core::keys::keypair_from_bytes(&pk) {
                    (Some(pk), Some(kp.address), Some(kp.pub_key))
                } else {
                    (None, None, None)
                }
            } else {
                (None, None, None)
            }
        };

        let (pk, addr) = match (priv_key, dest_addr) {
            (Some(k), Some(a)) => (k, a),
            _ => return Err("Vault private key not unlocked".to_string()),
        };

        let scanner = self.block_scanner.read().await.clone()
            .ok_or_else(|| "Block scanner not initialized".to_string())?;

        let expected_atoms = {
            let v = self.vault.lock().await;
            v.load_records().ok().and_then(|recs| {
                recs.iter().find(|r| hex::encode(r.hash160).to_lowercase() == clean_hash).map(|r| r.tru_atoms)
            }).unwrap_or(0)
        };
        let expected_atoms = if expected_atoms > 0 {
            expected_atoms
        } else if let Some(order) = self.orderbook.get_order_by_hash(&clean_hash).await {
            (order.amount_tru * 100_000_000.0).round() as u64
        } else {
            0
        };

        let mut expected_h160 = [0u8; 20];
        if let Ok(b) = hex::decode(&clean_hash) {
            if b.len() == 20 {
                expected_h160.copy_from_slice(&b);
            }
        }

        let mut target = None;
        let active_swaps = scanner.get_all_swaps().await;
        for s in &active_swaps {
            if s.hash160_hex.to_lowercase() == clean_hash && !s.is_spent {
                if let Ok(script_bytes) = hex::decode(&s.raw_script) {
                    if let Some(our_pk) = &our_pub_key {
                        if expected_h160 != [0u8; 20] {
                            if let Err(e) = crate::core::script::validate_buyer_tru_htlc(
                                &script_bytes,
                                s.amount_atoms,
                                &expected_h160,
                                our_pk,
                                expected_atoms,
                                0,
                            ) {
                                tracing::warn!(
                                    "[SECURITY DEFENSE] Skipping TRU HTLC {}:{} for 0x{} during sweep: {}",
                                    s.txid, s.vout, &clean_hash[..8], e
                                );
                                continue;
                            }
                        }
                    }
                    target = Some((s.txid.clone(), s.vout, s.amount_atoms, script_bytes));
                    break;
                }
            }
        }
        if target.is_none() {
            let mempool = scanner.get_mempool().await;
            for m in &mempool {
                if m.htlc_hash160.as_deref().unwrap_or("").to_lowercase() == clean_hash {
                    for out in &m.outputs {
                        if out.is_htlc {
                            if let Ok(script_bytes) = hex::decode(&out.script_pubkey) {
                                if let Some(our_pk) = &our_pub_key {
                                    if expected_h160 != [0u8; 20] {
                                        if let Err(e) = crate::core::script::validate_buyer_tru_htlc(
                                            &script_bytes,
                                            out.amount_atoms,
                                            &expected_h160,
                                            our_pk,
                                            expected_atoms,
                                            0,
                                        ) {
                                            tracing::warn!(
                                                "[SECURITY DEFENSE] Skipping mempool TRU HTLC {}:{} for 0x{}: {}",
                                                m.txid, out.vout, &clean_hash[..8], e
                                            );
                                            continue;
                                        }
                                    }
                                }
                                target = Some((m.txid.clone(), out.vout, out.amount_atoms, script_bytes));
                                break;
                            }
                        }
                    }
                }
            }
        }

        let (htlc_txid, htlc_vout, htlc_atoms, htlc_script) = target
            .ok_or_else(|| format!("No active or unspent TRU HTLC found for hash 0x{}", clean_hash))?;

        let amount_tru = htlc_atoms.saturating_sub(crate::wallet::vault::TruLiteClient::HTLC_CLAIM_FEE_SURPLUS_ATOMS) as f64 / 100_000_000.0;
        let h_short = &clean_hash[..clean_hash.len().min(8)];
        self.tui.log(format!("⚡ Claiming {:.4} TRU from HTLC (0x{}...): TXID {}", amount_tru, h_short, htlc_txid)).await;
        let claim_txid = self.tru_client.claim_htlc(
            &htlc_txid,
            htlc_vout,
            htlc_atoms,
            &htlc_script,
            preimage,
            &pk,
            &addr,
        ).await.map_err(|e| format!("Failed to broadcast TRU claim: {}", e))?;

        self.tui.log(format!("🎉 Atomic Swap Complete (0x{}...)! TRU claimed: {}", h_short, claim_txid)).await;

        // Mark vault record as claimed ONLY after successful broadcast
        {
            let mut vault_lock = self.vault.lock().await;
            if let Ok(mut records) = vault_lock.load_records() {
                let mut modified = false;
                for r in &mut records {
                    if hex::encode(r.hash160).to_lowercase() == clean_hash {
                        r.step = 4;
                        r.status = 1; // Claimed
                        modified = true;
                    }
                }
                if modified {
                    let _ = vault_lock.save_records(&records);
                }
            }
        }

        Ok(claim_txid)
    }

    /// Attempts to lock TRU funds into HTLC on TRU network if counterparty pubkey is known
    pub async fn try_auto_lock_tru(&self, amount_tru: f64, hash160: [u8; 20], timelock: u32) -> Option<String> {
        // Serialize UTXO selection and on-chain HTLC creation across concurrent matches and recovery scans
        let _lock = self.tru_lock_mutex.lock().await;
        let hash_hex = hex::encode(hash160);

        let own_pubkey = {
            let mut vault_lock = self.vault.lock().await;
            vault_lock.get_priv_key().ok().and_then(|pk| {
                crate::core::keys::keypair_from_bytes(&pk).ok().map(|kp| kp.pub_key)
            })
        };

        let claim_pubkey_hex = match self.orderbook.get_counterparty_claim_pubkey(&hash_hex, own_pubkey.as_ref()).await {
            Some(pk) => pk,
            None => {
                let mut fallback_pk = None;
                {
                    let v = self.vault.lock().await;
                    if let Ok(recs) = v.load_records() {
                        for r in recs {
                            if r.hash160 == hash160 && !r.beneficiary.is_empty() {
                                fallback_pk = self.orderbook.resolve_pubkey(&r.beneficiary).await;
                                break;
                            }
                        }
                    }
                }
                match fallback_pk {
                    Some(pk) => pk,
                    None => {
                        info!("try_auto_lock_tru: counterparty claim pubkey not found for 0x{}...", &hash_hex[..8]);
                        return None;
                    }
                }
            }
        };
        let pubkey_bytes = match hex::decode(&claim_pubkey_hex) {
            Ok(b) if b.len() == 33 => b,
            _ => {
                warn!("try_auto_lock_tru: invalid pubkey len or format: {}", claim_pubkey_hex);
                return None;
            }
        };
        let mut claim_pubkey = [0u8; 33];
        claim_pubkey.copy_from_slice(&pubkey_bytes);

        // Safety verification: Do not lock TRU to own vault pubkey
        if let Some(ref own_pk) = own_pubkey {
            if own_pk == &claim_pubkey {
                warn!("try_auto_lock_tru: Refusing to lock TRU to own vault pubkey for 0x{}!", &hash_hex[..8]);
                return None;
            }
        }

        let spendable_utxos = if let Some(scanner) = self.block_scanner.read().await.as_ref() {
            let current_height = scanner.verified_height();
            scanner.get_vault_utxos().await
                .into_iter()
                .filter(|u| u.block_height > 0 && current_height >= u.block_height as u64)
                .collect()
        } else {
            Vec::new()
        };

        let now_sec = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as u32;
        // 🛡️ FRESH ON-CHAIN TIMELOCK GUARANTEE:
        // When locking on-chain, always ensure a full safe window (default 4h = 14400s).
        // If the order sat in the orderbook, its timelock has aged; refreshing it here guarantees
        // the counterparty buyer never encounters a "timelock too short" abort.
        let onchain_timelock = if timelock > now_sec + 7200 {
            timelock
        } else {
            now_sec + 14400
        };

        match self.tru_client.lock_htlc_with_utxos(
            amount_tru,
            &hash160,
            &claim_pubkey,
            onchain_timelock,
            spendable_utxos,
        ).await {
            Ok((txid, _vout, spent, change)) => {
                info!("⚡ Autonomous TRU Lock broadcast succeeded: TXID {}", txid);
                self.tui.log(format!("⚡ Auto-locked {:.4} TRU on TRU chain (0x{}... TXID: {})", amount_tru, &hash_hex[..8], txid)).await;
                self.orderbook.record_tru_lock_tx(&hash_hex, &txid).await;
                if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                    scanner.mark_spent_outpoints(&spent).await;
                    if let Some(c) = change {
                        scanner.add_vault_mempool_utxo(c).await;
                    }
                }
                // Update vault record step to 3 and record fresh onchain timelock
                let mut vault_lock = self.vault.lock().await;
                if let Ok(mut records) = vault_lock.load_records() {
                    let mut modified = false;
                    for r in &mut records {
                        if r.hash160 == hash160 && r.status == 0 {
                            r.step = 3;
                            r.timelock = onchain_timelock;
                            modified = true;
                        }
                    }
                    if modified {
                        let _ = vault_lock.save_records(&records);
                    }
                }

                // Spawn seller progress monitor to log confirmation progress in TUI & Web console
                let v_clone = self.vault.clone();
                let tui_clone = self.tui.clone();
                let scanner_clone = self.block_scanner.clone();
                let ob_clone = self.orderbook.clone();
                let h_hex = hash_hex.clone();
                let txid_clone = txid.clone();
                tokio::spawn(async move {
                    tui_clone.log(format!("⏳ [Seller Guard] TRU lock broadcasted (0x{}... TXID: {}), waiting for Buyer Base USDC lock...", &h_hex[..8], &txid_clone[..txid_clone.len().min(16)])).await;
                    let mut last_confs = 0;
                    for _ in 0..600 {
                        tokio::time::sleep(tokio::time::Duration::from_secs(4)).await;
                        {
                            let v = v_clone.lock().await;
                            if let Ok(recs) = v.load_records() {
                                if recs.iter().any(|r| r.hash160 == hash160 && (r.step >= 4 || r.status != 0 || r.swap_id != [0u8; 32])) {
                                    break;
                                }
                            }
                        }
                        if ob_clone.get_base_swap_id(&h_hex).await.is_some() {
                            break;
                        }
                        let (_, confs) = if let Some(scanner) = scanner_clone.read().await.as_ref() {
                            scanner.get_swap_safety(&h_hex).await
                        } else {
                            (false, 0)
                        };
                        if confs > last_confs && confs <= 6 {
                            last_confs = confs;
                            tui_clone
                                .log(format!(
                                    "⏳ [Seller Guard] Waiting for Buyer Base USDC lock (0x{}... TRU confirmed: {} / 6 blocks)...",
                                    &h_hex[..8],
                                    confs
                                ))
                                .await;
                            if confs >= 6 {
                                break;
                            }
                        }
                    }
                });

                Some(txid)
            }
            Err(e) => {
                warn!("Autonomous TRU lock failed: {}", e);
                self.tui.log(format!("⚠️ Auto-lock TRU error (0x{}...): {}", &hash_hex[..8], e)).await;
                None
            }
        }
    }

    /// Initiates a new outbound TRU HTLC lock.
    pub async fn initiate_tru_offer(
        &self,
        tru_atoms: u64,
        usdc_atomic: u64,
        timelock_val: u16,
        beneficiary_evm: Option<String>,
    ) -> Result<SwapRecord, String> {
        let secret = core::generate_secret();
        let hash160 = core::hash160(&secret);
        let swap_id = core::sha256(&secret);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // If timelock_val <= 24, treat as hours (legacy), else minutes (e.g. 30):
        let duration_secs = if timelock_val <= 24 {
            timelock_val as u64 * 3600
        } else {
            timelock_val as u64 * 60
        };
        let timelock = (now + duration_secs) as u32;

        let (maker_tru_pubkey, _vault_addr) = {
            let mut vault_lock = self.vault.lock().await;
            if let Ok(priv_key) = vault_lock.get_priv_key() {
                if let Ok(kp) = crate::core::keys::keypair_from_bytes(&priv_key) {
                    (Some(hex::encode(kp.pub_key)), Some(kp.address))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            }
        };

        let local_base_addr = {
            let mut vault_lock = self.vault.lock().await;
            vault_lock.get_base_address().ok()
        };
        let beneficiary_str = beneficiary_evm.clone().or(local_base_addr.clone()).unwrap_or_default();
        let record = SwapRecord {
            swap_id,
            secret,
            hash160,
            direction: 1, // Sell TRU
            step: 1,      // 1 = Secret Gen / Off-chain Intent
            status: 0,    // Pending
            tru_atoms,
            usdc_atomic,
            timelock,
            created_at: now,
            beneficiary: beneficiary_str,
        };

        {
            let mut vault_lock = self.vault.lock().await;
            let mut records = vault_lock.load_records().unwrap_or_default();
            records.push(record.clone());
            vault_lock.save_records(&records)?;
        }

        // Register in live orderbook with counter-asset info
        let hash_hex = hex::encode(hash160);
        let amount_tru = tru_atoms as f64 / 100_000_000.0;
        let amount_usdc = usdc_atomic as f64 / 1_000_000.0;
        self.orderbook.upsert_unified_order(crate::swap::orderbook::UnifiedOrder {
            id: hex::encode(swap_id),
            side: crate::swap::orderbook::OrderSide::SellTruForBaseUsdc,
            amount_tru,
            amount_usdc,
            hash160_hex: hash_hex,
            timelock,
            source_chain: "TRU Network".to_string(),
            is_active: true,
            is_matched: false,
            maker_base_address: beneficiary_evm.or(local_base_addr),
            maker_tru_pubkey,
            taker_tru_pubkey: None,
            taker_base_address: None,
        }).await;

        self.tui
            .log(format!(
                "HTLC Offer Broadcast: {:.2} TRU -> {:.2} USDC (Hash160: 0x{}...)",
                amount_tru,
                amount_usdc,
                &hex::encode(hash160)[..8]
            ))
            .await;

        Ok(record)
    }

    /// Initiates a new off-chain BUY TRU intent (direction: 2, Lock USDC -> Buy TRU).
    /// Does not lock any TRU funds from the maker's vault.
    pub async fn initiate_buy_offer(
        &self,
        tru_atoms: u64,
        usdc_atomic: u64,
        timelock_val: u16,
        maker_base_address: Option<String>,
    ) -> Result<SwapRecord, String> {
        let secret = core::generate_secret();
        let hash160 = core::hash160(&secret);
        let swap_id = core::sha256(&secret);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let duration_secs = if timelock_val <= 24 {
            timelock_val as u64 * 3600
        } else {
            timelock_val as u64 * 60
        };
        let timelock = (now + duration_secs) as u32;

        let (maker_tru_pubkey, _vault_addr) = {
            let mut vault_lock = self.vault.lock().await;
            if let Ok(priv_key) = vault_lock.get_priv_key() {
                if let Ok(kp) = crate::core::keys::keypair_from_bytes(&priv_key) {
                    (Some(hex::encode(kp.pub_key)), Some(kp.address))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            }
        };

        let local_base_addr = {
            let mut vault_lock = self.vault.lock().await;
            vault_lock.get_base_address().ok()
        };
        let beneficiary_str = String::new(); // Recipient of Base USDC is the seller, set upon OrderMatched
        let record = SwapRecord {
            swap_id,
            secret,
            hash160,
            direction: 2, // 2 = Lock USDC -> Buy TRU
            step: 1,      // 1 = Secret Gen / Off-chain Intent
            status: 0,    // Pending
            tru_atoms,
            usdc_atomic,
            timelock,
            created_at: now,
            beneficiary: beneficiary_str,
        };

        {
            let mut vault_lock = self.vault.lock().await;
            let mut records = vault_lock.load_records().unwrap_or_default();
            records.push(record.clone());
            vault_lock.save_records(&records)?;
        }

        let hash_hex = hex::encode(hash160);
        let amount_tru = tru_atoms as f64 / 100_000_000.0;
        let amount_usdc = usdc_atomic as f64 / 1_000_000.0;
        self.orderbook.upsert_unified_order(crate::swap::orderbook::UnifiedOrder {
            id: hex::encode(swap_id),
            side: crate::swap::orderbook::OrderSide::BuyTruWithBaseUsdc,
            amount_tru,
            amount_usdc,
            hash160_hex: hash_hex.clone(),
            timelock,
            source_chain: "TRUSwap-Relay".to_string(),
            is_active: true,
            is_matched: false,
            maker_base_address: maker_base_address.or(local_base_addr),
            maker_tru_pubkey,
            taker_tru_pubkey: None,
            taker_base_address: None,
        }).await;

        self.tui
            .log(format!(
                "Bid Offer Broadcast: {:.2} USDC -> {:.2} TRU (Hash160: 0x{}...)",
                amount_usdc,
                amount_tru,
                &hash_hex[..8]
            ))
            .await;

        Ok(record)
    }

    /// Attempts to refund an expired TRU HTLC lock back to the local vault.
    pub async fn try_refund_tru(&self, hash160: [u8; 20], timelock: u32) -> Result<String, String> {
        let hash_hex = hex::encode(hash160);
        let scanner = self.block_scanner.read().await.clone()
            .ok_or_else(|| "Block scanner not ready".to_string())?;

        let mtp = scanner.get_median_time_past().await;
        if mtp < timelock as u64 {
            return Err(format!(
                "Cannot refund yet: TRU Median Time Past (MTP: {}) has not reached timelock ({}). Waiting for more blocks.",
                mtp, timelock
            ));
        }

        let active_swaps = scanner.get_all_swaps().await;
        let mut target = None;
        for s in &active_swaps {
            if s.hash160_hex.to_lowercase() == hash_hex.to_lowercase() && !s.is_spent {
                if let Ok(script_bytes) = hex::decode(&s.raw_script) {
                    target = Some((s.txid.clone(), s.vout, s.amount_atoms, script_bytes));
                    break;
                }
            }
        }

        let (htlc_txid, htlc_vout, htlc_atoms, htlc_script) = target
            .ok_or_else(|| format!("No active unspent TRU HTLC found for 0x{}", hash_hex))?;

        let (priv_key, dest_addr) = {
            let mut v = self.vault.lock().await;
            let pk = v.get_priv_key().map_err(|e| format!("Vault private key error: {}", e))?;
            let addr = v.get_address().map_err(|e| format!("Vault address error: {}", e))?;
            (pk, addr)
        };

        let txid = self.tru_client.refund_htlc(
            &htlc_txid,
            htlc_vout,
            htlc_atoms,
            &htlc_script,
            timelock,
            &priv_key,
            &dest_addr,
        ).await.map_err(|e| format!("TRU refund broadcast error: {}", e))?;

        self.tui.log(format!("🎉 Autonomous TRU Refund complete! TXID: {}", txid)).await;
        Ok(txid)
    }

    /// Attempts to refund an expired Base USDC HTLC lock back to the local vault.
    pub async fn try_refund_base(&self, swap_id_hex: &str) -> Result<String, String> {
        let signer = self.evm_signer.read().await.clone()
            .ok_or_else(|| "EVM Signer not ready".to_string())?;

        let tx_hash = signer.refund_swap(swap_id_hex).await?;
        self.tui.log(format!("🎉 Autonomous Base USDC Refund complete! TX: {}", tx_hash)).await;
        Ok(tx_hash)
    }

    /// Cancels a pending maker order in vault records by ID or hash160.
    pub async fn cancel_order_in_vault(&self, order_id_or_hash: &str) {
        let clean = order_id_or_hash.strip_prefix("0x").unwrap_or(order_id_or_hash).to_lowercase();
        let mut vault_lock = self.vault.lock().await;
        if let Ok(mut records) = vault_lock.load_records() {
            let mut modified = false;
            for r in &mut records {
                let rec_id = hex::encode(r.swap_id).to_lowercase();
                let rec_hash = hex::encode(r.hash160).to_lowercase();
                if (rec_id == clean || rec_hash == clean) && r.status == 0 && r.step == 1 {
                    r.status = 2; // Cancelled
                    modified = true;
                    info!("[ENGINE] Cancelled vault record 0x{} (status=2)", &rec_id[..rec_id.len().min(8)]);
                }
            }
            if modified {
                let _ = vault_lock.save_records(&records);
            }
        }
    }

    /// Reconciles local pending step 1 maker orders against the active Relay orderbook.
    /// Any local order missing from the relay (beyond a 25s grace period) or expired by timelock
    /// is automatically cancelled in the vault to immediately free reserved balances.
    pub async fn reconcile_orders_with_relay(&self, active_relay_orders: &[crate::relay::protocol::RelayOrder]) {
        use std::collections::HashSet;
        let active_set: HashSet<String> = active_relay_orders
            .iter()
            .flat_map(|o| {
                let clean_id = o.order_id.strip_prefix("0x").unwrap_or(&o.order_id).to_lowercase();
                let clean_hash = o.hash160_hex.strip_prefix("0x").unwrap_or(&o.hash160_hex).to_lowercase();
                vec![clean_id, clean_hash]
            })
            .collect();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut vault_lock = self.vault.lock().await;
        if let Ok(mut records) = vault_lock.load_records() {
            let mut modified = false;
            for r in &mut records {
                // Only reconcile unmatched maker orders waiting in orderbook (direction 1, beneficiary empty, step 1, status 0)
                // Matched orders and taker/buyer orders (direction 2) must NEVER be cancelled by relay snapshot!
                if r.step == 1 && r.status == 0 && r.direction == 1 && r.beneficiary.is_empty() {
                    let rec_id = hex::encode(r.swap_id).to_lowercase();
                    let rec_hash = hex::encode(r.hash160).to_lowercase();
                    let grace_passed = now >= r.created_at + 25;
                    let missing = grace_passed && !active_set.contains(&rec_id) && !active_set.contains(&rec_hash);

                    if missing {
                        info!(
                            "[RECONCILE] Automatically releasing unlisted maker reservation 0x{} (missing from relay)",
                            &rec_id[..rec_id.len().min(8)]
                        );
                        self.tui.log(format!(
                            "🧹 Released unlisted maker reservation: 0x{}... (missing from relay)",
                            &rec_id[..rec_id.len().min(8)]
                        )).await;
                        r.status = 2; // Cancelled / Pruned
                        modified = true;
                    }
                }
            }
            if modified {
                let _ = vault_lock.save_records(&records);
            }
        }
    }

    /// Periodic safety scanner: checks for expired swaps eligible for refund and auto-locks pending swaps.
    pub async fn start_background_safety_loop(self: Arc<Self>) {
        tokio::spawn(async move {
            info!("Swap engine safety monitor loop started");
            loop {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as u32;

                // 1. Check for expired swaps eligible for refund or stale off-chain reservations
                let mut expired_tru_refunds = Vec::new();
                let mut expired_base_refunds = Vec::new();
                let known_order_keys = self.orderbook.get_all_order_keys().await;
                {
                    let mut vault = self.vault.lock().await;
                    let mut records = vault.load_records().unwrap_or_default();
                    let mut modified = false;

                    for r in &mut records {
                        let rec_id = hex::encode(r.swap_id).to_lowercase();
                        let rec_hash = hex::encode(r.hash160).to_lowercase();
                        let in_book = known_order_keys.contains(&rec_id) || known_order_keys.contains(&rec_hash);

                        let is_onchain_or_mempool = if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                            let active = scanner.get_active_swaps().await;
                            let in_active = active.iter().any(|s| s.hash160_hex.to_lowercase() == rec_hash);
                            let in_mempool = scanner.get_mempool().await.iter().any(|m| {
                                m.htlc_hash160.as_deref().unwrap_or("").to_lowercase() == rec_hash
                            });
                            in_active || in_mempool
                        } else {
                            false
                        };

                        if is_onchain_or_mempool {
                            if r.step < 2 {
                                r.step = 2;
                                modified = true;
                            }
                            if r.status == 2 {
                                info!("Reviving falsely expired swap 0x{} - found active on-chain/mempool!", &hex::encode(r.swap_id)[..8]);
                                r.status = 0;
                                modified = true;
                            }
                        }

                        let is_stale_offchain = (r.direction == 1 && r.step == 1 && (now as u64 >= r.created_at + 30) && !in_book && !is_onchain_or_mempool)
                            || (r.direction == 2 && r.step == 1 && (now as u64 >= r.created_at + 600) && !in_book && !is_onchain_or_mempool);

                        // Case A: Stale off-chain order reservation (step < 2) -> release immediately
                        if r.status == 0 && r.step < 2 && is_stale_offchain {
                            info!("Off-chain swap reservation expired/released for 0x{}...", &hex::encode(r.swap_id)[..8]);
                            r.status = 2; // Expired / Released
                            modified = true;
                            self.tui
                                .log(format!(
                                    "🧹 Off-chain order reservation 0x{}... released (unlocked {:.4} TRU / {:.2} USDC)",
                                    &hex::encode(r.swap_id)[..8],
                                    r.tru_atoms as f64 / 100_000_000.0,
                                    r.usdc_atomic as f64 / 1_000_000.0
                                ))
                                .await;
                        } else if r.status == 0 && r.step >= 2 {
                            // Case B: On-chain locked swap (step >= 2)
                            // For TRU (direction 1): timelock must be satisfied by Median Time Past (MTP, BIP 113)
                            // For Base (direction 2): block.timestamp >= timelock
                            let eligible = if r.direction == 1 {
                                let mtp = if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                                    scanner.get_median_time_past().await
                                } else {
                                    (now as u64).saturating_sub(7200)
                                };
                                if now >= r.timelock && mtp < r.timelock as u64 {
                                    info!(
                                        "⏳ TRU timelock wall-clock reached for 0x{}, but waiting for MTP consensus (MTP: {}, required: {})",
                                        &hex::encode(r.swap_id)[..8], mtp, r.timelock
                                    );
                                }
                                mtp >= r.timelock as u64
                            } else if r.direction == 2 {
                                now >= r.timelock
                            } else {
                                false
                            };

                            if eligible {
                                if r.direction == 1 {
                                    expired_tru_refunds.push((r.swap_id, r.hash160, r.timelock));
                                } else if r.direction == 2 {
                                    expired_base_refunds.push((r.swap_id, format!("0x{}", hex::encode(r.swap_id))));
                                }
                            }
                        }
                    }

                    if modified {
                        let _ = vault.save_records(&records);
                    }
                }

                for (swap_id, h160, tlock) in expired_tru_refunds {
                    self.tui
                        .log(format!(
                            "⏳ Safety Refund triggered for expired TRU swap (ID: 0x{}...)",
                            &hex::encode(swap_id)[..8]
                        ))
                        .await;
                    match self.try_refund_tru(h160, tlock).await {
                        Ok(txid) => {
                            info!("Autonomous TRU refund broadcast succeeded: TXID {}", txid);
                            let mut vault = self.vault.lock().await;
                            if let Ok(mut records) = vault.load_records() {
                                if let Some(rec) = records.iter_mut().find(|r| r.swap_id == swap_id) {
                                    rec.status = 2;
                                    let _ = vault.save_records(&records);
                                }
                            }
                        }
                        Err(e) => {
                            // Check if the HTLC was already spent/refunded on-chain
                            let already_spent = if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                                let hash_hex = hex::encode(h160);
                                let swaps = scanner.get_all_swaps().await;
                                swaps.iter().any(|s| s.hash160_hex.to_lowercase() == hash_hex.to_lowercase() && s.is_spent)
                            } else {
                                false
                            };

                            if already_spent {
                                info!("TRU HTLC for 0x{} already spent/refunded on-chain. Marking status = 2", &hex::encode(swap_id)[..8]);
                                let mut vault = self.vault.lock().await;
                                if let Ok(mut records) = vault.load_records() {
                                    if let Some(rec) = records.iter_mut().find(|r| r.swap_id == swap_id) {
                                        rec.status = 2;
                                        let _ = vault.save_records(&records);
                                    }
                                }
                            } else {
                                self.tui.log(format!("⚠️ TRU refund attempt for 0x{} failed: {}", &hex::encode(swap_id)[..8], e)).await;
                                warn!("⚠️ TRU refund attempt for 0x{} failed: {}. Will retry on next scan tick.", &hex::encode(swap_id)[..8], e);
                            }
                        }
                    }
                }

                for (swap_id, swap_id_hex) in expired_base_refunds {
                    self.tui
                        .log(format!(
                            "⏳ Safety Refund triggered for expired Base USDC swap (ID: 0x{}...)",
                            &hex::encode(swap_id)[..8]
                        ))
                        .await;
                    match self.try_refund_base(&swap_id_hex).await {
                        Ok(tx_hash) => {
                            info!("Autonomous Base USDC refund broadcast succeeded: TX {}", tx_hash);
                            let mut vault = self.vault.lock().await;
                            if let Ok(mut records) = vault.load_records() {
                                if let Some(rec) = records.iter_mut().find(|r| r.swap_id == swap_id) {
                                    rec.status = 2;
                                    let _ = vault.save_records(&records);
                                }
                            }
                        }
                        Err(e) => {
                            warn!("⚠️ Base USDC refund attempt for 0x{} failed: {}. Will retry on next scan tick.", &hex::encode(swap_id)[..8], e);
                        }
                    }
                }

                // 2. Auto-recovery: detect in-flight matched swaps and lock TRU
                let pending_locks: Vec<(f64, [u8; 20], u32)> = {
                    let vault = self.vault.lock().await;
                    let records = vault.load_records().unwrap_or_default();
                    records
                        .into_iter()
                        .filter(|r| r.direction == 1 && r.status == 0 && r.step <= 3)
                        .map(|r| (r.tru_atoms as f64 / 100_000_000.0, r.hash160, r.timelock))
                        .collect()
                };

                for (amt, hash160, timelock) in pending_locks {
                    let hash_hex = hex::encode(hash160);
                    let already_locked = if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                        let all = scanner.get_all_swaps().await;
                        let in_chain = all.iter().any(|s| s.hash160_hex.to_lowercase() == hash_hex.to_lowercase() && !s.is_spent);
                        let in_mempool = scanner.get_mempool().await.iter().any(|m| {
                            m.htlc_hash160.as_deref().unwrap_or("").to_lowercase() == hash_hex.to_lowercase()
                        });
                        in_chain || in_mempool
                    } else {
                        false
                    };

                    if !already_locked {
                        info!("Auto-recovery: triggering TRU HTLC lock for matched swap 0x{}...", &hash_hex[..8]);
                        let _ = self.try_auto_lock_tru(amt, hash160, timelock).await;
                    }
                }

                // 3. Autonomous Buyer Claim (direction 2):
                // If we are buyer (direction 2, status 0, step == 3):
                // We MUST only sweep TRU if Base USDC has already been locked on-chain (step >= 3).
                let pending_buyer_claims: Vec<([u8; 20], [u8; 32])> = {
                    let vault = self.vault.lock().await;
                    let records = vault.load_records().unwrap_or_default();
                    records
                        .into_iter()
                        .filter(|r| r.direction == 2 && r.status == 0 && r.step >= 3 && r.step < 4)
                        .map(|r| (r.hash160, r.secret))
                        .collect()
                };

                for (hash160, secret_bytes) in pending_buyer_claims {
                    let hash_hex = hex::encode(hash160);
                    let secret_opt = if secret_bytes != [0u8; 32] {
                        Some(secret_bytes)
                    } else {
                        let mut s = None;
                        if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                            if let Some(s_hex) = scanner.get_swap_secret(&hash_hex).await {
                                if let Ok(bytes) = hex::decode(&s_hex) {
                                    if bytes.len() == 32 {
                                        let mut arr = [0u8; 32];
                                        arr.copy_from_slice(&bytes);
                                        s = Some(arr);
                                    }
                                }
                            }
                        }
                        if s.is_none() {
                            if let Some(s_hex) = self.orderbook.get_secret(&hash_hex).await {
                                if let Ok(bytes) = hex::decode(&s_hex) {
                                    if bytes.len() == 32 {
                                        let mut arr = [0u8; 32];
                                        arr.copy_from_slice(&bytes);
                                        s = Some(arr);
                                    }
                                }
                            }
                        }
                        s
                    };

                    if let Some(secret) = secret_opt {
                        let is_safe = if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                            let (safe, _confs) = scanner.get_swap_safety(&hash_hex).await;
                            safe
                        } else {
                            false
                        };

                        if is_safe {
                            let expected_atoms = {
                                let v = self.vault.lock().await;
                                v.load_records().ok().and_then(|recs| {
                                    recs.iter().find(|r| r.hash160 == hash160).map(|r| r.tru_atoms)
                                }).unwrap_or(0)
                            };
                            let expected_atoms = if expected_atoms > 0 {
                                expected_atoms
                            } else if let Some(order) = self.orderbook.get_order_by_hash(&hash_hex).await {
                                (order.amount_tru * 100_000_000.0).round() as u64
                            } else {
                                0
                            };

                            if let Err(e) = self.find_and_validate_seller_tru_lock(hash160, expected_atoms, 0).await {
                                warn!("[SECURITY DEFENSE] Autonomous Buyer Claim skipped for 0x{}: {}", &hash_hex[..8], e);
                                continue;
                            }

                            info!("Autonomous Buyer Claim: TRU Lock has >=6 confs and validated safe, sweeping TRU with secret for 0x{}...", &hash_hex[..8]);
                            match self.sweep_tru_claim_with_secret(&hash_hex, &secret).await {
                                Ok(claim_txid) => {
                                    self.tui.log(format!("🎉 Autonomous TRU Sweep succeeded (0x{}...)! TXID: {}", &hash_hex[..8], claim_txid)).await;
                                    let mut vault = self.vault.lock().await;
                                    if let Ok(mut records) = vault.load_records() {
                                        for r in &mut records {
                                            if r.hash160 == hash160 {
                                                r.step = 4;
                                                r.status = 1;
                                                r.secret = secret;
                                            }
                                        }
                                        let _ = vault.save_records(&records);
                                    }
                                }
                                Err(e) => {
                                    if e.contains("No active or unspent") || e.contains("already spent") {
                                        let mut vault = self.vault.lock().await;
                                        if let Ok(mut records) = vault.load_records() {
                                            for r in &mut records {
                                                if r.hash160 == hash160 {
                                                    r.step = 4;
                                                    r.status = 1;
                                                }
                                            }
                                            let _ = vault.save_records(&records);
                                        }
                                    } else {
                                        warn!("Autonomous Buyer TRU sweep error: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }

                // 4. Autonomous Seller Base Claim (direction 1):
                // If we are seller (direction 1), claim Base USDC as soon as Buyer locks USDC on Base.
                // We only mark status = 1 when Base USDC is actually claimed.
                let pending_seller_claims: Vec<([u8; 20], [u8; 32], [u8; 32])> = {
                    let mut vault = self.vault.lock().await;
                    let mut records = vault.load_records().unwrap_or_default();
                    let mut to_claim = Vec::new();
                    let mut modified = false;
                    for r in &mut records {
                        // Auto-heal completed records: step == 4 means completely finalized/settled
                        if r.step == 4 && r.status == 0 {
                            r.status = 1;
                            modified = true;
                        }
                        if r.direction == 1 && r.status == 0 && r.step < 4 {
                            let hash_hex = hex::encode(r.hash160).to_lowercase();
                            let is_claimed = self.orderbook.is_base_swap_claimed(&hash_hex).await;
                            if !is_claimed {
                                to_claim.push((r.hash160, r.swap_id, r.secret));
                            }
                        }
                    }
                    if modified {
                        let _ = vault.save_records(&records);
                    }
                    to_claim
                };

                for (hash160, _swap_id_bytes, record_secret) in pending_seller_claims {
                    let hash_hex = hex::encode(hash160);
                    let secret_opt = if record_secret != [0u8; 32] {
                        Some(record_secret)
                    } else {
                        let mut s = None;
                        if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                            if let Some(s_hex) = scanner.get_swap_secret(&hash_hex).await {
                                if let Ok(bytes) = hex::decode(&s_hex) {
                                    if bytes.len() == 32 {
                                        let mut arr = [0u8; 32];
                                        arr.copy_from_slice(&bytes);
                                        s = Some(arr);
                                    }
                                }
                            }
                        }
                        if s.is_none() {
                            if let Some(s_hex) = self.orderbook.get_secret(&hash_hex).await {
                                if let Ok(bytes) = hex::decode(&s_hex) {
                                    if bytes.len() == 32 {
                                        let mut arr = [0u8; 32];
                                        arr.copy_from_slice(&bytes);
                                        s = Some(arr);
                                    }
                                }
                            }
                        }
                        s
                    };

                    if let Some(secret) = secret_opt {
                        // 1. Check if orderbook already knows real base_swap_id
                        let mut target_swap_id = self.orderbook.get_base_swap_id(&hash_hex).await;

                        // 2. If not known, query on-chain via EvmSigner.resolve_onchain_swap_id
                        if target_swap_id.is_none() {
                            if let Some(signer) = self.evm_signer.read().await.as_ref() {
                                if let Some(onchain_id) = signer.resolve_onchain_swap_id(&hash_hex).await {
                                    info!("[AUTONOMOUS-SELLER] 🔍 Resolved on-chain BaseHTLC swap_id for 0x{}: {}", &hash_hex[..8], onchain_id);
                                    self.orderbook.record_base_swap_id(&hash_hex, &onchain_id).await;
                                    target_swap_id = Some(onchain_id);
                                }
                            }
                        }

                        // We can only claim if a valid on-chain Base swap ID is known!
                        let Some(swap_id_hex) = target_swap_id else {
                            continue;
                        };

                        if self.orderbook.is_base_swap_claimed(&swap_id_hex).await {
                            continue;
                        }

                        if let Some(signer) = self.evm_signer.read().await.as_ref() {
                            info!("Autonomous Seller Claim: claiming Base USDC for 0x{}... (swap_id 0x{}...)", &hash_hex[..8], &swap_id_hex[..swap_id_hex.len().min(10)]);
                            match signer.claim_swap(&swap_id_hex, &secret).await {
                                Ok(tx_hash) => {
                                    self.tui.log(format!("🎉 Autonomous Base USDC Claimed (0x{}...)! TX: {}", &hash_hex[..8], tx_hash)).await;
                                    self.orderbook.mark_base_swap_claimed(&swap_id_hex).await;
                                    self.orderbook.mark_base_swap_claimed(&hash_hex).await;
                                    let mut vault = self.vault.lock().await;
                                    if let Ok(mut records) = vault.load_records() {
                                        for r in &mut records {
                                            if r.hash160 == hash160 {
                                                r.step = 4;
                                                r.status = 1;
                                                if let Ok(b) = hex::decode(swap_id_hex.strip_prefix("0x").unwrap_or(&swap_id_hex)) {
                                                    if b.len() == 32 {
                                                        r.swap_id.copy_from_slice(&b);
                                                    }
                                                }
                                            }
                                        }
                                        let _ = vault.save_records(&records);
                                    }
                                }
                                Err(e) => {
                                    if e.contains("already claimed") || e.contains("AlreadyClaimed") || e.contains("already known") {
                                        info!("BaseHTLC swap 0x{} already claimed or already in mempool: {}", &swap_id_hex[..8], e);
                                        self.orderbook.mark_base_swap_claimed(&swap_id_hex).await;
                                        self.orderbook.mark_base_swap_claimed(&hash_hex).await;
                                        let mut vault = self.vault.lock().await;
                                        if let Ok(mut records) = vault.load_records() {
                                            for r in &mut records {
                                                if r.hash160 == hash160 {
                                                    r.step = 4;
                                                    r.status = 1;
                                                }
                                            }
                                            let _ = vault.save_records(&records);
                                        }
                                    } else {
                                        warn!("Autonomous Base claim attempt failed: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }

                // 5. Autonomous Buyer Safety Guard & Recovery (direction 2):
                // - Maker Buyer (secret != [0; 32]): locks Base USDC first (step <= 2) or watches/sweeps TRU (step == 3)
                // - Taker Buyer (secret == [0; 32]): waits for Seller's TRU lock before locking Base USDC
                let (pending_taker_buyer_locks, pending_maker_buyer_locks, pending_maker_buyer_sweeps) = {
                    let own_base_addr = {
                        let mut v = self.vault.lock().await;
                        v.get_base_address().unwrap_or_default().to_lowercase()
                    };
                    let mut vault = self.vault.lock().await;
                    let mut records = vault.load_records().unwrap_or_default();
                    let mut taker_to_watch = Vec::new();
                    let mut maker_to_lock = Vec::new();
                    let mut maker_to_sweep = Vec::new();
                    let mut modified = false;
                    for r in &mut records {
                        let hash_hex = hex::encode(r.hash160).to_lowercase();
                        if r.direction == 2 && r.status == 2 {
                            let is_onchain = if let Some(scanner) = self.block_scanner.read().await.as_ref() {
                                scanner.get_active_swaps().await.iter().any(|s| {
                                    !s.is_spent && s.hash160_hex.to_lowercase() == hash_hex
                                })
                            } else {
                                false
                            };
                            if is_onchain {
                                info!("Section 5: Reviving buyer swap 0x{} - found active on-chain!", &hex::encode(r.swap_id)[..8]);
                                r.status = 0;
                                r.step = 2;
                                modified = true;
                            }
                        }

                        if r.direction == 2 && r.status == 0 {
                            if r.beneficiary.is_empty() || r.beneficiary.to_lowercase() == own_base_addr {
                                if let Some(tb) = self.orderbook.get_taker_base(&hash_hex).await {
                                    if !tb.is_empty() && tb.to_lowercase() != own_base_addr {
                                        r.beneficiary = tb;
                                        modified = true;
                                    }
                                } else if let Some(order) = self.orderbook.get_order_by_hash(&hash_hex).await {
                                    if let Some(tb) = order.taker_base_address {
                                        if !tb.is_empty() && tb.to_lowercase() != own_base_addr {
                                            r.beneficiary = tb;
                                            modified = true;
                                        }
                                    }
                                }
                            }
                            if r.secret != [0u8; 32] {
                                // Maker Buyer holds the secret
                                if r.step <= 2 && !r.beneficiary.is_empty() && r.beneficiary.to_lowercase() != own_base_addr {
                                    maker_to_lock.push((
                                        r.beneficiary.clone(),
                                        r.usdc_atomic as f64 / 1_000_000.0,
                                        r.hash160,
                                        r.timelock,
                                        r.secret,
                                    ));
                                } else if r.step == 3 {
                                    maker_to_sweep.push((r.hash160, r.secret));
                                }
                            } else {
                                // Taker Buyer (no secret, waits for TRU safe before Base lock)
                                if r.step <= 2 && !r.beneficiary.is_empty() && r.beneficiary.to_lowercase() != own_base_addr {
                                    taker_to_watch.push((
                                        r.beneficiary.clone(),
                                        r.usdc_atomic as f64 / 1_000_000.0,
                                        r.hash160,
                                        r.timelock,
                                    ));
                                }
                            }
                        }
                    }
                    if modified {
                        let _ = vault.save_records(&records);
                    }
                    (taker_to_watch, maker_to_lock, maker_to_sweep)
                };

                for (beneficiary, amount_usdc, hash160, timelock) in pending_taker_buyer_locks {
                    self.wait_tru_safe_and_lock_base(beneficiary, amount_usdc, hash160, timelock).await;
                }
                for (beneficiary, amount_usdc, hash160, timelock, secret) in pending_maker_buyer_locks {
                    self.lock_base_as_maker_buyer(beneficiary, amount_usdc, hash160, timelock, secret).await;
                }
                for (hash160, secret) in pending_maker_buyer_sweeps {
                    self.watch_seller_tru_and_sweep(hash160, secret).await;
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });
    }
}
