use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use tracing::{info, warn};

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::handlers::{
    build_swaps_ledger_json, build_vault_status_json, execute_base_gas_estimate,
    execute_cancel_order, execute_claim_base_htlc, execute_claim_vault_htlc, execute_create_order,
    execute_lock_base_htlc, execute_take_order, execute_vault_withdraw,
    execute_vault_withdraw_base, fetch_raw_vault_status,
};
use super::routes::{AppState, AuthQuery};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    State(state): State<AppState>,
) -> Response {
    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());

    // 1. CSWSH Security Check: Validate Origin
    let is_trusted_origin = if let Some(orig) = origin {
        is_allowed_origin(orig, host)
    } else {
        true // Native/direct clients without Origin header
    };

    if !is_trusted_origin {
        warn!("Rejected WebSocket connection from unauthorized Origin: {:?}", origin);
        if let Some(orig) = origin {
            state.tui.log(format!("Rejected unauthorized WS Origin: {}", orig)).await;
        }
        return (StatusCode::FORBIDDEN, "Forbidden Origin").into_response();
    }

    // 2. Token Security Check
    let req_token = query.token.unwrap_or_default();
    let is_authenticated = !req_token.is_empty() && req_token == state.session_token;

    ws.on_upgrade(move |socket| handle_socket(socket, state, is_authenticated))
}

fn is_allowed_origin(origin: &str, host: &str) -> bool {
    let clean = origin
        .trim_start_matches("http://")
        .trim_start_matches("https://");

    // Exact host match (same-origin request)
    if !host.is_empty() {
        if clean == host || clean.split(':').next() == host.split(':').next() {
            return true;
        }
    }

    // Local loopback
    if clean.starts_with("127.0.0.1") || clean.starts_with("localhost") {
        return true;
    }

    // Private network & RFC 6598 CGNAT ranges (100.64.0.0/10)
    let domain = clean.split(':').next().unwrap_or("");
    if domain.starts_with("192.168.")
        || domain.starts_with("10.")
        || domain.starts_with("100.")
        || domain.starts_with("172.")
    {
        return true;
    }

    false
}

pub async fn handle_socket(socket: WebSocket, state: AppState, is_authenticated: bool) {
    let (mut sender, mut receiver) = socket.split();
    info!("WebSocket client connected (authenticated: {})", is_authenticated);
    state.tui.set_ws_clients(1).await;

    // Outbound text message queue
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<String>(256);

    // Initial state snapshot pushed immediately
    let verified = state.block_scanner.verified_height() as u32;
    let target = state.block_scanner.target_height() as u32;
    let p2p_tip = state.tru_client.p2p_node.current_height() as u32;
    let block_height = verified.max(target).max(p2p_tip);
    let block_height = if block_height > 0 {
        block_height
    } else {
        state.tru_client.get_block_count().await
    };

    let p2p_peers = state.tru_client.p2p_node.peer_count();
    let raw_vault = fetch_raw_vault_status(&state).await;
    let vault_json = build_vault_status_json(&state).await;

    // 1. Send "init"
    let init_json = serde_json::json!({
        "type": "init",
        "data": {
            "block_height": block_height,
            "confirmed_atoms": raw_vault.confirmed_atoms,
            "unconfirmed_atoms": raw_vault.unconfirmed_atoms,
            "vault_address": raw_vault.address,
            "p2p_peers": p2p_peers,
            "verified_height": verified,
            "target_height": target,
        }
    });
    let _ = out_tx.send(init_json.to_string()).await;

    // 2. Send "vault"
    let _ = out_tx.send(serde_json::json!({
        "type": "vault",
        "data": vault_json,
    }).to_string()).await;

    // 3. Send "mempool"
    let mempool_txs = state.block_scanner.get_mempool().await;
    let _ = out_tx.send(serde_json::json!({
        "type": "mempool",
        "data": mempool_txs,
    }).to_string()).await;

    // 4. Send "swaps_ledger"
    let ledger_json = build_swaps_ledger_json(&state).await;
    let _ = out_tx.send(serde_json::json!({
        "type": "swaps_ledger",
        "data": ledger_json,
    }).to_string()).await;

    // 5. Send "orderbook_snapshot"
    let active_orders = state.orderbook.get_active_orders().await;
    let _ = out_tx.send(serde_json::json!({
        "type": "orderbook_snapshot",
        "data": active_orders,
    }).to_string()).await;

    // Listen for broadcast events
    let mut rx_ui = state.ui_events.subscribe();
    let mut rx_tx_events = state.tx_events.subscribe();
    let state_reader = state.clone();
    let out_tx_reader = out_tx.clone();

    // Reader task: handles incoming JSON text messages from frontend
    let mut reader_task = tokio::spawn(async move {
        let mut is_auth = is_authenticated;
        let mut qr_scanner_handle: Option<crate::ndk_camera::ScannerHandle> = None;
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                    let action = val.get("action").and_then(|v| v.as_str()).unwrap_or("");
                    let req_id = val.get("req_id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let payload_val = val.get("payload").unwrap_or(&val).clone();

                    // 1. Explicit auth message
                    if action == "auth" {
                        let tok = payload_val.get("token")
                            .or_else(|| val.get("token"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if tok == state_reader.session_token {
                            is_auth = true;
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "auth",
                                    "req_id": req_id,
                                    "success": true,
                                    "message": "Authenticated successfully"
                                }
                            }).to_string()).await;
                        } else {
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "auth",
                                    "req_id": req_id,
                                    "success": false,
                                    "error": "Invalid session token"
                                }
                            }).to_string()).await;
                        }
                        continue;
                    }

                    // 2. Sensitive / Mutating actions require authentication
                    let requires_auth = matches!(
                        action,
                        "req_withdraw" | "withdraw_tru" | "withdraw_base" | "req_vault_export" |
                        "create_order" | "take_order" | "cancel_order" | "lock_base_htlc" |
                        "claim_base_htlc" | "claim_vault_htlc"
                    );

                    if requires_auth && !is_auth {
                        let msg_tok = val.get("token")
                            .or_else(|| payload_val.get("token"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if msg_tok == state_reader.session_token {
                            is_auth = true;
                        } else {
                            warn!("Blocked unauthorized sensitive WebSocket action '{}'", action);
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": action,
                                    "req_id": req_id,
                                    "success": false,
                                    "error": "Unauthorized: valid session token required"
                                }
                            }).to_string()).await;
                            continue;
                        }
                    }

                    match action {
                        "start_qr_scan" => {
                            if let Some(h) = qr_scanner_handle.take() {
                                h.stop();
                            }
                            let facing_str = payload_val.get("facing")
                                .or_else(|| val.get("facing"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("environment");
                            let facing = if facing_str == "user" {
                                crate::ndk_camera::CameraFacing::Front
                            } else {
                                crate::ndk_camera::CameraFacing::Back
                            };

                            let initial_zoom = payload_val.get("zoom")
                                .or_else(|| val.get("zoom"))
                                .and_then(|v| v.as_f64())
                                .unwrap_or(1.5) as f32;
                            crate::ndk_camera::set_zoom(initial_zoom);

                            let out_code = out_tx_reader.clone();
                            let out_frame = out_tx_reader.clone();

                            match crate::ndk_camera::start_qr_scanner(
                                facing,
                                move |code| {
                                    let _ = out_code.try_send(serde_json::json!({
                                        "type": "qr_scanned",
                                        "data": { "code": code }
                                    }).to_string());
                                },
                                move |b64_frame| {
                                    let _ = out_frame.try_send(serde_json::json!({
                                        "type": "qr_frame",
                                        "data": { "img": b64_frame }
                                    }).to_string());
                                },
                            ) {
                                Ok(handle) => {
                                    qr_scanner_handle = Some(handle);
                                    let _ = out_tx_reader.send(serde_json::json!({
                                        "type": "action_result",
                                        "data": {
                                            "action": "start_qr_scan",
                                            "req_id": req_id,
                                            "success": true
                                        }
                                    }).to_string()).await;
                                }
                                Err(e) => {
                                    warn!("start_qr_scanner error: {}", e);
                                    let _ = out_tx_reader.send(serde_json::json!({
                                        "type": "action_result",
                                        "data": {
                                            "action": "start_qr_scan",
                                            "req_id": req_id,
                                            "success": false,
                                            "error": e
                                        }
                                    }).to_string()).await;
                                }
                            }
                        }
                        "stop_qr_scan" => {
                            if let Some(h) = qr_scanner_handle.take() {
                                h.stop();
                            }
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "stop_qr_scan",
                                    "req_id": req_id,
                                    "success": true
                                }
                            }).to_string()).await;
                        }
                        "set_qr_zoom" => {
                            let zoom = payload_val.get("zoom")
                                .or_else(|| val.get("zoom"))
                                .and_then(|v| v.as_f64())
                                .unwrap_or(1.5) as f32;
                            crate::ndk_camera::set_zoom(zoom);
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "set_qr_zoom",
                                    "req_id": req_id,
                                    "success": true,
                                    "zoom": zoom
                                }
                            }).to_string()).await;
                        }
                        "get_clipboard" => {
                            let text = crate::android::get_clipboard_text().unwrap_or_default();
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "get_clipboard",
                                    "req_id": req_id,
                                    "success": true,
                                    "text": text
                                }
                            }).to_string()).await;
                        }
                        "set_clipboard" => {
                            let text = payload_val.get("text")
                                .or_else(|| val.get("text"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let ok = crate::android::set_clipboard_text(text);
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "set_clipboard",
                                    "req_id": req_id,
                                    "success": ok
                                }
                            }).to_string()).await;
                        }
                        "open_browser" | "open_url" => {
                            let url = payload_val.get("url")
                                .or_else(|| val.get("url"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let ok = if !url.is_empty() {
                                crate::android::open_browser_url(url)
                            } else {
                                false
                            };
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "open_browser",
                                    "req_id": req_id,
                                    "success": ok,
                                    "url": url
                                }
                            }).to_string()).await;
                        }
                        "ping" => {
                            let _ = out_tx_reader.send(serde_json::json!({ "type": "pong" }).to_string()).await;
                        }
                        "req_vault_status" => {
                            let status = build_vault_status_json(&state_reader).await;
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "vault",
                                "data": status,
                            }).to_string()).await;
                        }
                        "req_mempool" => {
                            let txs = state_reader.block_scanner.get_mempool().await;
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "mempool",
                                "data": txs,
                            }).to_string()).await;
                        }
                        "req_swaps_ledger" => {
                            let ledger = build_swaps_ledger_json(&state_reader).await;
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "swaps_ledger",
                                "data": ledger,
                            }).to_string()).await;
                        }
                        "req_automation_logs" => {
                            let logs = state_reader.tui.get_events().await;
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "automation_logs",
                                "data": logs,
                            }).to_string()).await;
                        }
                        "req_orderbook" => {
                            let orders = state_reader.orderbook.get_active_orders().await;
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "orderbook_snapshot",
                                "data": orders,
                            }).to_string()).await;
                        }
                        "req_vault_export" => {
                            let token = val.get("token").and_then(|v| v.as_str()).unwrap_or("");
                            let export_result = if token == state_reader.session_token {
                                let mut vault = state_reader.vault.lock().await;
                                match vault.export_wallet() {
                                    Ok(exp) => serde_json::json!({
                                        "success": true,
                                        "address": exp.address,
                                        "pub_key_hex": exp.pub_key_hex,
                                        "priv_key_hex": exp.priv_key_hex,
                                        "priv_key_wif": exp.priv_key_wif,
                                        "mnemonic": exp.mnemonic,
                                        "base_address": exp.base_address,
                                        "base_priv_key_hex": exp.base_priv_key_hex,
                                        "base_mnemonic": exp.base_mnemonic,
                                    }),
                                    Err(e) => serde_json::json!({
                                        "success": false,
                                        "error": e,
                                    }),
                                }
                            } else {
                                serde_json::json!({
                                    "success": false,
                                    "error": "Unauthorized: Invalid session token",
                                })
                            };
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "vault_export",
                                "data": export_result,
                            }).to_string()).await;
                        }
                        "req_withdraw" | "withdraw_tru" => {
                            let target = payload_val.get("destination")
                                .or_else(|| payload_val.get("target_address"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let amount = if let Some(tru) = payload_val.get("amount_tru").and_then(|v| v.as_f64()) {
                                tru
                            } else if let Some(atoms) = payload_val.get("amount_atoms").and_then(|v| v.as_u64()) {
                                atoms as f64 / 100_000_000.0
                            } else {
                                0.0
                            };

                            let (success, txid, err_msg) = match execute_vault_withdraw(&state_reader, target, amount).await {
                                Ok(t) => (true, t, String::new()),
                                Err(e) => (false, String::new(), e),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": action,
                                    "req_id": req_id,
                                    "success": success,
                                    "txid": txid,
                                    "error": if err_msg.is_empty() { None } else { Some(&err_msg) },
                                }
                            }).to_string()).await;

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "withdraw_result",
                                "data": {
                                    "req_id": req_id,
                                    "success": success,
                                    "txid": txid,
                                    "error": if err_msg.is_empty() { None } else { Some(err_msg) },
                                }
                            }).to_string()).await;

                            // Immediate push of updated vault status & mempool
                            let status = build_vault_status_json(&state_reader).await;
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "vault",
                                "data": status,
                            }).to_string()).await;

                            let txs = state_reader.block_scanner.get_mempool().await;
                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "mempool",
                                "data": txs,
                            }).to_string()).await;
                        }
                        "create_order" => {
                            let req_res = serde_json::from_value::<super::routes::CreateOrderRequest>(payload_val);
                            let (success, res_data, err_msg) = match req_res {
                                Ok(req) => match execute_create_order(&state_reader, req).await {
                                    Ok(data) => (true, Some(data), None),
                                    Err(e) => (false, None, Some(e)),
                                },
                                Err(e) => (false, None, Some(format!("Invalid create_order payload: {}", e))),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "create_order",
                                    "req_id": req_id,
                                    "success": success,
                                    "data": res_data,
                                    "error": err_msg,
                                }
                            }).to_string()).await;

                            if success {
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "vault",
                                    "data": build_vault_status_json(&state_reader).await,
                                }).to_string()).await;
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "orderbook_snapshot",
                                    "data": state_reader.orderbook.get_active_orders().await,
                                }).to_string()).await;
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "swaps_ledger",
                                    "data": build_swaps_ledger_json(&state_reader).await,
                                }).to_string()).await;
                            }
                        }
                        "take_order" => {
                            let req_res = serde_json::from_value::<super::handlers::TakeOrderRequest>(payload_val);
                            let (success, res_data, err_msg) = match req_res {
                                Ok(req) => match execute_take_order(&state_reader, req).await {
                                    Ok(data) => (true, Some(data), None),
                                    Err(e) => (false, None, Some(e)),
                                },
                                Err(e) => (false, None, Some(format!("Invalid take_order payload: {}", e))),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "take_order",
                                    "req_id": req_id,
                                    "success": success,
                                    "data": res_data,
                                    "error": err_msg,
                                }
                            }).to_string()).await;

                            if success {
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "vault",
                                    "data": build_vault_status_json(&state_reader).await,
                                }).to_string()).await;
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "orderbook_snapshot",
                                    "data": state_reader.orderbook.get_active_orders().await,
                                }).to_string()).await;
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "swaps_ledger",
                                    "data": build_swaps_ledger_json(&state_reader).await,
                                }).to_string()).await;
                            }
                        }
                        "cancel_order" => {
                            let id_val = payload_val.get("id")
                                .or_else(|| payload_val.get("order_id"))
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let hash_val = payload_val.get("hash160")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();

                            let (success, res_data, err_msg) = match execute_cancel_order(&state_reader, id_val, hash_val).await {
                                Ok(data) => (true, Some(data), None),
                                Err(e) => (false, None, Some(e)),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "cancel_order",
                                    "req_id": req_id,
                                    "success": success,
                                    "data": res_data,
                                    "error": err_msg,
                                }
                            }).to_string()).await;

                            if success {
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "orderbook_snapshot",
                                    "data": state_reader.orderbook.get_active_orders().await,
                                }).to_string()).await;
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "swaps_ledger",
                                    "data": build_swaps_ledger_json(&state_reader).await,
                                }).to_string()).await;
                            }
                        }
                        "lock_base_htlc" => {
                            let req_res = serde_json::from_value::<super::routes::LockBaseHtlcRequest>(payload_val);
                            let (success, res_data, err_msg) = match req_res {
                                Ok(req) => match execute_lock_base_htlc(&state_reader, req).await {
                                    Ok(data) => (true, Some(data), None),
                                    Err(e) => (false, None, Some(e)),
                                },
                                Err(e) => (false, None, Some(format!("Invalid lock_base_htlc payload: {}", e))),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "lock_base_htlc",
                                    "req_id": req_id,
                                    "success": success,
                                    "data": res_data,
                                    "error": err_msg,
                                }
                            }).to_string()).await;

                            if success {
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "vault",
                                    "data": build_vault_status_json(&state_reader).await,
                                }).to_string()).await;
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "swaps_ledger",
                                    "data": build_swaps_ledger_json(&state_reader).await,
                                }).to_string()).await;
                            }
                        }
                        "claim_base_htlc" => {
                            let req_res = serde_json::from_value::<super::routes::ClaimBaseHtlcRequest>(payload_val);
                            let (success, res_data, err_msg) = match req_res {
                                Ok(req) => match execute_claim_base_htlc(&state_reader, req).await {
                                    Ok(data) => (true, Some(data), None),
                                    Err(e) => (false, None, Some(e)),
                                },
                                Err(e) => (false, None, Some(format!("Invalid claim_base_htlc payload: {}", e))),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "claim_base_htlc",
                                    "req_id": req_id,
                                    "success": success,
                                    "data": res_data,
                                    "error": err_msg,
                                }
                            }).to_string()).await;

                            if success {
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "vault",
                                    "data": build_vault_status_json(&state_reader).await,
                                }).to_string()).await;
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "swaps_ledger",
                                    "data": build_swaps_ledger_json(&state_reader).await,
                                }).to_string()).await;
                            }
                        }
                        "claim_vault_htlc" => {
                            let hash160 = payload_val.get("hash160").and_then(|v| v.as_str()).unwrap_or_default();
                            let secret_hex = payload_val.get("secret").and_then(|v| v.as_str());

                            let (success, res_data, err_msg) = match execute_claim_vault_htlc(&state_reader, hash160, secret_hex).await {
                                Ok(data) => (true, Some(data), None),
                                Err(e) => (false, None, Some(e)),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "claim_vault_htlc",
                                    "req_id": req_id,
                                    "success": success,
                                    "data": res_data,
                                    "error": err_msg,
                                }
                            }).to_string()).await;

                            if success {
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "vault",
                                    "data": build_vault_status_json(&state_reader).await,
                                }).to_string()).await;
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "swaps_ledger",
                                    "data": build_swaps_ledger_json(&state_reader).await,
                                }).to_string()).await;
                            }
                        }
                        "withdraw_base" => {
                            let dest = payload_val.get("destination").and_then(|v| v.as_str()).unwrap_or("");
                            let amount = payload_val.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let asset = payload_val.get("asset").and_then(|v| v.as_str()).unwrap_or("USDC");

                            let (success, tx_hash, err_msg) = match execute_vault_withdraw_base(&state_reader, dest, amount, asset).await {
                                Ok(h) => (true, Some(h), None),
                                Err(e) => (false, None, Some(e)),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "withdraw_base",
                                    "req_id": req_id,
                                    "success": success,
                                    "tx_hash": tx_hash,
                                    "error": err_msg,
                                }
                            }).to_string()).await;

                            if success {
                                let _ = out_tx_reader.send(serde_json::json!({
                                    "type": "vault",
                                    "data": build_vault_status_json(&state_reader).await,
                                }).to_string()).await;
                            }
                        }
                        "get_base_gas_estimate" => {
                            let asset = payload_val.get("asset").and_then(|v| v.as_str()).unwrap_or("USDC");
                            let (success, res_data, err_msg) = match execute_base_gas_estimate(&state_reader, asset).await {
                                Ok(d) => (true, Some(d), None),
                                Err(e) => (false, None, Some(e)),
                            };

                            let _ = out_tx_reader.send(serde_json::json!({
                                "type": "action_result",
                                "data": {
                                    "action": "get_base_gas_estimate",
                                    "req_id": req_id,
                                    "success": success,
                                    "data": res_data,
                                    "error": err_msg,
                                }
                            }).to_string()).await;
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    let state_writer = state.clone();
    let mut last_block_height = block_height;
    let mut last_verified = verified as u64;
    let mut last_scanned = verified as u64;
    let mut last_target = target as u64;
    let mut last_p2p_peers = p2p_peers;

    let initial_utxos_fingerprint: u64 = raw_vault.utxos.iter().fold(0u64, |acc, u| {
        acc.wrapping_add(u.amount_atoms)
            .wrapping_add(u.height)
            .wrapping_add(u.vout as u64)
    });
    let mut initial_mempool_hasher = DefaultHasher::new();
    for tx in &mempool_txs {
        tx.txid.to_lowercase().hash(&mut initial_mempool_hasher);
    }
    let initial_mempool_fingerprint = initial_mempool_hasher.finish();

    let mut last_confirmed_atoms = raw_vault.confirmed_atoms;
    let mut last_unconfirmed_atoms = raw_vault.unconfirmed_atoms;
    let mut last_mempool_count = raw_vault.mempool_count;
    let mut last_utxos_count = raw_vault.utxos.len();
    let mut last_utxos_fingerprint = initial_utxos_fingerprint;
    let mut last_mempool_len = mempool_txs.len();
    let mut last_mempool_fingerprint = initial_mempool_fingerprint;
    let mut heartbeat_secs: u32 = 0;

    let mut writer_task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(250));
        loop {
            tokio::select! {
                Some(text) = out_rx.recv() => {
                    if sender.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Ok(ui_evt) = rx_ui.recv() => {
                    if sender.send(Message::Text(ui_evt.to_string().into())).await.is_err() {
                        break;
                    }
                }
                Ok(order) = rx_tx_events.recv() => {
                    let hash_hex = hex::encode(order.hash160);
                    let is_counterparty_fund = {
                        let orders = state_writer.orderbook.get_active_orders().await;
                        orders.iter().any(|o| o.hash160_hex == hash_hex && o.side == crate::swap::orderbook::OrderSide::SellTruForBaseUsdc)
                    };

                    let progress_frame = if is_counterparty_fund {
                        serde_json::json!({
                            "type": "swap_progress",
                            "data": {
                                "step": 3,
                                "status": 0,
                                "swap_id": order.swap_id,
                                "hash160": hash_hex,
                            }
                        })
                    } else {
                        serde_json::json!({
                            "type": "swap_progress",
                            "data": {
                                "step": 2,
                                "status": 0,
                                "swap_id": order.swap_id,
                                "hash160": hash_hex,
                            }
                        })
                    };
                    if sender.send(Message::Text(progress_frame.to_string().into())).await.is_err() {
                        break;
                    }
                }
                _ = ticker.tick() => {
                    let vf = state_writer.block_scanner.verified_height();
                    let sc = state_writer.block_scanner.scanned_height();
                    let tg = state_writer.block_scanner.target_height();
                    let p2p_tip = state_writer.tru_client.p2p_node.current_height() as u64;
                    let tg_effective = tg.max(p2p_tip);
                    if tg_effective > tg {
                        state_writer.block_scanner.update_target_height(tg_effective);
                    }
                    let block_height = (vf as u32).max(tg_effective as u32);
                    let block_height = if block_height > 0 {
                        block_height
                    } else {
                        state_writer.tru_client.get_block_count().await
                    };
                    let raw_v = fetch_raw_vault_status(&state_writer).await;
                    let p2p_peers = state_writer.tru_client.p2p_node.peer_count();

                    let utxos_fingerprint: u64 = raw_v.utxos.iter().fold(0u64, |acc, u| {
                        acc.wrapping_add(u.amount_atoms)
                            .wrapping_add(u.height)
                            .wrapping_add(u.vout as u64)
                    });

                    let init_changed = block_height != last_block_height
                        || vf != last_verified
                        || sc != last_scanned
                        || tg_effective != last_target
                        || p2p_peers != last_p2p_peers
                        || raw_v.confirmed_atoms != last_confirmed_atoms;

                    let vault_changed = raw_v.confirmed_atoms != last_confirmed_atoms
                        || raw_v.unconfirmed_atoms != last_unconfirmed_atoms
                        || raw_v.mempool_count != last_mempool_count
                        || raw_v.utxos.len() != last_utxos_count
                        || utxos_fingerprint != last_utxos_fingerprint;

                    let mempool_txs = state_writer.block_scanner.get_mempool().await;
                    let mut mempool_hasher = DefaultHasher::new();
                    for tx in &mempool_txs {
                        tx.txid.to_lowercase().hash(&mut mempool_hasher);
                    }
                    let mempool_fingerprint = mempool_hasher.finish();
                    let mempool_changed = mempool_txs.len() != last_mempool_len || mempool_fingerprint != last_mempool_fingerprint;

                    heartbeat_secs = heartbeat_secs.saturating_add(1);

                    // Send telemetry JSON immediately when chain state changed, or once every 30 ticks (~7.5s) as keepalive
                    if init_changed || heartbeat_secs >= 30 {
                        let init_data = serde_json::json!({
                            "type": "init",
                            "data": {
                                "block_height": block_height,
                                "confirmed_atoms": raw_v.confirmed_atoms,
                                "unconfirmed_atoms": raw_v.unconfirmed_atoms,
                                "vault_address": raw_v.address,
                                "p2p_peers": p2p_peers,
                                "verified_height": vf,
                                "target_height": tg_effective,
                            }
                        });
                        if sender.send(Message::Text(init_data.to_string().into())).await.is_err() {
                            break;
                        }

                        // Also sync swaps ledger immediately on block/scan update
                        let ledger_val = build_swaps_ledger_json(&state_writer).await;
                        let sw_frame = serde_json::json!({
                            "type": "swaps_ledger",
                            "data": ledger_val,
                        });
                        let _ = sender.send(Message::Text(sw_frame.to_string().into())).await;

                        last_block_height = block_height;
                        last_verified = vf;
                        last_scanned = sc;
                        last_target = tg_effective;
                        last_p2p_peers = p2p_peers;
                        heartbeat_secs = 0;
                    }

                    // Send vault status JSON ONLY when vault or UTXOs actually changed
                    if vault_changed {
                        let v_json = build_vault_status_json(&state_writer).await;
                        let v_msg = serde_json::json!({
                            "type": "vault",
                            "data": v_json,
                        });
                        if sender.send(Message::Text(v_msg.to_string().into())).await.is_err() {
                            break;
                        }
                        last_confirmed_atoms = raw_v.confirmed_atoms;
                        last_unconfirmed_atoms = raw_v.unconfirmed_atoms;
                        last_mempool_count = raw_v.mempool_count;
                        last_utxos_count = raw_v.utxos.len();
                        last_utxos_fingerprint = utxos_fingerprint;
                    }

                    // Send live mempool transactions JSON immediately when mempool contents change
                    if mempool_changed {
                        let m_msg = serde_json::json!({
                            "type": "mempool",
                            "data": mempool_txs,
                        });
                        if sender.send(Message::Text(m_msg.to_string().into())).await.is_err() {
                            break;
                        }
                        last_mempool_len = mempool_txs.len();
                        last_mempool_fingerprint = mempool_fingerprint;
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = (&mut reader_task) => writer_task.abort(),
        _ = (&mut writer_task) => reader_task.abort(),
    };

    state.tui.set_ws_clients(0).await;
    info!("WebSocket client disconnected");
}
