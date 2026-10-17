use crate::tru_p2p::TruP2PNode;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TruSwapOrder {
    pub txid: String,
    pub vout: u32,
    pub amount_tru: f64,
    pub hash160: [u8; 20],
    pub timelock: u32,
    pub claim_pubkey: String,
    pub refund_pubkey: String,
}

/// Pure Rust TRU Lite Client (SPV / LE).
/// Completely eliminates any dependency on local JSON-RPC daemons (trud).
/// All blockchain state, height tracking, and transaction relaying occur
/// directly through the embedded peer-to-peer wire protocol.
pub struct TruLiteClient {
    pub p2p_node: Arc<TruP2PNode>,
    pub vault: Arc<tokio::sync::Mutex<crate::wallet::keystore::EncryptedVault>>,
    local_balance_atoms: AtomicU64,
}

pub type TruNodeClient = TruLiteClient;

impl TruLiteClient {
    pub fn new(
        p2p_node: Arc<TruP2PNode>,
        vault: Arc<tokio::sync::Mutex<crate::wallet::keystore::EncryptedVault>>,
    ) -> Self {
        Self {
            p2p_node,
            vault,
            local_balance_atoms: AtomicU64::new(0),
        }
    }

    /// Gets current block height of TRU blockchain from P2P client.
    pub async fn get_block_count(&self) -> u32 {
        let h = self.p2p_node.current_height();
        if h > 0 {
            h as u32
        } else {
            25_032 // Default recent mainnet checkpoint tip
        }
    }

    /// Broadcasts a raw transaction to the TRU P2P network.
    pub async fn broadcast_raw_tx(&self, txid: &str, raw_hex: &str, pipe_str: Option<&str>) -> Result<(), String> {
        let raw_bytes = hex::decode(raw_hex).map_err(|e| format!("Invalid hex: {}", e))?;

        let wire_pipe_string = match pipe_str {
            Some(s) => s.to_string(),
            None => {
                match crate::core::RawTransaction::deserialize_binary(&raw_bytes) {
                    Ok(parsed_tx) => parsed_tx.to_pipe_string(txid),
                    Err(_) => {
                        if let Ok(s) = std::str::from_utf8(&raw_bytes) {
                            s.to_string()
                        } else {
                            return Err("Failed to convert transaction to canonical wire pipe format".to_string());
                        }
                    }
                }
            }
        };

        info!(
            "[TRU-P2P] Relaying tx {} to network (payload: {} bytes)",
            txid, wire_pipe_string.len()
        );
        self.p2p_node.broadcast_tx(txid, wire_pipe_string.as_bytes()).await?;
        Ok(())
    }

    /// Loads or creates an isolated mainnet address for TRU from the encrypted compressed vault.
    pub async fn get_or_create_swap_address(&self) -> String {
        let mut vault = self.vault.lock().await;
        vault
            .get_or_create_wallet()
            .map(|kp| kp.address)
            .unwrap_or_default()
    }

    /// Gets balance of TRU in atoms (1 TRU = 100_000_000 atoms).
    pub async fn get_balance_atoms(&self) -> u64 {
        self.local_balance_atoms.load(Ordering::Relaxed)
    }

    /// Sets balance in atoms.
    pub fn set_balance_atoms(&self, atoms: u64) {
        self.local_balance_atoms.store(atoms, Ordering::Relaxed);
    }

    /// Withdraws TRU coins to an external address using available vault UTXOs.
    /// Performs exact byte-by-byte fee calculation with +10 atoms buffer.
    pub async fn withdraw_with_utxos(
        &self,
        target_address: &str,
        amount_tru: f64,
        available_utxos: Vec<crate::tru_p2p::scanner::UtxoEntry>,
    ) -> Result<(String, Vec<String>, Option<crate::tru_p2p::scanner::UtxoEntry>), String> {
        let amount_atoms = (amount_tru * 100_000_000.0).round() as u64;
        if amount_atoms < Self::DUST_LIMIT_ATOMS {
            return Err(format!(
                "Withdrawal amount must be at least {} atoms ({:.8} TRU) to satisfy network dust limit",
                Self::DUST_LIMIT_ATOMS,
                Self::DUST_LIMIT_ATOMS as f64 / 100_000_000.0
            ));
        }

        let priv_arr = {
            let mut vault = self.vault.lock().await;
            vault.get_priv_key()?
        };
        let kp = crate::crypto::keypair_from_bytes(&priv_arr)?;

        // Ensure target address is valid TRU P2PKH
        let target_spk = crate::crypto::address_to_script_pubkey(target_address)?;
        let vault_spk = crate::crypto::address_to_script_pubkey(&kp.address)?;

        // Detect if sweep (withdrawing total available balance minus 1-output fee)
        let total_available: u64 = available_utxos.iter().map(|u| u.amount_atoms).sum();
        let sweep_fee_atoms = (crate::crypto::estimate_tx_vsize(available_utxos.len().max(1), 1, 0) as u64) + 10;
        let is_sweep = amount_atoms >= total_available.saturating_sub(sweep_fee_atoms);

        // Select UTXOs
        let mut selected = Vec::new();
        let mut accumulated_atoms = 0u64;
        if is_sweep {
            selected = available_utxos.clone();
            accumulated_atoms = total_available;
        } else {
            for u in available_utxos {
                selected.push(u.clone());
                accumulated_atoms += u.amount_atoms;
                let est_fee = (crate::crypto::estimate_tx_vsize(selected.len(), 2, 0) as u64) + 10;
                if accumulated_atoms >= amount_atoms + est_fee {
                    break;
                }
            }
        }

        let vin_count = selected.len();
        if vin_count == 0 {
            return Err("No spendable UTXOs available in vault".to_string());
        }

        let est_vout_count = if is_sweep { 1 } else { 2 };
        let est_fee_atoms = (crate::crypto::estimate_tx_vsize(vin_count, est_vout_count, 0) as u64) + 10;
        if accumulated_atoms < amount_atoms + est_fee_atoms {
            return Err(format!(
                "Insufficient spendable balance: have {:.8} TRU, needed {:.8} TRU + {:.8} TRU estimated fee",
                accumulated_atoms as f64 / 100_000_000.0,
                amount_tru,
                est_fee_atoms as f64 / 100_000_000.0
            ));
        }

        // Build raw transaction
        let mut tx = crate::crypto::RawTransaction::new();
        for u in &selected {
            tx.inputs.push(crate::crypto::TxInput {
                txid: u.txid.clone(),
                vout: u.vout,
                script_sig: Vec::new(),
                sequence: 0xffffffff,
                pub_key: Vec::new(),
            });
        }

        // Output 0: recipient
        tx.outputs.push(crate::crypto::TxOutput {
            amount_atoms: amount_atoms,
            script_pubkey: target_spk,
        });

        // Output 1: change (if not sweep and above dust limit)
        let trial_change = accumulated_atoms.saturating_sub(amount_atoms + est_fee_atoms);
        if !is_sweep && trial_change >= Self::DUST_LIMIT_ATOMS {
            tx.outputs.push(crate::crypto::TxOutput {
                amount_atoms: trial_change,
                script_pubkey: vault_spk.clone(),
            });
        }

        // Initial sign to determine exact byte size
        for i in 0..vin_count {
            tx.sign_p2pkh_input(i, &vault_spk, &kp.priv_key, &kp.pub_key)?;
        }

        let mut actual_bytes = tx.serialize_binary().len();
        let mut exact_fee_atoms = (actual_bytes as u64) + 10;
        let mut final_change = accumulated_atoms.saturating_sub(amount_atoms + exact_fee_atoms);

        if tx.outputs.len() == 2 {
            if final_change >= Self::DUST_LIMIT_ATOMS {
                tx.outputs[1].amount_atoms = final_change;
            } else {
                tx.outputs.pop();
                final_change = 0;
            }
        } else if !is_sweep && final_change >= Self::DUST_LIMIT_ATOMS {
            tx.outputs.push(crate::crypto::TxOutput {
                amount_atoms: final_change,
                script_pubkey: vault_spk.clone(),
            });
        }

        // Re-sign with exact amounts
        for i in 0..vin_count {
            tx.sign_p2pkh_input(i, &vault_spk, &kp.priv_key, &kp.pub_key)?;
        }

        let new_bytes = tx.serialize_binary().len();
        if new_bytes != actual_bytes {
            actual_bytes = new_bytes;
            exact_fee_atoms = (actual_bytes as u64) + 10;
            final_change = accumulated_atoms.saturating_sub(amount_atoms + exact_fee_atoms);
            if tx.outputs.len() == 2 {
                if final_change >= Self::DUST_LIMIT_ATOMS {
                    tx.outputs[1].amount_atoms = final_change;
                } else {
                    tx.outputs.pop();
                    final_change = 0;
                }
            }
            for i in 0..vin_count {
                tx.sign_p2pkh_input(i, &vault_spk, &kp.priv_key, &kp.pub_key)?;
            }
        }

        let final_raw = tx.serialize_binary();
        let txid = tx.compute_txid();
        let raw_hex = hex::encode(&final_raw);
        let pipe_str = tx.to_pipe_string(&txid);

        info!(
            "[TRU-LITE] Withdrawal transaction signed! TXID: {}, size: {} bytes, fee: {} atoms, is_sweep: {}",
            txid, final_raw.len(), exact_fee_atoms, is_sweep
        );

        // Broadcast to TRU P2P network
        self.broadcast_raw_tx(&txid, &raw_hex, Some(&pipe_str)).await?;

        let spent_outpoints: Vec<String> = selected
            .iter()
            .map(|u| format!("{}:{}", u.txid, u.vout))
            .collect();

        let change_entry = if tx.outputs.len() == 2 && final_change >= Self::DUST_LIMIT_ATOMS {
            Some(crate::tru_p2p::scanner::UtxoEntry {
                txid: txid.clone(),
                vout: 1,
                amount_atoms: final_change,
                amount_tru: final_change as f64 / 100_000_000.0,
                block_height: 0,
                script_pubkey: hex::encode(&vault_spk),
            })
        } else {
            None
        };

        Ok((txid, spent_outpoints, change_entry))
    }

    /// Convenience wrapper for withdrawal
    pub async fn withdraw(&self, target_address: &str, amount_tru: f64) -> Result<String, String> {
        self.withdraw_with_utxos(target_address, amount_tru, Vec::new()).await.map(|(txid, _, _)| txid)
    }

    pub const HTLC_CLAIM_FEE_SURPLUS_ATOMS: u64 = 250;
    pub const DUST_LIMIT_ATOMS: u64 = 546;

    /// Locks TRU funds into the canonical 103-byte HTLC script on the TRU network.
    pub async fn lock_htlc_with_utxos(
        &self,
        amount_tru: f64,
        hash160: &[u8; 20],
        claim_pubkey: &[u8; 33],
        timelock: u32,
        available_utxos: Vec<crate::tru_p2p::scanner::UtxoEntry>,
    ) -> Result<(String, u32, Vec<String>, Option<crate::tru_p2p::scanner::UtxoEntry>), String> {
        let deal_atoms = (amount_tru * 100_000_000.0).round() as u64;
        if deal_atoms < Self::DUST_LIMIT_ATOMS {
            return Err(format!(
                "HTLC lock amount must be at least {} atoms ({:.8} TRU) to satisfy network dust limit",
                Self::DUST_LIMIT_ATOMS,
                Self::DUST_LIMIT_ATOMS as f64 / 100_000_000.0
            ));
        }

        let priv_arr = {
            let mut vault = self.vault.lock().await;
            vault.get_priv_key()?
        };
        let kp = crate::crypto::keypair_from_bytes(&priv_arr)?;
        let vault_spk = crate::crypto::address_to_script_pubkey(&kp.address)?;

        // Build canonical 103-byte HTLC script with refund_pubkey = vault public key
        let htlc_script = Self::build_canonical_htlc_script(hash160, claim_pubkey, &kp.pub_key, timelock);

        // OP_RETURN metadata: TRU_CONTRACT:HTLC_Atomic_Swap_V1 (32 bytes)
        let op_return_spk = hex::decode("6a205452555f434f4e54524143543a48544c435f41746f6d69635f537761705f5631")
            .map_err(|e| e.to_string())?;

        // Select UTXOs
        let mut selected = Vec::new();
        let mut accumulated_atoms = 0u64;
        let target_atoms = deal_atoms + Self::HTLC_CLAIM_FEE_SURPLUS_ATOMS;
        for u in available_utxos {
            selected.push(u.clone());
            accumulated_atoms += u.amount_atoms;
            let est_fee = (crate::crypto::estimate_tx_vsize(selected.len(), 3, 0) as u64) + 10;
            if accumulated_atoms >= target_atoms + est_fee {
                break;
            }
        }

        let vin_count = selected.len();
        if vin_count == 0 {
            return Err("No spendable UTXOs available in vault".to_string());
        }

        let min_2out_fee = (crate::crypto::estimate_tx_vsize(vin_count, 2, 0) as u64) + 10;
        if accumulated_atoms < deal_atoms + min_2out_fee {
            return Err(format!(
                "Insufficient spendable balance: have {:.8} TRU, needed {:.8} TRU + {:.8} TRU network fee",
                accumulated_atoms as f64 / 100_000_000.0,
                amount_tru,
                min_2out_fee as f64 / 100_000_000.0
            ));
        }

        let est_3out_fee = (crate::crypto::estimate_tx_vsize(vin_count, 3, 0) as u64) + 10;
        let (lock_atoms, _est_fee_atoms, trial_change) = if accumulated_atoms >= deal_atoms + Self::HTLC_CLAIM_FEE_SURPLUS_ATOMS + est_3out_fee + Self::DUST_LIMIT_ATOMS {
            let surplus = Self::HTLC_CLAIM_FEE_SURPLUS_ATOMS;
            let change = accumulated_atoms - (deal_atoms + surplus + est_3out_fee);
            (deal_atoms + surplus, est_3out_fee, change)
        } else {
            // Funds are tight or change would be dust: 2 outputs (no change output needed), surplus dynamically capped to whatever is available
            let available_surplus = accumulated_atoms.saturating_sub(deal_atoms + min_2out_fee);
            let surplus = available_surplus.min(Self::HTLC_CLAIM_FEE_SURPLUS_ATOMS);
            (deal_atoms + surplus, min_2out_fee, 0)
        };

        // Build raw transaction
        let mut tx = crate::crypto::RawTransaction::new();
        for u in &selected {
            tx.inputs.push(crate::crypto::TxInput {
                txid: u.txid.clone(),
                vout: u.vout,
                script_sig: Vec::new(),
                sequence: 0xffffffff,
                pub_key: Vec::new(),
            });
        }

        // Output 0: OP_RETURN metadata
        tx.outputs.push(crate::crypto::TxOutput {
            amount_atoms: 0,
            script_pubkey: op_return_spk,
        });

        // Output 1: HTLC Lock (103-byte script) with surplus fee included
        tx.outputs.push(crate::crypto::TxOutput {
            amount_atoms: lock_atoms,
            script_pubkey: htlc_script.clone(),
        });

        // Output 2: Change (only if above dust limit)
        if trial_change >= Self::DUST_LIMIT_ATOMS {
            tx.outputs.push(crate::crypto::TxOutput {
                amount_atoms: trial_change,
                script_pubkey: vault_spk.clone(),
            });
        }

        // Sign inputs
        for i in 0..vin_count {
            tx.sign_p2pkh_input(i, &vault_spk, &kp.priv_key, &kp.pub_key)?;
        }

        let actual_bytes = tx.serialize_binary().len();
        let exact_fee_atoms = (actual_bytes as u64) + 10;
        let mut final_change = accumulated_atoms.saturating_sub(lock_atoms + exact_fee_atoms);
        if tx.outputs.len() == 3 {
            if final_change >= Self::DUST_LIMIT_ATOMS {
                tx.outputs[2].amount_atoms = final_change;
            } else {
                tx.outputs.pop();
                final_change = 0;
            }
        }
        for i in 0..vin_count {
            tx.sign_p2pkh_input(i, &vault_spk, &kp.priv_key, &kp.pub_key)?;
        }

        let new_bytes = tx.serialize_binary().len();
        if new_bytes != actual_bytes {
            for i in 0..vin_count {
                tx.sign_p2pkh_input(i, &vault_spk, &kp.priv_key, &kp.pub_key)?;
            }
        }

        let final_raw = tx.serialize_binary();
        let txid = tx.compute_txid();
        let raw_hex = hex::encode(&final_raw);
        let pipe_str = tx.to_pipe_string(&txid);

        info!(
            "[TRU-HTLC] HTLC lock transaction signed! TXID: {}, deal: {:.4} TRU (locked: {} atoms, fee: {} atoms)",
            txid, amount_tru, lock_atoms, exact_fee_atoms
        );

        self.broadcast_raw_tx(&txid, &raw_hex, Some(&pipe_str)).await?;

        let spent_outpoints: Vec<String> = selected
            .iter()
            .map(|u| format!("{}:{}", u.txid, u.vout))
            .collect();

        let change_entry = if tx.outputs.len() == 3 && final_change >= Self::DUST_LIMIT_ATOMS {
            Some(crate::tru_p2p::scanner::UtxoEntry {
                txid: txid.clone(),
                vout: 2,
                amount_atoms: final_change,
                amount_tru: final_change as f64 / 100_000_000.0,
                block_height: 0,
                script_pubkey: hex::encode(&vault_spk),
            })
        } else {
            None
        };

        Ok((txid, 1, spent_outpoints, change_entry))
    }

    /// Claims locked funds from an on-chain HTLC output on TRU using the revealed secret preimage.
    pub async fn claim_htlc(
        &self,
        htlc_txid: &str,
        htlc_vout: u32,
        htlc_atoms: u64,
        htlc_script: &[u8],
        secret: &[u8],
        claimer_priv_key: &[u8; 32],
        destination_address: &str,
    ) -> Result<String, String> {
        let target_spk = crate::crypto::address_to_script_pubkey(destination_address)?;

        let mut tx = crate::crypto::RawTransaction::new();
        tx.inputs.push(crate::crypto::TxInput {
            txid: htlc_txid.to_string(),
            vout: htlc_vout,
            script_sig: Vec::new(),
            sequence: 0xfffffffd, // BIP 125 Opt-In RBF enabled (< 0xffffffff - 1)
            pub_key: Vec::new(),
        });

        // Dynamic fee: 1 in / 1 out ~ 196 bytes, with max of exact estimate and configured surplus
        let est_fee = crate::core::fee::calculate_exact_fee_atoms(1, 1, 0);
        let fee_atoms = est_fee.max(Self::HTLC_CLAIM_FEE_SURPLUS_ATOMS);
        let claim_atoms = htlc_atoms.saturating_sub(fee_atoms);

        tx.outputs.push(crate::crypto::TxOutput {
            amount_atoms: claim_atoms,
            script_pubkey: target_spk,
        });

        // SIGHASH_ALL on the htlc_script
        let sighash = tx.get_sighash(0, htlc_script);
        let signing_key = k256::ecdsa::SigningKey::from_bytes(claimer_priv_key.into())
            .map_err(|e| format!("Invalid claimer key: {}", e))?;

        use k256::ecdsa::signature::hazmat::PrehashSigner;
        let (sig, _): (k256::ecdsa::Signature, _) = signing_key
            .sign_prehash(&sighash)
            .map_err(|e| format!("Claim signing failed: {}", e))?;
        let der = sig.normalize_s().to_der();

        // script_sig: <sig + 0x01> <secret> OP_TRUE
        let mut script_sig = Vec::with_capacity(1 + der.as_bytes().len() + 1 + 1 + secret.len() + 1);
        script_sig.push((der.as_bytes().len() + 1) as u8);
        script_sig.extend_from_slice(der.as_bytes());
        script_sig.push(0x01); // SIGHASH_ALL

        script_sig.push(secret.len() as u8);
        script_sig.extend_from_slice(secret);

        script_sig.push(0x51); // OP_TRUE (0x51)

        tx.inputs[0].script_sig = script_sig;

        let final_raw = tx.serialize_binary();
        let txid = tx.compute_txid();
        let raw_hex = hex::encode(&final_raw);
        let pipe_str = tx.to_pipe_string(&txid);

        info!(
            "[TRU-HTLC] HTLC claim transaction signed! TXID: {}, claimed: {:.4} TRU (fee: {} atoms, RBF: enabled)",
            txid, claim_atoms as f64 / 100_000_000.0, fee_atoms
        );

        self.broadcast_raw_tx(&txid, &raw_hex, Some(&pipe_str)).await?;

        Ok(txid)
    }

    /// Refunds expired TRU HTLC coins back to local vault address using OP_CHECKLOCKTIMEVERIFY branch.
    pub async fn refund_htlc(
        &self,
        htlc_txid: &str,
        htlc_vout: u32,
        htlc_atoms: u64,
        htlc_script: &[u8],
        timelock: u32,
        refunder_priv_key: &[u8; 32],
        destination_address: &str,
    ) -> Result<String, String> {
        let target_spk = crate::crypto::address_to_script_pubkey(destination_address)?;

        let mut tx = crate::crypto::RawTransaction::new();
        tx.lock_time = timelock; // Must satisfy OP_CHECKLOCKTIMEVERIFY
        tx.inputs.push(crate::crypto::TxInput {
            txid: htlc_txid.to_string(),
            vout: htlc_vout,
            script_sig: Vec::new(),
            sequence: 0xfffffffd, // Satisfies OP_CHECKLOCKTIMEVERIFY (< 0xffffffff) AND BIP 125 Opt-In RBF (< 0xffffffff - 1)
            pub_key: Vec::new(),
        });

        let est_fee = crate::core::fee::calculate_exact_fee_atoms(1, 1, 0);
        let fee_atoms = est_fee.max(Self::HTLC_CLAIM_FEE_SURPLUS_ATOMS);
        let refund_atoms = htlc_atoms.saturating_sub(fee_atoms);

        tx.outputs.push(crate::crypto::TxOutput {
            amount_atoms: refund_atoms,
            script_pubkey: target_spk,
        });

        let sighash = tx.get_sighash(0, htlc_script);
        let signing_key = k256::ecdsa::SigningKey::from_bytes(refunder_priv_key.into())
            .map_err(|e| format!("Invalid refunder key: {}", e))?;

        use k256::ecdsa::signature::hazmat::PrehashSigner;
        let (sig, _): (k256::ecdsa::Signature, _) = signing_key
            .sign_prehash(&sighash)
            .map_err(|e| format!("Refund signing failed: {}", e))?;
        let der = sig.normalize_s().to_der();

        // script_sig: <refund_sig + 0x01> OP_FALSE (0x00 branches to OP_ELSE)
        let mut script_sig = Vec::with_capacity(1 + der.as_bytes().len() + 1 + 1);
        script_sig.push((der.as_bytes().len() + 1) as u8);
        script_sig.extend_from_slice(der.as_bytes());
        script_sig.push(0x01); // SIGHASH_ALL
        script_sig.push(0x00); // OP_FALSE

        tx.inputs[0].script_sig = script_sig;

        let final_raw = tx.serialize_binary();
        let txid = tx.compute_txid();
        let raw_hex = hex::encode(&final_raw);
        let pipe_str = tx.to_pipe_string(&txid);

        info!(
            "[TRU-HTLC] HTLC refund transaction signed! TXID: {}, refunded: {:.4} TRU",
            txid, refund_atoms as f64 / 100_000_000.0
        );

        self.broadcast_raw_tx(&txid, &raw_hex, Some(&pipe_str)).await?;

        Ok(txid)
    }

    /// Gets public Base EVM checksummed address from local vault.
    pub async fn get_base_address(&self) -> String {
        let mut vault = self.vault.lock().await;
        vault.get_base_address().unwrap_or_default()
    }

    /// Gets raw 32-byte Base EVM private key from local vault.
    pub async fn get_base_priv_key(&self) -> Result<[u8; 32], String> {
        let mut vault = self.vault.lock().await;
        vault.get_base_priv_key()
    }

    /// Sets or imports a specific Base EVM private key into the local vault.
    pub async fn set_base_priv_key(&self, priv_key_hex: &str) -> Result<String, String> {
        let mut vault = self.vault.lock().await;
        vault.set_base_priv_key(priv_key_hex)
    }

    /// Sets or imports a specific 12-word Base BIP-39 mnemonic phrase into the local vault.
    pub async fn set_base_mnemonic(&self, phrase: &str) -> Result<String, String> {
        let mut vault = self.vault.lock().await;
        vault.set_base_mnemonic(phrase)
    }

    /// Regenerates a fresh independent Base EVM wallet in the local vault.
    pub async fn regenerate_base_wallet(&self) -> Result<String, String> {
        let mut vault = self.vault.lock().await;
        vault.regenerate_base_wallet()
    }

    /// Constructs the canonical 103-byte TRU HTLC script.
    /// Format:
    /// OP_IF OP_HASH160 <hash160> OP_EQUALVERIFY <claim_pub> OP_CHECKSIG OP_ELSE <timelock> OP_CHECKLOCKTIMEVERIFY OP_DROP <refund_pub> OP_CHECKSIG OP_ENDIF
    pub fn build_canonical_htlc_script(
        hash160: &[u8; 20],
        claim_pubkey: &[u8; 33],
        refund_pubkey: &[u8; 33],
        timelock: u32,
    ) -> Vec<u8> {
        let mut script = Vec::with_capacity(103);
        script.push(0x63); // OP_IF
        script.push(0xa9); // OP_HASH160
        script.push(0x14); // PUSH 20 bytes
        script.extend_from_slice(hash160);
        script.push(0x88); // OP_EQUALVERIFY
        script.push(0x21); // PUSH 33 bytes (compressed pubkey)
        script.extend_from_slice(claim_pubkey);
        script.push(0xac); // OP_CHECKSIG
        script.push(0x67); // OP_ELSE
        script.push(0x04); // PUSH 4 bytes (timelock LE)
        script.extend_from_slice(&timelock.to_le_bytes());
        script.push(0xb1); // OP_CHECKLOCKTIMEVERIFY
        script.push(0x75); // OP_DROP
        script.push(0x21); // PUSH 33 bytes (compressed refund pubkey)
        script.extend_from_slice(refund_pubkey);
        script.push(0xac); // OP_CHECKSIG
        script.push(0x68); // OP_ENDIF
        script
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_script_size() {
        let hash160 = [0xabu8; 20];
        let claim_pubkey = [0x02u8; 33];
        let refund_pubkey = [0x03u8; 33];
        let timelock = 1750000000;

        let script = TruLiteClient::build_canonical_htlc_script(
            &hash160,
            &claim_pubkey,
            &refund_pubkey,
            timelock,
        );
        assert_eq!(script.len(), 103, "TRU HTLC script must be exactly 103 bytes");
    }
}
