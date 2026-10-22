use axum::{
    extract::{Json, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::core::fee::estimate_tx_vsize;
use crate::tru_p2p::scanner::SwapStatus;

use super::routes::{AppState, FeeEstimateQuery, WithdrawRequest};

pub async fn build_swaps_ledger_json(state: &AppState) -> serde_json::Value {
    let all = state.block_scanner.get_all_swaps().await;
    let scanned = state.block_scanner.scanned_height();
    let verified = state.block_scanner.verified_height();
    let target = state.block_scanner.target_height();
    let is_scanning = state.block_scanner.is_scanning();
    let tip_hash = state.block_scanner.last_verified_hash().await;
    let violations = state.block_scanner.integrity_violations();
    let duration = state.block_scanner.sync_duration_secs().await;
    let bps = state.block_scanner.sync_blocks_per_sec().await;
    let elapsed = state.block_scanner.elapsed_secs().await;
    let active_count = all.iter().filter(|s| (s.status == SwapStatus::Active || s.status == SwapStatus::ExpiringSoon) && !s.is_spent).count();

    let vault_records = {
        let vault = state.vault.lock().await;
        vault.load_records().unwrap_or_default()
    };
    let base_swap_map = state.orderbook.get_all_base_swap_ids().await;
    let base_lock_tx_map = state.orderbook.get_all_base_lock_txs().await;
    let tru_lock_tx_map = state.orderbook.get_all_tru_lock_txs().await;
    let base_claimed_set = state.orderbook.get_all_base_claimed().await;
    let taker_base_map = state.orderbook.get_all_taker_base_addresses().await;
    let secrets_map = state.orderbook.get_all_secrets().await;
    let base_orders = state.orderbook.get_active_orders().await;

    let mut enriched_swaps: Vec<serde_json::Value> = all
        .iter()
        .map(|s| {
            let mut val = serde_json::to_value(s).unwrap_or_default();
            let clean_hash = s.hash160_hex.to_lowercase();

            let local_rec = vault_records.iter().find(|r| hex::encode(r.hash160).to_lowercase() == clean_hash);

            let base_ord = base_orders.iter().find(|o| o.hash160_hex.to_lowercase() == clean_hash);
            let base_swap_id = base_swap_map.get(&clean_hash).cloned();
            let base_lock_tx = base_lock_tx_map.get(&clean_hash).cloned().or_else(|| {
                base_swap_id.as_ref().and_then(|id| {
                    let c = id.strip_prefix("0x").unwrap_or(id).to_lowercase();
                    base_lock_tx_map.get(&c).cloned().or_else(|| base_lock_tx_map.get(id).cloned())
                })
            });
            let base_usdc = base_ord.map(|o| o.amount_usdc);
            let taker_base = taker_base_map.get(&clean_hash).cloned()
                .or_else(|| base_ord.and_then(|o| o.taker_base_address.clone()));

            let is_base_claimed = base_claimed_set.contains(&clean_hash)
                || base_claimed_set.contains(&format!("0x{}", clean_hash))
                || base_swap_id.as_ref().map(|id| {
                    let c = id.strip_prefix("0x").unwrap_or(id).to_lowercase();
                    base_claimed_set.contains(&c) || base_claimed_set.contains(id)
                }).unwrap_or(false);

            if let Some(obj) = val.as_object_mut() {
                obj.insert("base_claimed".to_string(), serde_json::json!(is_base_claimed));
                if let Some(bs_id) = base_swap_id {
                    obj.insert("base_swap_id".to_string(), serde_json::json!(bs_id));
                }
                if let Some(bl_tx) = base_lock_tx {
                    obj.insert("base_lock_tx".to_string(), serde_json::json!(bl_tx));
                }
                if s.txid.is_empty() {
                    if let Some(t_tx) = tru_lock_tx_map.get(&clean_hash) {
                        obj.insert("txid".to_string(), serde_json::json!(t_tx));
                    }
                }
                if let Some(usdc) = base_usdc {
                    obj.insert("amount_usdc".to_string(), serde_json::json!(usdc));
                }
                if let Some(ref tb) = taker_base {
                    obj.insert("taker_base".to_string(), serde_json::json!(tb));
                }
                if let Some(rec) = local_rec {
                    let is_active_mine = (rec.status == 0 || (!s.is_spent && (s.confirmations > 0 || s.block_height > 0))) && !s.is_spent && !is_base_claimed;
                    obj.insert("is_mine".to_string(), serde_json::json!(is_active_mine || rec.status == 1));
                    obj.insert("vault_status".to_string(), serde_json::json!(rec.status));
                    obj.insert("vault_step".to_string(), serde_json::json!(rec.step));
                    obj.insert("direction".to_string(), serde_json::json!(rec.direction));

                    let rec_has_secret = rec.secret != [0u8; 32];
                    let onchain_revealed = if s.is_spent {
                        secrets_map.get(&clean_hash).cloned().or_else(|| s.secret_hex.clone())
                    } else {
                        None
                    };
                    let has_sec = rec_has_secret || onchain_revealed.is_some();
                    obj.insert("has_secret".to_string(), serde_json::json!(has_sec));
                    if let Some(sec) = onchain_revealed {
                        obj.insert("secret".to_string(), serde_json::json!(sec));
                    }

                    if rec.status == 1 || is_base_claimed {
                        if s.is_spent {
                            obj.insert("status".to_string(), serde_json::json!("Claimed"));
                        }
                    }
                    if !rec.beneficiary.is_empty() && !obj.contains_key("taker_base") {
                        obj.insert("taker_base".to_string(), serde_json::json!(rec.beneficiary));
                    }
                    if !obj.contains_key("amount_usdc") || obj.get("amount_usdc").and_then(|v| v.as_f64()).unwrap_or(0.0) == 0.0 {
                        obj.insert("amount_usdc".to_string(), serde_json::json!(rec.usdc_atomic as f64 / 1_000_000.0));
                    }
                } else {
                    let known_secret = secrets_map.get(&clean_hash).cloned().or_else(|| s.secret_hex.clone());
                    let has_sec = known_secret.is_some();
                    let sec_str = known_secret.unwrap_or_default();
                    obj.insert("is_mine".to_string(), serde_json::json!(false));
                    obj.insert("has_secret".to_string(), serde_json::json!(has_sec));
                    obj.insert("secret".to_string(), serde_json::json!(sec_str));
                }
            }
            val
        })
        .collect();

    // Include local active pending vault records that haven't been detected on-chain yet
    let now_sec = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for rec in &vault_records {
        let clean_hash = hex::encode(rec.hash160).to_lowercase();
        let has_tru_lock = tru_lock_tx_map.contains_key(&clean_hash);
        let is_active_taker_buyer = rec.direction == 2 && !rec.beneficiary.is_empty();
        if rec.status == 0 && (rec.step >= 2 || has_tru_lock || is_active_taker_buyer) {
            let already_included = enriched_swaps.iter().any(|s| {
                s.get("hash160_hex")
                    .and_then(|h| h.as_str())
                    .map(|h| h.trim_start_matches("0x").to_lowercase() == clean_hash)
                    .unwrap_or(false)
            });

            if !already_included {
                let base_swap_id = base_swap_map.get(&clean_hash).cloned();

                let base_lock_tx = base_lock_tx_map.get(&clean_hash).cloned().or_else(|| {
                    base_swap_id.as_ref().and_then(|id| {
                        let c = id.strip_prefix("0x").unwrap_or(id).to_lowercase();
                        base_lock_tx_map.get(&c).cloned().or_else(|| base_lock_tx_map.get(id).cloned())
                    })
                });
                let tru_tx = tru_lock_tx_map.get(&clean_hash).cloned().unwrap_or_default();

                let remaining_hours = if rec.timelock > now_sec as u32 {
                    (rec.timelock - now_sec as u32) as f64 / 3600.0
                } else {
                    0.0
                };

                let syn_swap = serde_json::json!({
                    "txid": tru_tx,
                    "vout": 0,
                    "block_height": 0,
                    "confirmations": 0,
                    "status": "Active",
                    "hash160_hex": clean_hash,
                    "amount_tru": rec.tru_atoms as f64 / 100_000_000.0,
                    "amount_usdc": rec.usdc_atomic as f64 / 1_000_000.0,
                    "timelock": rec.timelock,
                    "is_mine": true,
                    "is_safe": false,
                    "is_spent": false,
                    "has_secret": rec.secret != [0u8; 32],
                    "vault_status": rec.status,
                    "vault_step": rec.step,
                    "direction": rec.direction,
                    "base_claimed": false,
                    "base_swap_id": base_swap_id,
                    "base_lock_tx": base_lock_tx,
                    "taker_base": rec.beneficiary,
                    "hours_remaining": remaining_hours
                });
                enriched_swaps.push(syn_swap);
            }
        }
    }

    // Include 0-conf real-time HTLC swaps observed in the live P2P mempool
    let mempool_entries = state.block_scanner.get_mempool().await;
    for mem_tx in &mempool_entries {
        if mem_tx.is_htlc {
            if let Some(ref h160) = mem_tx.htlc_hash160 {
                let clean_h = h160.trim_start_matches("0x").to_lowercase();
                let already_included = enriched_swaps.iter().any(|s| {
                    s.get("hash160_hex")
                        .and_then(|h| h.as_str())
                        .map(|h| h.trim_start_matches("0x").to_lowercase() == clean_h)
                        .unwrap_or(false)
                });
                if !already_included {
                    let local_rec = vault_records.iter().find(|r| hex::encode(r.hash160).to_lowercase() == clean_h);
                    let is_mine = local_rec.is_some() || mem_tx.is_vault_related;
                    let base_swap_id = base_swap_map.get(&clean_h).cloned();
                    let base_lock_tx = base_lock_tx_map.get(&clean_h).cloned().or_else(|| {
                        base_swap_id.as_ref().and_then(|id| {
                            let c = id.strip_prefix("0x").unwrap_or(id).to_lowercase();
                            base_lock_tx_map.get(&c).cloned().or_else(|| base_lock_tx_map.get(id).cloned())
                        })
                    });
                    let base_ord = base_orders.iter().find(|o| o.hash160_hex.to_lowercase() == clean_h);
                    let usdc_amt = base_ord.map(|o| o.amount_usdc)
                        .or_else(|| local_rec.map(|r| r.usdc_atomic as f64 / 1_000_000.0))
                        .unwrap_or(mem_tx.target_amount_tru);
                    let taker_base = local_rec.map(|r| r.beneficiary.clone())
                        .filter(|b| !b.is_empty())
                        .or_else(|| taker_base_map.get(&clean_h).cloned())
                        .or_else(|| base_ord.and_then(|o| o.taker_base_address.clone()))
                        .unwrap_or_default();
                    let remaining_hours = mem_tx.htlc_timelock.map(|tl| {
                        if tl > now_sec as u32 {
                            (tl - now_sec as u32) as f64 / 3600.0
                        } else {
                            0.0
                        }
                    }).unwrap_or(4.0);

                    let rec_has_sec = local_rec.map(|r| r.secret != [0u8; 32]).unwrap_or(false);
                    let rec_sec = local_rec.map(|r| hex::encode(r.secret)).unwrap_or_default();
                    let direction = local_rec.map(|r| r.direction).unwrap_or(1);
                    let vault_step = local_rec.map(|r| r.step).unwrap_or(2);

                    let syn_swap = serde_json::json!({
                        "txid": mem_tx.txid,
                        "vout": 0,
                        "block_height": 0,
                        "confirmations": 0,
                        "status": "Active",
                        "hash160_hex": clean_h,
                        "amount_tru": mem_tx.target_amount_tru,
                        "amount_usdc": usdc_amt,
                        "timelock": mem_tx.htlc_timelock.unwrap_or(0),
                        "is_mine": is_mine,
                        "is_safe": false,
                        "is_spent": false,
                        "has_secret": rec_has_sec,
                        "vault_status": 0,
                        "vault_step": vault_step,
                        "direction": direction,
                        "secret": rec_sec,
                        "base_claimed": false,
                        "base_swap_id": base_swap_id,
                        "base_lock_tx": base_lock_tx,
                        "taker_base": taker_base,
                        "hours_remaining": remaining_hours
                    });
                    enriched_swaps.push(syn_swap);
                }
            }
        }
    }

    serde_json::json!({
        "scanned_height": scanned,
        "verified_height": verified,
        "target_height": target,
        "is_scanning": is_scanning,
        "tip_hash": tip_hash,
        "integrity_violations": violations,
        "chain_integrity_ok": violations == 0,
        "sync_duration_secs": duration,
        "sync_blocks_per_sec": bps,
        "elapsed_secs": elapsed,
        "total_swaps": all.len(),
        "active_swaps": active_count,
        "swaps": enriched_swaps,
    })
}

pub async fn get_swaps_ledger(State(state): State<AppState>) -> impl IntoResponse {
    axum::Json(build_swaps_ledger_json(&state).await)
}

pub async fn fetch_raw_vault_status(state: &AppState) -> crate::api::codec::RawVaultStatus {
    let address = if let Some(addr) = state.block_scanner.vault_address().await {
        addr
    } else {
        state.tru_client.get_or_create_swap_address().await
    };
    let utxos = state.block_scanner.get_vault_utxos().await;
    let current_height = state.block_scanner.verified_height();

    let mut confirmed_atoms = 0u64;
    let mut unconfirmed_atoms = 0u64;
    let mut mempool_count = 0u32;
    let mut raw_utxos = Vec::with_capacity(utxos.len());

    for u in utxos {
        let confs = if u.block_height > 0 && current_height >= u.block_height as u64 {
            current_height - u.block_height as u64 + 1
        } else {
            0
        };
        if confs > 0 {
            confirmed_atoms += u.amount_atoms;
        } else {
            unconfirmed_atoms += u.amount_atoms;
            mempool_count += 1;
        }

        let mut txid_raw = [0u8; 32];
        if let Ok(b) = hex::decode(&u.txid) {
            if b.len() == 32 {
                txid_raw.copy_from_slice(&b);
            }
        }

        raw_utxos.push(crate::api::codec::RawUtxo {
            txid: txid_raw,
            vout: u.vout,
            amount_atoms: u.amount_atoms,
            height: u.block_height as u64,
        });
    }

    crate::api::codec::RawVaultStatus {
        address,
        confirmed_atoms,
        unconfirmed_atoms,
        mempool_count,
        utxos: raw_utxos,
    }
}

pub async fn build_vault_status_json(state: &AppState) -> serde_json::Value {
    let raw = fetch_raw_vault_status(state).await;
    let total_atoms = raw.confirmed_atoms + raw.unconfirmed_atoms;
    let (kp_pubkey, _) = {
        let mut vault = state.vault.lock().await;
        if let Ok(priv_key) = vault.get_priv_key() {
            if let Ok(kp) = crate::core::keys::keypair_from_bytes(&priv_key) {
                (hex::encode(kp.pub_key), kp.address)
            } else {
                (String::new(), raw.address.clone())
            }
        } else {
            (String::new(), raw.address.clone())
        }
    };
    let utxos_json: Vec<_> = raw.utxos.iter().map(|u| {
        let confs = if u.height > 0 && state.block_scanner.verified_height() >= u.height {
            state.block_scanner.verified_height() - u.height + 1
        } else {
            0
        };
        serde_json::json!({
            "txid": hex::encode(u.txid),
            "vout": u.vout,
            "amount_atoms": u.amount_atoms,
            "amount_tru": u.amount_atoms as f64 / 100_000_000.0,
            "block_height": u.height,
            "confirmations": confs,
            "is_mempool": confs == 0,
        })
    }).collect();

    let (base_address, base_eth_balance, base_usdc_balance) = if let Some(signer) = state.swap_engine.evm_signer.read().await.as_ref() {
        let eth = signer.get_eth_balance(None).await.unwrap_or(0.0);
        let usdc = signer.get_usdc_balance(None).await.unwrap_or(0.0);
        (signer.address().to_string(), eth, usdc)
    } else {
        let mut vault = state.vault.lock().await;
        let addr = vault.get_base_address().unwrap_or_default();
        (addr, 0.0, 0.0)
    };

    serde_json::json!({
        "address": raw.address,
        "pubkey": kp_pubkey,
        "balance_atoms": total_atoms,
        "balance_tru": total_atoms as f64 / 100_000_000.0,
        "confirmed_atoms": raw.confirmed_atoms,
        "confirmed_tru": raw.confirmed_atoms as f64 / 100_000_000.0,
        "unconfirmed_atoms": raw.unconfirmed_atoms,
        "unconfirmed_tru": raw.unconfirmed_atoms as f64 / 100_000_000.0,
        "base_address": base_address,
        "base_eth_balance": base_eth_balance,
        "base_usdc_balance": base_usdc_balance,
        "is_testnet": state.config.is_testnet,
        "chain_id": state.config.chain_id().as_u64(),
        "network_name": state.config.network_name(),
        "explorer_url": state.config.explorer_url(),
        "base_htlc_contract": state.config.base_htlc_contract,
        "base_usdc_contract": state.config.base_usdc_contract,
        "relay_node_url": state.config.relay_node_url.clone(),
        "utxo_count": utxos_json.len(),
        "mempool_count": raw.mempool_count,
        "utxos": utxos_json,
    })
}

pub async fn get_vault_status(State(state): State<AppState>) -> impl IntoResponse {
    axum::Json(build_vault_status_json(&state).await)
}

pub async fn get_mempool(State(state): State<AppState>) -> impl IntoResponse {
    let txs = state.block_scanner.get_mempool().await;
    axum::Json(serde_json::json!({
        "success": true,
        "count": txs.len(),
        "transactions": txs,
    }))
}

pub async fn get_estimate_fee(
    State(state): State<AppState>,
    Query(query): Query<FeeEstimateQuery>,
) -> impl IntoResponse {
    let amount_tru = query.amount_tru.unwrap_or(1.0);
    let amount_atoms = (amount_tru * 100_000_000.0).round() as u64;

    let available_utxos = state.block_scanner.get_vault_utxos().await;
    let total_available: u64 = available_utxos.iter().map(|u| u.amount_atoms).sum();

    let mut selected_count = 0usize;
    let mut accumulated = 0u64;

    for u in &available_utxos {
        selected_count += 1;
        accumulated += u.amount_atoms;
        let est_fee = (estimate_tx_vsize(selected_count, 2, 0) as u64) + 10;
        if accumulated >= amount_atoms + est_fee {
            break;
        }
    }

    if selected_count == 0 {
        selected_count = 1;
    }

    let is_sweep = amount_atoms >= total_available.saturating_sub((estimate_tx_vsize(available_utxos.len().max(1), 1, 0) as u64) + 10);
    let vout_count = if is_sweep { 1 } else { 2 };
    let vin_count = if is_sweep { available_utxos.len().max(1) } else { selected_count };

    let tx_bytes = estimate_tx_vsize(vin_count, vout_count, 0);
    let fee_atoms = (tx_bytes as u64) + 10;
    let fee_tru = fee_atoms as f64 / 100_000_000.0;
    let max_sendable_atoms = total_available.saturating_sub((estimate_tx_vsize(available_utxos.len().max(1), 1, 0) as u64) + 10);

    axum::Json(serde_json::json!({
        "amount_tru": amount_tru,
        "amount_atoms": amount_atoms,
        "tx_bytes": tx_bytes,
        "rate_atoms_per_byte": 1,
        "safety_buffer_atoms": 10,
        "fee_atoms": fee_atoms,
        "fee_tru": fee_tru,
        "vin_count": vin_count,
        "vout_count": vout_count,
        "vault_balance_atoms": total_available,
        "max_sendable_atoms": max_sendable_atoms,
        "max_sendable_tru": max_sendable_atoms as f64 / 100_000_000.0,
        "breakdown": format!("{} bytes * 1 atom/byte + 10 atoms buffer = {} atoms ({:.8} TRU)", tx_bytes, fee_atoms, fee_tru),
    }))
}

pub async fn execute_vault_withdraw(
    state: &AppState,
    dest: &str,
    amount_tru: f64,
) -> Result<String, String> {
    let dest = dest.trim();
    if dest.is_empty() {
        return Err("Destination address cannot be empty".to_string());
    }

    if amount_tru <= 0.0 {
        return Err("Withdrawal amount must be greater than 0".to_string());
    }

    // 🛡️ Serialize all vault UTXO spending (prevents double-spend race condition across concurrent withdrawals and auto-locks)
    let _tru_guard = state.swap_engine.tru_lock_mutex.lock().await;

    let utxos = state.block_scanner.get_vault_utxos().await;
    let current_height = state.block_scanner.verified_height();

    let mut spendable_utxos = Vec::new();
    let mut unconfirmed_change_atoms = 0u64;

    for u in utxos {
        let is_confirmed = u.block_height > 0 && current_height >= u.block_height as u64;
        if is_confirmed {
            spendable_utxos.push(u);
        } else {
            unconfirmed_change_atoms += u.amount_atoms;
        }
    }

    let spendable_atoms: u64 = spendable_utxos.iter().map(|u| u.amount_atoms).sum();
    let requested_atoms = (amount_tru * 100_000_000.0).round() as u64;

    if spendable_atoms < requested_atoms {
        if unconfirmed_change_atoms > 0 {
            let unconf_tru = unconfirmed_change_atoms as f64 / 100_000_000.0;
            return Err(format!(
                "Change of {:.8} TRU is awaiting 1 block confirmation on TRU network (~15-30s). Please wait for next block before withdrawing.",
                unconf_tru
            ));
        } else {
            let avail_tru = spendable_atoms as f64 / 100_000_000.0;
            return Err(format!(
                "Insufficient confirmed balance: have {:.8} TRU, requested {:.8} TRU",
                avail_tru, amount_tru
            ));
        }
    }

    let is_sweep = requested_atoms >= spendable_atoms.saturating_sub((estimate_tx_vsize(spendable_utxos.len().max(1), 1, 0) as u64) + 10);
    let needed_vouts = if is_sweep { 1 } else { 2 };

    let mut trial_selected = 0usize;
    let mut trial_accum = 0u64;
    for u in &spendable_utxos {
        trial_selected += 1;
        trial_accum += u.amount_atoms;
        let est_fee = (estimate_tx_vsize(trial_selected, needed_vouts, 0) as u64) + 10;
        if trial_accum >= requested_atoms + est_fee {
            break;
        }
    }
    let exact_est_fee = (estimate_tx_vsize(trial_selected, needed_vouts, 0) as u64) + 10;

    if spendable_atoms < requested_atoms + exact_est_fee {
        return Err(format!(
            "Balance too low to pay network transaction fee: have {:.8} TRU, needed {:.8} TRU + {:.8} TRU fee ({:.8} TRU short)",
            spendable_atoms as f64 / 100_000_000.0,
            amount_tru,
            exact_est_fee as f64 / 100_000_000.0,
            ((requested_atoms + exact_est_fee).saturating_sub(spendable_atoms)) as f64 / 100_000_000.0
        ));
    }

    match state.tru_client.withdraw_with_utxos(dest, amount_tru, spendable_utxos).await {
        Ok((txid, spent_outpoints, change_entry)) => {
            state.block_scanner.mark_spent_outpoints(&spent_outpoints).await;
            if let Some(change) = change_entry {
                state.block_scanner.add_vault_mempool_utxo(change).await;
            }

            state
                .tui
                .log(format!(
                    "⚡ Vault Withdrawal: {:.8} TRU to {} (TXID: {}) [{} outpoints spent]",
                    amount_tru, dest, txid, spent_outpoints.len()
                ))
                .await;
            Ok(txid)
        }
        Err(e) => Err(e),
    }
}

pub async fn post_vault_withdraw(
    State(state): State<AppState>,
    Json(payload): Json<WithdrawRequest>,
) -> impl IntoResponse {
    match execute_vault_withdraw(&state, &payload.destination, payload.amount_tru).await {
        Ok(txid) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "success": true,
                "txid": txid,
                "amount_tru": payload.amount_tru,
                "destination": payload.destination,
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "success": false,
                "error": e
            })),
        ),
    }
}

pub async fn get_vault_export(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let mut vault = state.vault.lock().await;
    match vault.export_wallet() {
        Ok(export) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "status": "ok",
                "export": export
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({
                "status": "error",
                "message": e
            })),
        ),
    }
}

pub async fn post_lock_htlc(
    State(state): State<AppState>,
    Json(payload): Json<super::routes::LockHtlcRequest>,
) -> impl IntoResponse {
    let mut hash160 = [0u8; 20];
    if let Ok(b) = hex::decode(&payload.hash160_hex) {
        if b.len() == 20 {
            hash160.copy_from_slice(&b);
        } else {
            return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "success": false, "error": "Invalid hash160 length" })));
        }
    } else {
        return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "success": false, "error": "Invalid hash160 hex" })));
    }

    let resolved_pk_str = if let Some(resolved) = state.orderbook.resolve_pubkey(&payload.claim_pubkey_hex).await {
        resolved
    } else {
        let clean_h = payload.hash160_hex.strip_prefix("0x").unwrap_or(&payload.hash160_hex).to_lowercase();
        let mut found = None;
        for o in state.orderbook.get_active_orders().await {
            if o.hash160_hex.eq_ignore_ascii_case(&clean_h) {
                if let Some(pk) = o.maker_tru_pubkey.as_deref() {
                    if let Some(r) = state.orderbook.resolve_pubkey(pk).await {
                        found = Some(r);
                        break;
                    }
                }
                if let Some(pk) = o.taker_tru_pubkey.as_deref() {
                    if let Some(r) = state.orderbook.resolve_pubkey(pk).await {
                        found = Some(r);
                        break;
                    }
                }
            }
        }
        found.unwrap_or_else(|| payload.claim_pubkey_hex.clone())
    };

    let mut claim_pubkey = [0u8; 33];
    if let Ok(b) = hex::decode(&resolved_pk_str) {
        if b.len() == 33 {
            claim_pubkey.copy_from_slice(&b);
        } else {
            return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "success": false, "error": "Invalid claim_pubkey length" })));
        }
    } else {
        return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "success": false, "error": "Invalid claim_pubkey hex" })));
    }

    // 🛡️ Serialize all vault UTXO spending (prevents double-spend race condition across concurrent locks and withdrawals)
    let _tru_guard = state.swap_engine.tru_lock_mutex.lock().await;

    let utxos = state.block_scanner.get_vault_utxos().await;
    let current_height = state.block_scanner.verified_height();
    let spendable_utxos: Vec<_> = utxos
        .into_iter()
        .filter(|u| u.block_height > 0 && current_height >= u.block_height as u64)
        .collect();

    match state.tru_client.lock_htlc_with_utxos(
        payload.amount_tru,
        &hash160,
        &claim_pubkey,
        payload.timelock,
        spendable_utxos,
    ).await {
        Ok((txid, vout, spent_outpoints, change_entry)) => {
            state.block_scanner.mark_spent_outpoints(&spent_outpoints).await;
            if let Some(change) = change_entry {
                state.block_scanner.add_vault_mempool_utxo(change).await;
            }
            state.tui.log(format!("⚡ Vault HTLC Locked: {:.4} TRU (TXID: {})", payload.amount_tru, txid)).await;
            (StatusCode::OK, axum::Json(serde_json::json!({
                "success": true,
                "txid": txid,
                "vout": vout,
                "amount_tru": payload.amount_tru
            })))
        }
        Err(e) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({
            "success": false,
            "error": e
        }))),
    }
}

pub async fn post_claim_htlc(
    State(state): State<AppState>,
    Json(payload): Json<super::routes::ClaimHtlcRequest>,
) -> impl IntoResponse {
    let htlc_script = match hex::decode(&payload.htlc_script_hex) {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "success": false, "error": "Invalid htlc_script hex" }))),
    };
    let secret = match hex::decode(&payload.secret_hex) {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "success": false, "error": "Invalid secret hex" }))),
    };
    let claimer_priv = match hex::decode(&payload.claimer_priv_key_hex) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "success": false, "error": "Invalid claimer_priv_key" }))),
    };

    match state.tru_client.claim_htlc(
        &payload.htlc_txid,
        payload.htlc_vout,
        payload.htlc_atoms,
        &htlc_script,
        &secret,
        &claimer_priv,
        &payload.destination_address,
    ).await {
        Ok(txid) => {
            state.tui.log(format!("⚡ HTLC Claim Broadcast: TXID {}", txid)).await;
            (StatusCode::OK, axum::Json(serde_json::json!({
                "success": true,
                "txid": txid
            })))
        }
        Err(e) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({
            "success": false,
            "error": e
        }))),
    }
}

pub async fn get_orderbook(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let orders = state.orderbook.get_active_orders().await;
    axum::Json(orders)
}

pub async fn execute_create_order(
    state: &AppState,
    payload: super::routes::CreateOrderRequest,
) -> Result<serde_json::Value, String> {
    let tru_atoms = (payload.amount_tru * 100_000_000.0).round() as u64;
    let usdc_atomic = (payload.amount_usdc * 1_000_000.0).round() as u64;
    let tl = payload.timelock_hours.unwrap_or(4).max(2);

    let side_str = payload.side.as_deref().unwrap_or("SELL_TRU");
    let is_buy = side_str.eq_ignore_ascii_case("BUY_TRU")
        || side_str.eq_ignore_ascii_case("BuyTruWithBaseUsdc");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let existing_records = {
        let mut vault_lock = state.vault.lock().await;
        let mut records = vault_lock.load_records().unwrap_or_default();
        let mut modified = false;
        for r in &mut records {
            // Auto-heal completed records: step == 4 means completely finalized/settled
            if r.step == 4 && r.status == 0 {
                r.status = 1; // Completed / Settled
                modified = true;
            }
            if r.step == 1 && r.status == 0 {
                let rec_id = hex::encode(r.swap_id).to_lowercase();
                let rec_hash = hex::encode(r.hash160).to_lowercase();
                let grace_passed = now >= r.created_at + 25;
                let in_book = state.orderbook.get_order_by_hash(&rec_id).await.is_some()
                    || state.orderbook.get_order_by_hash(&rec_hash).await.is_some();
                // Orders in the orderbook remain live as long as maker client is online and order is in the book.
                // Only prune reservations if the order was removed or cancelled from the book after grace period.
                if grace_passed && !in_book {
                    tracing::info!(
                        "[BALANCE-GUARD] Automatically pruned unlisted maker reservation 0x{} (in_book={})",
                        &rec_id[..rec_id.len().min(8)],
                        in_book
                    );
                    r.status = 2; // Cancelled / Pruned
                    modified = true;
                }
            }
        }
        if modified {
            let _ = vault_lock.save_records(&records);
        }
        records
    };

    if !is_buy {
        let utxos = state.block_scanner.get_vault_utxos().await;
        let verified_h = state.block_scanner.verified_height();
        let spendable: u64 = utxos
            .iter()
            .filter(|u| u.block_height > 0 && verified_h >= u.block_height as u64)
            .map(|u| u.amount_atoms)
            .sum();

        let required_fee_buffer = 1000u64; // safe buffer for multi-input fee + claim surplus
        // Cumulative check: sum already allocated TRU in existing open sell orders resting in orderbook.
        // Only step == 1 represents off-chain reserved TRU. Orders at step >= 2 have already locked
        // their UTXOs on-chain into HTLC outputs, which are already excluded from vault spendable UTXOs.
        let allocated_tru_atoms: u64 = existing_records
            .iter()
            .filter(|r| r.direction == 1 && r.status == 0 && r.step == 1) // Direction 1 = Sell TRU, status 0 = Pending, step 1 = open in orderbook
            .map(|r| r.tru_atoms.saturating_add(required_fee_buffer))
            .sum();

        let total_required = allocated_tru_atoms.saturating_add(tru_atoms).saturating_add(required_fee_buffer);
        if total_required > spendable {
            let have_tru = spendable as f64 / 100_000_000.0;
            let alloc_tru = allocated_tru_atoms as f64 / 100_000_000.0;
            let max_new_atoms = spendable.saturating_sub(allocated_tru_atoms).saturating_sub(required_fee_buffer);
            let max_new_tru = max_new_atoms as f64 / 100_000_000.0;
            return Err(format!(
                "Insufficient confirmed TRU balance: total confirmed is {:.8} TRU, but {:.8} TRU is already allocated in active sell orders. Maximum new sell order is {:.8} TRU",
                have_tru, alloc_tru, max_new_tru
            ));
        }

        // Pre-relay Balance Guard: Maker Sell TRU also requires Base ETH for claiming USDC after swap
        if let Some(signer) = state.swap_engine.evm_signer.read().await.as_ref() {
            if let Ok(eth_bal) = signer.get_eth_balance(None).await {
                // Dynamically estimate gas required for BaseHTLC.claim on Base Sepolia
                let min_eth_fee = signer
                    .estimate_claim_swap_fee()
                    .await
                    .map(|(eth, _)| eth)
                    .unwrap_or(0.00002); // Safe fallback if RPC oracle query fails
                if eth_bal < min_eth_fee {
                    return Err(format!(
                        "Insufficient Base ETH balance for gas: have {:.6} ETH, estimated required is {:.6} ETH (dynamic gas fee to claim USDC on Base after swap)",
                        eth_bal, min_eth_fee
                    ));
                }
            }
        }
    } else {
        // Pre-relay Balance Guard: Maker Buy TRU requires cumulative USDC and ETH gas buffer.
        // Only step == 1 represents off-chain reserved USDC waiting in orderbook.
        let allocated_usdc: f64 = existing_records
            .iter()
            .filter(|r| r.direction == 2 && r.status == 0 && r.step == 1) // Direction 2 = Buy TRU (locking USDC), status 0 = Pending, step 1 = open maker
            .map(|r| r.usdc_atomic as f64 / 1_000_000.0)
            .sum();

        if let Some(signer) = state.swap_engine.evm_signer.read().await.as_ref() {
            if let Ok(bal) = signer.get_usdc_balance(None).await {
                let total_usdc_needed = allocated_usdc + payload.amount_usdc;
                if total_usdc_needed > bal {
                    let max_avail = (bal - allocated_usdc).max(0.0);
                    return Err(format!(
                        "Insufficient Base USDC balance: have {:.2} USDC, but {:.2} USDC is already allocated in active buy orders. Maximum new buy order is {:.2} USDC",
                        bal, allocated_usdc, max_avail
                    ));
                }
            }
            if let Ok(eth_bal) = signer.get_eth_balance(None).await {
                // Dynamically estimate gas required for BaseHTLC.createSwap on Base Sepolia
                let min_eth_fee = signer
                    .estimate_create_swap_fee()
                    .await
                    .map(|(eth, _)| eth)
                    .unwrap_or(0.00003); // Safe fallback if RPC oracle query fails
                if eth_bal < min_eth_fee {
                    return Err(format!(
                        "Insufficient Base ETH balance for gas: have {:.6} ETH, estimated required is {:.6} ETH (dynamic gas fee to lock USDC on Base)",
                        eth_bal, min_eth_fee
                    ));
                }
            }
        }
    }

    let result = if is_buy {
        state
            .swap_engine
            .initiate_buy_offer(tru_atoms, usdc_atomic, tl, payload.maker_base_address.clone())
            .await
    } else {
        state
            .swap_engine
            .initiate_tru_offer(tru_atoms, usdc_atomic, tl, payload.maker_base_address.clone())
            .await
    };

    match result {
        Ok(rec) => {
            let (maker_tru_pubkey, maker_tru_addr, maker_base_addr, tru_priv_key, base_priv_key) = {
                let mut vault_lock = state.vault.lock().await;
                let t_key = vault_lock.get_priv_key().ok();
                let (pk_hex, addr) = t_key.as_ref().and_then(|pk| {
                    crate::core::keys::keypair_from_bytes(pk).ok().map(|kp| (hex::encode(kp.pub_key), kp.address))
                }).unwrap_or_default();
                let b_addr = vault_lock.get_base_address().unwrap_or_default();
                let b_key = vault_lock.get_base_priv_key().ok();
                (pk_hex, addr, b_addr, t_key, b_key)
            };

            let order_id = hex::encode(rec.swap_id);
            let side_enum = if is_buy {
                crate::relay::protocol::RelayOrderSide::BuyTru
            } else {
                crate::relay::protocol::RelayOrderSide::SellTru
            };
            let hash160_hex = hex::encode(rec.hash160);
            let effective_base_addr = payload.maker_base_address.clone().unwrap_or_else(|| maker_base_addr.clone());

            let sig_tru = tru_priv_key.as_ref().and_then(|tpk| {
                let canon_tru = crate::relay::crypto::canonical_order_message(
                    &order_id,
                    side_enum.as_str(),
                    payload.amount_tru,
                    payload.amount_usdc,
                    &hash160_hex,
                    rec.timelock,
                    &maker_tru_addr,
                );
                crate::relay::crypto::sign_tru_message(&canon_tru, tpk).ok()
            });

            let sig_base = base_priv_key.as_ref().and_then(|bpk| {
                let canonical_msg = crate::relay::crypto::canonical_order_message(
                    &order_id,
                    side_enum.as_str(),
                    payload.amount_tru,
                    payload.amount_usdc,
                    &hash160_hex,
                    rec.timelock,
                    &effective_base_addr,
                );
                crate::relay::crypto::sign_personal_message(&canonical_msg, bpk).ok()
            }).unwrap_or_default();

            if sig_tru.is_some() || !sig_base.is_empty() {
                let signed_payload = crate::relay::protocol::SignedOrderPayload {
                    order_id: order_id.clone(),
                    side: side_enum,
                    amount_tru: payload.amount_tru,
                    amount_usdc: payload.amount_usdc,
                    hash160_hex,
                    timelock: rec.timelock,
                    maker_tru_address: maker_tru_addr.clone(),
                    maker_tru_pubkey: if maker_tru_pubkey.is_empty() { None } else { Some(maker_tru_pubkey.clone()) },
                    maker_base_address: effective_base_addr,
                    signature_base: sig_base,
                    signature_tru: sig_tru,
                    created_at: Some(rec.created_at),
                };

                // Send via persistent multiplex connection and await verified ACK (8s safety timeout)
                match state.relay_client.submit_order_with_ack(signed_payload, std::time::Duration::from_secs(8)).await {
                        Ok(()) => {
                            tracing::info!("[RELAY-DISPATCH] Order {} confirmed and registered by Relay mesh", order_id);
                        }
                        Err(e) => {
                            tracing::warn!("[RELAY-DISPATCH] Order {} rejected by Relay mesh: {}", order_id, e);
                            // Rollback newly inserted order from vault & local orderbook to avoid dead phantom records
                            {
                                let mut vault_lock = state.vault.lock().await;
                                if let Ok(mut records) = vault_lock.load_records() {
                                    records.retain(|r| r.swap_id != rec.swap_id);
                                    let _ = vault_lock.save_records(&records);
                                }
                            }
                            state.orderbook.cancel_order(&order_id).await;
                            return Err(format!("Order rejected by Relay mesh: {}", e));
                        }
                    }
            } else {
                return Err("Failed to sign order: No private keys (TRU or Base) found in vault".to_string());
            }

            let _ = state.ui_events.send(serde_json::json!({
                "type": "order_added",
                "data": {
                    "id": hex::encode(rec.swap_id),
                    "order_id": hex::encode(rec.swap_id),
                    "side": if is_buy { "BUY_TRU" } else { "SELL_TRU" },
                    "amount_tru": payload.amount_tru,
                    "amount_usdc": payload.amount_usdc,
                    "hash160_hex": hex::encode(rec.hash160),
                    "timelock": rec.timelock,
                    "maker_base_address": payload.maker_base_address.clone().unwrap_or_else(|| maker_base_addr.clone()),
                    "maker_tru_address": maker_tru_addr.clone(),
                    "maker_tru_pubkey": if maker_tru_pubkey.is_empty() { None } else { Some(maker_tru_pubkey.clone()) },
                    "is_matched": false,
                    "is_active": true,
                }
            }));

            Ok(serde_json::json!({
                "success": true,
                "swap_id": hex::encode(rec.swap_id),
                "hash160": hex::encode(rec.hash160),
                "timelock": rec.timelock,
                "amount_tru": payload.amount_tru,
                "amount_usdc": payload.amount_usdc,
                "maker_tru_pubkey": if maker_tru_pubkey.is_empty() { None } else { Some(maker_tru_pubkey) },
            }))
        }
        Err(e) => Err(e),
    }
}

pub async fn post_create_order(
    State(state): State<AppState>,
    Json(payload): Json<super::routes::CreateOrderRequest>,
) -> impl IntoResponse {
    match execute_create_order(&state, payload).await {
        Ok(v) => (StatusCode::OK, axum::Json(v)),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "success": false, "error": e })),
        ),
    }
}

pub async fn post_import_order(
    State(state): State<AppState>,
    Json(order): Json<crate::swap::orderbook::UnifiedOrder>,
) -> impl IntoResponse {
    state.orderbook.upsert_unified_order(order).await;
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "success": true })),
    )
}

pub async fn execute_cancel_order(
    state: &AppState,
    id_val: &str,
    hash_val: &str,
) -> Result<serde_json::Value, String> {
    let clean_id = id_val.strip_prefix("0x").unwrap_or(id_val).to_lowercase();
    let clean_hash = hash_val.strip_prefix("0x").unwrap_or(hash_val).to_lowercase();

    // 1. Cancel in local memory orderbook
    let mut cancelled = false;
    if !clean_id.is_empty() {
        cancelled = state.orderbook.cancel_order(&clean_id).await || cancelled;
    }
    if !clean_hash.is_empty() {
        cancelled = state.orderbook.cancel_order(&clean_hash).await || cancelled;
    }

    // 2. Look up secret from local vault records
    let mut secret_hex = None;
    let mut matched_hash160 = None;
    let mut matched_order_id = None;
    {
        let mut vault_lock = state.vault.lock().await;
        if let Ok(mut records) = vault_lock.load_records() {
            let mut modified = false;
            for r in &mut records {
                let rec_id = hex::encode(r.swap_id).to_lowercase();
                let rec_hash = hex::encode(r.hash160).to_lowercase();
                let matches = (!clean_id.is_empty() && (rec_id == clean_id || rec_hash == clean_id))
                    || (!clean_hash.is_empty() && (rec_id == clean_hash || rec_hash == clean_hash));
                if matches {
                    secret_hex = Some(hex::encode(r.secret));
                    matched_hash160 = Some(rec_hash);
                    matched_order_id = Some(rec_id);
                    r.status = 2;
                    modified = true;
                    break;
                }
            }
            if modified {
                let _ = vault_lock.save_records(&records);
            }
        }
    }

    let r_order_id = matched_order_id.clone().unwrap_or_else(|| clean_id.clone());
    let (r_tru_addr, r_tru_priv, r_base_addr, r_base_priv) = {
        let mut v = state.vault.lock().await;
        let t_priv = v.get_priv_key().ok();
        let t_addr = t_priv.as_ref().and_then(|pk| {
            crate::core::keys::keypair_from_bytes(pk).ok().map(|kp| kp.address)
        }).unwrap_or_default();
        let b_addr = v.get_base_address().unwrap_or_default();
        let b_priv = v.get_base_priv_key().ok();
        (t_addr, t_priv, b_addr, b_priv)
    };

    let mut sig_tru = None;
    if let Some(ref tpk) = r_tru_priv {
        let canon = crate::relay::crypto::canonical_cancel_message(&r_order_id, &r_tru_addr);
        if let Ok(sig) = crate::relay::crypto::sign_tru_message(&canon, tpk) {
            sig_tru = Some(sig);
        }
    }

    let mut sig_base = String::new();
    if let Some(ref bpk) = r_base_priv {
        let canon_b = crate::relay::crypto::canonical_cancel_message(&r_order_id, &r_base_addr);
        if let Ok(sig) = crate::relay::crypto::sign_personal_message(&canon_b, bpk) {
            sig_base = format!("0x{}", sig);
        }
    }

    let cancel_payload = crate::relay::protocol::SignedCancelPayload {
        order_id: r_order_id.clone(),
        maker_base_address: r_base_addr,
        signature_base: sig_base,
        maker_tru_address: if r_tru_addr.is_empty() { None } else { Some(r_tru_addr) },
        signature_tru: sig_tru,
    };

    // Dispatch cancel via existing persistent multiplex WebSocket
    let cancel_res = state.relay_client.cancel_order(cancel_payload, std::time::Duration::from_secs(5)).await;
    if let Err(ref e) = cancel_res {
        tracing::warn!("[CANCEL] Relay cancel returned warning/error: {}", e);
    }

    state
        .tui
        .log(format!(
            "Order cancelled: {} (local_success: {}, has_secret: {})",
            clean_id,
            cancelled,
            secret_hex.is_some()
        ))
        .await;

    let cancel_id = matched_order_id.clone().unwrap_or_else(|| clean_id.clone());
    let _ = state.ui_events.send(serde_json::json!({
        "type": "order_removed",
        "data": {
            "order_id": cancel_id,
            "hash160": matched_hash160,
        }
    }));

    Ok(serde_json::json!({
        "success": cancelled || secret_hex.is_some(),
        "id": matched_order_id.unwrap_or_else(|| clean_id.clone()),
        "hash160": matched_hash160,
    }))
}

pub async fn post_cancel_order(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id_val = payload.get("id").and_then(|v| v.as_str()).unwrap_or_default();
    let hash_val = payload.get("hash160").and_then(|v| v.as_str()).unwrap_or_default();
    match execute_cancel_order(&state, id_val, hash_val).await {
        Ok(v) => (StatusCode::OK, axum::Json(v)),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "success": false, "error": e })),
        ),
    }
}

pub async fn post_match_order(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let order_id = payload
        .get("order_id")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("id").and_then(|v| v.as_str()))
        .unwrap_or_default();
    let hash160 = payload
        .get("hash160")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let taker_tru = payload
        .get("taker_tru")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("taker_tru_address").and_then(|v| v.as_str()))
        .unwrap_or_default();
    let taker_tru_pubkey = payload
        .get("taker_tru_pubkey")
        .and_then(|v| v.as_str());
    let taker_base = payload
        .get("taker_base")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("taker_base_address").and_then(|v| v.as_str()))
        .unwrap_or_default();
    let signature_base = payload
        .get("signature_base")
        .and_then(|v| v.as_str());

    if !taker_tru.is_empty() || !taker_base.is_empty() {
        if !order_id.is_empty() {
            state.swap_engine.on_order_matched(order_id, taker_tru, taker_tru_pubkey, taker_base, signature_base).await;
        }
        if !hash160.is_empty() {
            state.swap_engine.on_order_matched(hash160, taker_tru, taker_tru_pubkey, taker_base, signature_base).await;
        }
        let target_str = if !hash160.is_empty() { hash160 } else { order_id };
        let short_id = if target_str.len() >= 8 { &target_str[..8] } else { target_str };
        state
            .tui
            .log(format!("Order match registered: Taker details set for 0x{}...", short_id))
            .await;
        let _ = state.ui_events.send(serde_json::json!({
            "type": "order_matched",
            "data": {
                "order_id": order_id,
                "hash160": hash160,
                "taker_base": taker_base,
                "taker_tru": taker_tru,
            }
        }));
        (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "success": true })),
        )
    } else {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "success": false, "error": "Missing taker details" })),
        )
    }
}

pub async fn execute_claim_vault_htlc(
    state: &AppState,
    hash160: &str,
    secret_hex: Option<&str>,
) -> Result<serde_json::Value, String> {
    let clean_hash = hash160.strip_prefix("0x").unwrap_or(hash160).to_lowercase();

    let payload_secret = secret_hex
        .and_then(|s| hex::decode(s.strip_prefix("0x").unwrap_or(s)).ok())
        .filter(|b| b.len() == 32)
        .map(|b| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        });

    // Look up secret from vault records
    let secret_opt = if let Some(s) = payload_secret {
        Some(s)
    } else {
        let vault = state.vault.lock().await;
        let records = vault.load_records().unwrap_or_default();
        records.into_iter().find(|r| hex::encode(r.hash160).to_lowercase() == clean_hash && r.secret != [0u8; 32]).map(|r| r.secret)
    };

    let secret = match secret_opt {
        Some(s) => s,
        None => return Err("Secret not found in vault for this hash160".to_string()),
    };

    // If payload provided the secret, update the vault record with the secret
    {
        let mut vault = state.vault.lock().await;
        if let Ok(mut records) = vault.load_records() {
            let mut modified = false;
            for r in &mut records {
                if hex::encode(r.hash160).to_lowercase() == clean_hash && r.secret == [0u8; 32] {
                    r.secret = secret;
                    modified = true;
                }
            }
            if modified {
                let _ = vault.save_records(&records);
            }
        }
    }

    match state.swap_engine.sweep_tru_claim_with_secret(&clean_hash, &secret).await {
        Ok(txid) => Ok(serde_json::json!({ "success": true, "txid": txid })),
        Err(e) => Err(e),
    }
}

pub async fn post_claim_vault_htlc(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let hash160 = payload.get("hash160").and_then(|v| v.as_str()).unwrap_or_default();
    let secret_hex = payload.get("secret").and_then(|v| v.as_str());
    match execute_claim_vault_htlc(&state, hash160, secret_hex).await {
        Ok(v) => (StatusCode::OK, axum::Json(v)),
        Err(e) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "success": false, "error": e }))),
    }
}

#[derive(serde::Deserialize)]
pub struct SecretQuery {
    pub hash160: Option<String>,
    pub swap_id: Option<String>,
}

pub async fn get_swap_secret(
    State(state): State<AppState>,
    Query(query): Query<SecretQuery>,
) -> impl IntoResponse {
    let hash_param = query
        .hash160
        .as_deref()
        .unwrap_or("")
        .trim();
    let clean_hash = hash_param
        .strip_prefix("0x")
        .unwrap_or(hash_param)
        .to_lowercase();

    let swap_param = query
        .swap_id
        .as_deref()
        .unwrap_or("")
        .trim();
    let clean_swap_id = swap_param
        .strip_prefix("0x")
        .unwrap_or(swap_param)
        .to_lowercase();

    let vault = state.vault.lock().await;
    let records = vault.load_records().unwrap_or_default();
    drop(vault);

    for r in records {
        let r_hash = hex::encode(r.hash160).to_lowercase();
        let r_id = hex::encode(r.swap_id).to_lowercase();
        if (!clean_hash.is_empty() && r_hash == clean_hash)
            || (!clean_swap_id.is_empty() && r_id == clean_swap_id)
        {
            return (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "success": true,
                    "secret": hex::encode(r.secret),
                    "hash160": r_hash,
                    "swap_id": r_id,
                    "step": r.step,
                    "status": r.status,
                    "direction": r.direction,
                    "amount_tru": r.tru_atoms as f64 / 100_000_000.0,
                    "amount_usdc": r.usdc_atomic as f64 / 1_000_000.0,
                })),
            );
        }
    }

    // 2. Check Orderbook & BlockScanner for revealed secrets from on-chain claims
    let query_hash = if !clean_hash.is_empty() {
        clean_hash.clone()
    } else {
        let all_swaps = state.orderbook.get_all_base_swap_ids().await;
        all_swaps.iter().find(|(_, id)| id.strip_prefix("0x").unwrap_or(id).to_lowercase() == clean_swap_id)
            .map(|(h, _)| h.clone())
            .unwrap_or_default()
    };

    if !query_hash.is_empty() {
        if let Some(sec) = state.orderbook.get_secret(&query_hash).await {
            let base_id = state.orderbook.get_base_swap_id(&query_hash).await.unwrap_or_default();
            return (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "success": true,
                    "secret": sec,
                    "hash160": query_hash,
                    "swap_id": base_id,
                })),
            );
        }

        if let Some(sec) = state.block_scanner.get_swap_secret(&query_hash).await {
            let base_id = state.orderbook.get_base_swap_id(&query_hash).await.unwrap_or_default();
            return (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "success": true,
                    "secret": sec,
                    "hash160": query_hash,
                    "swap_id": base_id,
                })),
            );
        }
    }

    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "success": false,
            "error": "Swap record or revealed preimage secret not found"
        })),
    )
}

#[derive(Deserialize)]
pub struct MarkClaimedRequest {
    pub swap_id: Option<String>,
    pub hash160: Option<String>,
}

pub async fn mark_swap_claimed(
    State(state): State<AppState>,
    Json(payload): Json<MarkClaimedRequest>,
) -> impl IntoResponse {
    if let Some(ref id) = payload.swap_id {
        state.orderbook.mark_base_swap_claimed(id).await;
    }
    if let Some(ref h) = payload.hash160 {
        state.orderbook.mark_base_swap_claimed(h).await;
    }
    Json(serde_json::json!({ "success": true }))
}

#[derive(Deserialize)]
pub struct WithdrawBaseRequest {
    pub asset: String, // "ETH" or "USDC"
    pub destination: String,
    pub amount: f64,
}

pub async fn execute_vault_withdraw_base(
    state: &AppState,
    dest: &str,
    amount: f64,
    asset: &str,
) -> Result<String, String> {
    let dest = dest.trim();
    if dest.is_empty() {
        return Err("Destination address cannot be empty".to_string());
    }
    if amount <= 0.0 {
        return Err("Amount must be greater than 0".to_string());
    }

    let signer = match state.swap_engine.evm_signer.read().await.as_ref().cloned() {
        Some(s) => s,
        None => return Err("Base EVM signer not initialized".to_string()),
    };

    let result = if asset.eq_ignore_ascii_case("ETH") {
        signer.withdraw_eth(dest, amount).await
    } else {
        signer.withdraw_usdc(dest, amount).await
    };

    match result {
        Ok(tx_hash) => {
            state.tui.log(format!("Base withdrawal broadcast ({:.4} {} to {}...): {}", amount, asset.to_uppercase(), &dest[..dest.len().min(8)], tx_hash)).await;
            Ok(tx_hash)
        }
        Err(e) => Err(e),
    }
}

pub async fn post_vault_withdraw_base(
    State(state): State<AppState>,
    Json(payload): Json<WithdrawBaseRequest>,
) -> impl IntoResponse {
    match execute_vault_withdraw_base(&state, &payload.destination, payload.amount, &payload.asset).await {
        Ok(tx_hash) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "status": "ok",
                "tx_hash": tx_hash,
                "message": format!("Successfully broadcasted {} withdrawal", payload.asset.to_uppercase())
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "status": "error", "message": e })),
        ),
    }
}

#[derive(Deserialize)]
pub struct BaseGasEstimateQuery {
    pub asset: Option<String>,
}

pub async fn execute_base_gas_estimate(
    state: &AppState,
    asset: &str,
) -> Result<serde_json::Value, String> {
    let signer = match state.swap_engine.evm_signer.read().await.as_ref().cloned() {
        Some(s) => s,
        None => return Err("Base EVM signer not initialized".to_string()),
    };

    match signer.estimate_withdraw_fee(asset).await {
        Ok((total_fee_eth, gas_price_gwei, gas_limit, l1_fee_eth)) => {
            let formatted = if total_fee_eth < 0.000001 {
                format!("~{:.7} ETH ({:.3} Gwei)", total_fee_eth, gas_price_gwei)
            } else {
                format!("~{:.6} ETH ({:.3} Gwei)", total_fee_eth, gas_price_gwei)
            };
            Ok(serde_json::json!({
                "status": "ok",
                "asset": asset.to_uppercase(),
                "gas_price_gwei": gas_price_gwei,
                "gas_limit": gas_limit,
                "l1_fee_eth": l1_fee_eth,
                "total_fee_eth": total_fee_eth,
                "formatted_fee": formatted
            }))
        }
        Err(e) => Err(e),
    }
}

pub async fn get_vault_base_gas_estimate(
    State(state): State<AppState>,
    Query(query): Query<BaseGasEstimateQuery>,
) -> impl IntoResponse {
    let asset = query.asset.as_deref().unwrap_or("USDC");
    match execute_base_gas_estimate(&state, asset).await {
        Ok(v) => (StatusCode::OK, axum::Json(v)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "status": "error", "message": e })),
        ),
    }
}

#[derive(Deserialize)]
pub struct TakeOrderRequest {
    pub hash160: Option<String>,
    pub order_id: Option<String>,
}

pub async fn execute_take_order(
    state: &AppState,
    payload: TakeOrderRequest,
) -> Result<serde_json::Value, String> {
    let clean_hash = payload.hash160.as_deref().unwrap_or("")
        .strip_prefix("0x").unwrap_or(payload.hash160.as_deref().unwrap_or(""))
        .to_lowercase();
    let clean_id = payload.order_id.as_deref().unwrap_or("")
        .strip_prefix("0x").unwrap_or(payload.order_id.as_deref().unwrap_or(""))
        .to_lowercase();

    // 1. Find order in orderbook
    let order = if !clean_hash.is_empty() {
        state.orderbook.get_order_by_hash(&clean_hash).await
    } else if !clean_id.is_empty() {
        state.orderbook.get_order_by_hash(&clean_id).await
    } else {
        None
    };

    let order = match order {
        Some(o) => o,
        None => return Err("Order not found in orderbook".to_string()),
    };

    // 2. Fetch our local vault credentials
    let (our_tru_pubkey, our_tru_addr, our_base_addr, our_tru_priv, our_base_priv) = {
        let mut v = state.vault.lock().await;
        let t_priv = v.get_priv_key().ok();
        let (pk, addr) = v.get_or_create_wallet().map(|kp| (hex::encode(kp.pub_key), kp.address)).unwrap_or_default();
        let base = v.get_base_address().unwrap_or_default();
        let b_key = v.get_base_priv_key().ok();
        (pk, addr, base, t_priv, b_key)
    };

    let canonical_take_msg = crate::relay::crypto::canonical_take_message(
        &order.id,
        &our_base_addr,
        &our_tru_addr,
    );

    let signature_tru = our_tru_priv.as_ref().and_then(|tpk| {
        crate::relay::crypto::sign_tru_message(&canonical_take_msg, tpk).ok()
    });

    let signature_base = our_base_priv.as_ref().and_then(|bpk| {
        crate::relay::crypto::sign_personal_message(&canonical_take_msg, bpk).ok()
    });

    if signature_tru.is_none() && signature_base.is_none() {
        return Err("Cannot take order: No private keys (TRU or Base) found in vault".to_string());
    }

    let hash_bytes = match hex::decode(&order.hash160_hex) {
        Ok(b) if b.len() == 20 => {
            let mut arr = [0u8; 20];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return Err("Invalid order hash160".to_string()),
    };

    match order.side {
        crate::swap::orderbook::OrderSide::SellTruForBaseUsdc => {
            // Maker is selling TRU. Taker (we) is BUYING TRU with Base USDC!
            // Zero-Trust Security Protocol: Taker does NOT lock USDC on Base upfront!
            // Taker registers intent, notifies Maker via Relay mesh so Maker locks TRU first,
            // and spawns `wait_tru_safe_and_lock_base` which locks USDC on Base ONLY AFTER 6 TRU confirmations!
            if let Some(signer) = state.swap_engine.evm_signer.read().await.as_ref() {
                if let Ok(bal) = signer.get_usdc_balance(None).await {
                    if order.amount_usdc > bal {
                        return Err(format!("Insufficient Base USDC balance to take order: have {:.2} USDC, need {:.2} USDC", bal, order.amount_usdc));
                    }
                }
                if let Ok(eth_bal) = signer.get_eth_balance(None).await {
                    let min_eth_fee = signer
                        .estimate_create_swap_fee()
                        .await
                        .map(|(eth, _)| eth)
                        .unwrap_or(0.00003);
                    if eth_bal < min_eth_fee {
                        return Err(format!(
                            "Insufficient Base ETH gas to take order: have {:.6} ETH, estimated required is {:.6} ETH (gas fee to lock USDC on Base)",
                            eth_bal, min_eth_fee
                        ));
                    }
                }
            }

            let beneficiary = match order.maker_base_address.as_deref() {
                Some(b) if !b.is_empty() => b.to_string(),
                _ => return Err("Maker Base address unknown for this order".to_string()),
            };

            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            let mut swap_id_bytes = [0u8; 32];
            if let Ok(b) = hex::decode(&order.id) {
                if b.len() == 32 {
                    swap_id_bytes.copy_from_slice(&b);
                }
            }

            let record = crate::wallet::keystore::SwapRecord {
                swap_id: swap_id_bytes,
                secret: [0u8; 32],
                hash160: hash_bytes,
                direction: 2, // Buy TRU
                step: 1,      // Step 1: Matched / Intent, awaiting 6 TRU confirmations
                status: 0,    // Pending
                tru_atoms: (order.amount_tru * 100_000_000.0).round() as u64,
                usdc_atomic: (order.amount_usdc * 1_000_000.0).round() as u64,
                timelock: (now as u32) + 14400,
                created_at: now,
                beneficiary: beneficiary.clone(),
            };

            {
                let mut v = state.vault.lock().await;
                let mut recs = v.load_records().unwrap_or_default();
                recs.push(record);
                let _ = v.save_records(&recs);
            }

            state.orderbook.set_taker_pubkey(&order.hash160_hex, &our_tru_pubkey).await;
            state.orderbook.set_taker_base(&order.hash160_hex, &our_base_addr).await;
            state.orderbook.mark_matched(&order.hash160_hex).await;
            state.orderbook.mark_matched(&order.id).await;

            let payload = crate::relay::protocol::TakeOrderPayload {
                order_id: order.id.clone(),
                taker_base_address: our_base_addr.clone(),
                taker_tru_address: our_tru_addr.clone(),
                taker_tru_pubkey: Some(our_tru_pubkey.clone()),
                signature_base: signature_base.clone(),
                signature_tru: signature_tru.clone(),
            };
            if let Err(e) = state.relay_client.take_order(payload, std::time::Duration::from_secs(6)).await {
                tracing::warn!("[RELAY-DISPATCH] Relay rejected TakeOrder: {}", e);
                return Err(format!("Relay rejected take: {}", e));
            }
            tracing::info!("[RELAY-DISPATCH] Relay accepted TakeOrder for {}", order.id);

            // Start autonomous 6-confirmation safety guard for Base USDC lock
            state.swap_engine.wait_tru_safe_and_lock_base(beneficiary, order.amount_usdc, hash_bytes, (now as u32) + 14400).await;

            Ok(serde_json::json!({
                "success": true,
                "message": format!("Order matched! Monitoring TRU blockchain: {:.2} USDC will be locked on Base automatically after 6 TRU confirmations (Zero-Risk Protection).", order.amount_usdc)
            }))
        }
        crate::swap::orderbook::OrderSide::BuyTruWithBaseUsdc => {
            // Maker wants to buy TRU with USDC. Taker (we) is SELLING TRU for USDC!
            // We lock TRU on the TRU network for Maker!
            let utxos = state.block_scanner.get_vault_utxos().await;
            let verified_h = state.block_scanner.verified_height();
            let spendable: u64 = utxos
                .iter()
                .filter(|u| u.block_height > 0 && verified_h >= u.block_height as u64)
                .map(|u| u.amount_atoms)
                .sum();
            let required_atoms = (order.amount_tru * 100_000_000.0).round() as u64;

            let required_fee_buffer = 500u64; // 250 claim surplus + 250 lock tx fee
            if required_atoms + required_fee_buffer > spendable {
                let have_tru = spendable as f64 / 100_000_000.0;
                let max_tru = spendable.saturating_sub(required_fee_buffer) as f64 / 100_000_000.0;
                return Err(format!("Insufficient confirmed TRU balance to take order: have {:.8} TRU, maximum sellable is {:.8} TRU (requires ~{} atoms for network fee & claim buffer)", have_tru, max_tru, required_fee_buffer));
            }

            // Pre-relay Gas Guard: Taker selling TRU requires Base ETH gas to claim USDC after swap
            if let Some(signer) = state.swap_engine.evm_signer.read().await.as_ref() {
                if let Ok(eth_bal) = signer.get_eth_balance(None).await {
                    let min_eth_fee = signer
                        .estimate_claim_swap_fee()
                        .await
                        .map(|(eth, _)| eth)
                        .unwrap_or(0.00002);
                    if eth_bal < min_eth_fee {
                        return Err(format!(
                            "Insufficient Base ETH gas to take order: have {:.6} ETH, estimated required is {:.6} ETH (gas fee to claim USDC on Base after swap)",
                            eth_bal, min_eth_fee
                        ));
                    }
                }
            }

            let maker_tru_pubkey = match order.maker_tru_pubkey.as_deref() {
                Some(pk) if !pk.is_empty() => pk.to_string(),
                _ => return Err("Maker TRU pubkey unknown for this order".to_string()),
            };

            state.orderbook.register_pubkey(&maker_tru_pubkey).await;
            state.orderbook.register_pubkey(&our_tru_pubkey).await;
            state.orderbook.set_taker_pubkey(&order.hash160_hex, &our_tru_pubkey).await;
            state.orderbook.set_taker_pubkey(&order.id, &our_tru_pubkey).await;
            state.orderbook.set_taker_base(&order.hash160_hex, &our_base_addr).await;
            state.orderbook.set_taker_base(&order.id, &our_base_addr).await;
            state.orderbook.mark_matched(&order.hash160_hex).await;
            state.orderbook.mark_matched(&order.id).await;

            let payload = crate::relay::protocol::TakeOrderPayload {
                order_id: order.id.clone(),
                taker_base_address: our_base_addr.clone(),
                taker_tru_address: our_tru_addr.clone(),
                taker_tru_pubkey: Some(our_tru_pubkey.clone()),
                signature_base,
                signature_tru,
            };
            if let Err(e) = state.relay_client.take_order(payload, std::time::Duration::from_secs(6)).await {
                tracing::warn!("[RELAY-DISPATCH] Relay rejected TakeOrder: {}", e);
                return Err(format!("Relay rejected take: {}", e));
            }
            tracing::info!("[RELAY-DISPATCH] Relay accepted TakeOrder for {}", order.id);

            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            let mut swap_id_bytes = [0u8; 32];
            if let Ok(b) = hex::decode(&order.id) {
                if b.len() == 32 {
                    swap_id_bytes.copy_from_slice(&b);
                }
            }

            let record = crate::wallet::keystore::SwapRecord {
                swap_id: swap_id_bytes,
                secret: [0u8; 32],
                hash160: hash_bytes,
                direction: 1, // Sell TRU
                step: 1,      // Step 1: Matched / Intent, awaiting Buyer's Base USDC lock (Zero-Trust)
                status: 0,
                tru_atoms: (order.amount_tru * 100_000_000.0).round() as u64,
                usdc_atomic: (order.amount_usdc * 1_000_000.0).round() as u64,
                timelock: (now as u32) + 14400,
                created_at: now,
                beneficiary: our_base_addr.clone(),
            };

            {
                let mut v = state.vault.lock().await;
                let mut recs = v.load_records().unwrap_or_default();
                if let Some(pos) = recs.iter().position(|r| r.hash160 == hash_bytes) {
                    recs[pos] = record;
                } else {
                    recs.push(record);
                }
                let _ = v.save_records(&recs);
            }

            state.tui.log(format!(
                "🔒 [Seller Zero-Trust Guard] Order accepted! Awaiting Buyer's deposit of {:.2} USDC in BaseHTLC before locking {:.4} TRU on chain...",
                order.amount_usdc, order.amount_tru
            )).await;

            Ok(serde_json::json!({
                "success": true,
                "message": format!(
                    "Order matched! Monitoring {}: {:.4} TRU will be locked on TRU chain automatically once Buyer locks {:.2} USDC on Base (Zero-Risk Protection).",
                    state.config.network_name(),
                    order.amount_tru,
                    order.amount_usdc
                )
            }))
        }
    }
}

pub async fn post_take_order(
    State(state): State<AppState>,
    Json(payload): Json<TakeOrderRequest>,
) -> impl IntoResponse {
    match execute_take_order(&state, payload).await {
        Ok(v) => (StatusCode::OK, axum::Json(v)),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "success": false, "error": e })),
        ),
    }
}

pub async fn execute_lock_base_htlc(
    state: &AppState,
    payload: super::routes::LockBaseHtlcRequest,
) -> Result<serde_json::Value, String> {
    let clean_hash = payload.hash160.strip_prefix("0x").unwrap_or(&payload.hash160).to_lowercase();
    let hash_bytes = match hex::decode(&clean_hash) {
        Ok(b) if b.len() == 20 => {
            let mut arr = [0u8; 20];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return Err("Invalid hash160 hex".to_string()),
    };

    // Prevent duplicate or already completed locks
    {
        let v = state.vault.lock().await;
        if let Ok(recs) = v.load_records() {
            if let Some(r) = recs.iter().find(|r| hex::encode(r.hash160).to_lowercase() == clean_hash) {
                if r.status == 1 || r.status == 2 {
                    return Err("Swap order has already been completed or settled".to_string());
                }
            }
        }
    }

    let base_swap_map = state.orderbook.get_all_base_swap_ids().await;
    if base_swap_map.contains_key(&clean_hash) {
        return Err("Base HTLC lock already exists for this swap".to_string());
    }

    let (_is_safe, confs) = state.block_scanner.get_swap_safety(&clean_hash).await;
    if confs < 3 {
        return Err(format!(
            "Security rule violation: Seller TRU lock has {} of 6 confirmations. Manual lock requires at least 3 confirmations in TRU blockchain.",
            confs
        ));
    }

    let signer = match state.swap_engine.evm_signer.read().await.as_ref().cloned() {
        Some(s) => s,
        None => return Err("Base EVM signer not initialized".to_string()),
    };

    let signer_addr = signer.address().to_string();
    let beneficiary = {
        let mut target = payload.beneficiary.unwrap_or_default().trim().to_string();
        // If beneficiary is empty or accidentally passed as our own address, resolve counterparty from vault or orderbook
        if target.is_empty() || target.eq_ignore_ascii_case(&signer_addr) {
            let v = state.vault.lock().await;
            if let Ok(recs) = v.load_records() {
                if let Some(r) = recs.iter().find(|r| r.hash160 == hash_bytes) {
                    if !r.beneficiary.is_empty() && !r.beneficiary.eq_ignore_ascii_case(&signer_addr) {
                        target = r.beneficiary.clone();
                    }
                }
            }
        }
        if target.is_empty() || target.eq_ignore_ascii_case(&signer_addr) {
            for o in state.orderbook.get_active_orders().await {
                if o.hash160_hex.eq_ignore_ascii_case(&clean_hash) {
                    if let Some(ref b) = o.maker_base_address {
                        if !b.eq_ignore_ascii_case(&signer_addr) {
                            target = b.clone();
                            break;
                        }
                    }
                    if let Some(ref b) = o.taker_base_address {
                        if !b.eq_ignore_ascii_case(&signer_addr) {
                            target = b.clone();
                            break;
                        }
                    }
                }
            }
        }
        if target.is_empty() || target.eq_ignore_ascii_case(&signer_addr) {
            return Err("Cannot lock USDC: counterparty Base address could not be resolved or equals own address (self-swap)".to_string());
        }
        target
    };

    let timelock = payload.timelock.unwrap_or_else(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        (now + 7200) as u32
    });

    state.tui.log(format!("Locking {:.2} USDC on Base for {}...", payload.amount_usdc, &beneficiary[..beneficiary.len().min(8)])).await;

    match signer.create_swap(&beneficiary, payload.amount_usdc, hash_bytes, timelock).await {
        Ok((swap_id, tx_hash)) => {
            state.tui.log(format!("USDC locked on Base (SwapId: 0x{}... TX: {})", &swap_id[2..swap_id.len().min(10)], tx_hash)).await;
            state.orderbook.record_base_swap_id(&clean_hash, &swap_id).await;
            state.orderbook.record_base_lock_tx(&clean_hash, &tx_hash).await;
            state.orderbook.record_base_lock_tx(&swap_id, &tx_hash).await;

            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            let mut swap_id_bytes = [0u8; 32];
            if let Ok(b) = hex::decode(swap_id.strip_prefix("0x").unwrap_or(&swap_id)) {
                if b.len() == 32 {
                    swap_id_bytes.copy_from_slice(&b);
                }
            }

            {
                let mut v = state.vault.lock().await;
                let mut recs = v.load_records().unwrap_or_default();
                if let Some(pos) = recs.iter().position(|r| r.hash160 == hash_bytes) {
                    recs[pos].step = 3;
                    recs[pos].swap_id = swap_id_bytes;
                    if !beneficiary.is_empty() {
                        recs[pos].beneficiary = beneficiary.clone();
                    }
                } else {
                    let record = crate::wallet::keystore::SwapRecord {
                        swap_id: swap_id_bytes,
                        secret: [0u8; 32],
                        hash160: hash_bytes,
                        direction: 2,
                        step: 3,
                        status: 0,
                        tru_atoms: 0,
                        usdc_atomic: (payload.amount_usdc * 1_000_000.0).round() as u64,
                        timelock,
                        created_at: now,
                        beneficiary: beneficiary.clone(),
                    };
                    recs.push(record);
                }
                let _ = v.save_records(&recs);
            }

            Ok(serde_json::json!({
                "success": true,
                "swap_id": swap_id,
                "tx_hash": tx_hash,
                "message": format!("Successfully locked {:.2} USDC on Base", payload.amount_usdc)
            }))
        }
        Err(e) => {
            state.tui.log(format!("⚠️ Failed to lock USDC on Base: {}", e)).await;
            Err(e)
        }
    }
}

pub async fn post_lock_base_htlc(
    State(state): State<AppState>,
    Json(payload): Json<super::routes::LockBaseHtlcRequest>,
) -> impl IntoResponse {
    match execute_lock_base_htlc(&state, payload).await {
        Ok(v) => (StatusCode::OK, axum::Json(v)),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "success": false, "error": e })),
        ),
    }
}

pub async fn execute_claim_base_htlc(
    state: &AppState,
    payload: super::routes::ClaimBaseHtlcRequest,
) -> Result<serde_json::Value, String> {
    let clean_hash = payload.hash160.as_deref().unwrap_or("")
        .strip_prefix("0x").unwrap_or(payload.hash160.as_deref().unwrap_or(""))
        .to_lowercase();
    let clean_swap_id = payload.swap_id.as_deref().unwrap_or("")
        .strip_prefix("0x").unwrap_or(payload.swap_id.as_deref().unwrap_or(""))
        .to_lowercase();

    let signer = match state.swap_engine.evm_signer.read().await.as_ref().cloned() {
        Some(s) => s,
        None => return Err("Base EVM signer not initialized".to_string()),
    };

    let (found_swap_id, secret) = {
        let v = state.vault.lock().await;
        let recs = v.load_records().unwrap_or_default();
        let mut sid = if clean_swap_id.len() == 64 { Some(clean_swap_id.clone()) } else { None };
        let mut sec = None;

        for r in &recs {
            let r_hash = hex::encode(r.hash160);
            let r_swap = hex::encode(r.swap_id);
            if (!clean_hash.is_empty() && r_hash.eq_ignore_ascii_case(&clean_hash)) ||
               (!clean_swap_id.is_empty() && r_swap.eq_ignore_ascii_case(&clean_swap_id)) {
                if sid.is_none() && r.swap_id != [0u8; 32] {
                    sid = Some(r_swap);
                }
                if r.secret != [0u8; 32] {
                    sec = Some(r.secret);
                }
            }
        }
        (sid, sec)
    };

    let mut secret = secret;
    if secret.is_none() {
        if let Some(ref s_hex) = payload.secret {
            let clean_s = s_hex.strip_prefix("0x").unwrap_or(s_hex);
            if let Ok(b) = hex::decode(clean_s) {
                if b.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&b);
                    secret = Some(arr);
                }
            }
        }
    }
    if secret.is_none() && !clean_hash.is_empty() {
        if let Some(s_hex) = state.block_scanner.get_swap_secret(&clean_hash).await {
            if let Ok(b) = hex::decode(&s_hex) {
                if b.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&b);
                    secret = Some(arr);
                }
            }
        }
    }
    if secret.is_none() && !clean_hash.is_empty() {
        if let Some(s_hex) = state.orderbook.get_secret(&clean_hash).await {
            if let Ok(b) = hex::decode(&s_hex) {
                if b.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&b);
                    secret = Some(arr);
                }
            }
        }
    }

    let secret = match secret {
        Some(s) => s,
        None => return Err("Preimage secret not found in vault, scanner, or orderbook for this swap".to_string()),
    };

    let mut final_swap_id = found_swap_id;
    if final_swap_id.is_none() && !clean_hash.is_empty() {
        if let Some(o) = state.orderbook.get_order_by_hash(&clean_hash).await {
            let clean_oid = o.id.strip_prefix("0x").unwrap_or(&o.id).to_lowercase();
            if clean_oid.len() == 64 {
                final_swap_id = Some(clean_oid);
            }
        }
    }

    let final_swap_id = match final_swap_id {
        Some(s) => s,
        None => return Err("Target Base swap_id unknown".to_string()),
    };

    state.tui.log(format!("Claiming Base HTLC (SwapId: 0x{}...)...", &final_swap_id[..final_swap_id.len().min(8)])).await;

    match signer.claim_swap(&final_swap_id, &secret).await {
        Ok(tx_hash) => {
            state.tui.log(format!("USDC claimed on Base! TX: {}", tx_hash)).await;
            {
                let mut v = state.vault.lock().await;
                if let Ok(mut recs) = v.load_records() {
                    for r in &mut recs {
                        let r_hash = hex::encode(r.hash160);
                        let r_swap = hex::encode(r.swap_id);
                        if r_hash.eq_ignore_ascii_case(&clean_hash) || r_swap.eq_ignore_ascii_case(&final_swap_id) {
                            r.step = 4;
                            r.status = 1;
                        }
                    }
                    let _ = v.save_records(&recs);
                }
            }
            Ok(serde_json::json!({
                "success": true,
                "tx_hash": tx_hash,
                "message": "Successfully claimed USDC on Base"
            }))
        }
        Err(e) => Err(e),
    }
}

pub async fn post_claim_base_htlc(
    State(state): State<AppState>,
    Json(payload): Json<super::routes::ClaimBaseHtlcRequest>,
) -> impl IntoResponse {
    match execute_claim_base_htlc(&state, payload).await {
        Ok(v) => (StatusCode::OK, axum::Json(v)),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "success": false, "error": e })),
        ),
    }
}

pub async fn get_tx_details(
    axum::extract::Path(txid): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let clean_txid = txid.trim().to_lowercase();
    if clean_txid.is_empty() || clean_txid.len() > 64 {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "found": false,
                "error": "Invalid transaction hash",
                "txid": clean_txid
            })),
        );
    }

    if let Some(details) = state.block_scanner.get_tx_details(&clean_txid).await {
        let is_confirmed = details.confirmations > 0 && details.block_height.is_some();
        let status = if is_confirmed { "CONFIRMED" } else { "PENDING" };

        let formatted_time = if details.timestamp > 0 {
            chrono::DateTime::from_timestamp(details.timestamp as i64, 0)
                .map(|dt| dt.format("%m/%d/%Y, %I:%M:%S %p").to_string())
                .unwrap_or_else(|| "Recent".to_string())
        } else {
            "Pending".to_string()
        };

        return (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "found": true,
                "txid": details.txid,
                "status": status,
                "is_confirmed": is_confirmed,
                "confirmations": details.confirmations,
                "block_height": details.block_height,
                "block_hash": details.block_hash,
                "timestamp": details.timestamp,
                "formatted_time": formatted_time,
                "version": details.version,
                "lock_time": details.lock_time,
                "is_coinbase": details.is_coinbase,
                "is_quantum": details.is_quantum,
                "inputs": details.inputs,
                "outputs": details.outputs,
            })),
        );
    }

    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "found": false,
            "error": "Transaction not found on TRU network or local node",
            "txid": clean_txid
        })),
    )
}

#[derive(Deserialize)]
pub struct BlocksQuery {
    pub limit: Option<usize>,
}

pub async fn get_blocks(
    State(state): State<AppState>,
    Query(query): Query<BlocksQuery>,
) -> Json<Vec<crate::tru_p2p::scanner::CachedTruBlock>> {
    let limit = query.limit.unwrap_or(30);
    Json(state.block_scanner.get_recent_blocks(limit).await)
}

pub async fn get_block_details(
    State(state): State<AppState>,
    Path(query): Path<String>,
) -> impl IntoResponse {
    if let Some(block) = state.block_scanner.get_block_by_query(&query).await {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "found": true,
                "block": block
            })),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "found": false,
                "error": "Block not found in local chain index",
                "query": query
            })),
        )
    }
}

pub async fn get_explorer_stats(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(state.block_scanner.get_explorer_stats().await)
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

pub async fn get_explorer_search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Json<serde_json::Value> {
    let q = query.q.unwrap_or_default();
    Json(state.block_scanner.search_explorer(&q).await)
}

pub async fn get_automation_logs(State(state): State<AppState>) -> Json<serde_json::Value> {
    let logs = state.tui.get_events().await;
    Json(serde_json::json!({
        "status": "ok",
        "logs": logs
    }))
}


