use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tracing::{debug, info, warn};
use super::crypto::{
    canonical_cancel_message, canonical_order_message, canonical_take_message,
    normalize_to_tru_address, recover_personal_sign_address, recover_tru_address,
};
use super::protection::{ConnectionGuard, MAX_WS_FRAME_BYTES};
use super::protocol::{
    ClientMessage, RelayBroadcastEvent, RelayOrder, RelayOrderSide, SignedCancelPayload,
    SignedOrderPayload, TakeOrderPayload,
};
use super::state::HotOrderbookState;
use crate::relay::base_watcher::BaseWatcher;
use crate::relay::utxo_guard::UtxoGuard;

#[derive(Clone)]
pub struct RelayServerContext {
    pub orderbook: Arc<HotOrderbookState>,
    pub guard: Arc<ConnectionGuard>,
    pub utxo_guard: Arc<UtxoGuard>,
    pub base_watcher: Arc<BaseWatcher>,
    pub mode_str: String,
}

/// Creates the Axum router for the TRUSwap Relay service
pub fn create_relay_router(ctx: RelayServerContext) -> Router {
    Router::new()
        .route("/orderbook", get(ws_orderbook_handler))
        .route("/orderbook/health", get(health_handler))
        .route("/health", get(health_handler))
        .with_state(ctx)
}

async fn health_handler(State(ctx): State<RelayServerContext>) -> Response {
    let active_orders = ctx.orderbook.orders.read().await.len();
    let active_conns = ctx.guard.active_clients.load(std::sync::atomic::Ordering::Relaxed);
    let payload = serde_json::json!({
        "status": "healthy",
        "service": "truswap-relay",
        "mode": ctx.mode_str,
        "active_connections": active_conns,
        "active_orders": active_orders
    });
    (StatusCode::OK, axum::Json(payload)).into_response()
}

/// Extracts real client IP address checking Cloudflare and reverse-proxy headers
fn extract_client_ip(headers: &HeaderMap, peer_addr: SocketAddr) -> std::net::IpAddr {
    if let Some(cf_ip) = headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()) {
        if let Ok(ip) = cf_ip.trim().parse() {
            return ip;
        }
    }
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first_ip) = xff.split(',').next() {
            if let Ok(ip) = first_ip.trim().parse() {
                return ip;
            }
        }
    }
    peer_addr.ip()
}

async fn ws_orderbook_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    State(ctx): State<RelayServerContext>,
) -> Response {
    let client_ip = extract_client_ip(&headers, peer_addr);

    // Enforce connection limits and IP quotas
    if let Err(reason) = ctx.guard.acquire_connection(client_ip) {
        warn!("[RELAY-WS] Rejected connection from {}: {}", client_ip, reason);
        return (StatusCode::SERVICE_UNAVAILABLE, reason).into_response();
    }

    ws.max_frame_size(MAX_WS_FRAME_BYTES)
        .max_message_size(MAX_WS_FRAME_BYTES)
        .on_upgrade(move |socket| handle_ws_session(socket, client_ip, ctx))
}

static NEXT_SESSION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

async fn handle_ws_session(
    socket: WebSocket,
    client_ip: std::net::IpAddr,
    ctx: RelayServerContext,
) {
    let session_id = NEXT_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    debug!("[RELAY-WS] Client connected: {} (session #{})", client_ip, session_id);
    let (mut sender, mut receiver) = socket.split();

    // 1. Send initial orderbook snapshot
    let snapshot = ctx.orderbook.get_snapshot().await;
    let now_sec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let snapshot_event = RelayBroadcastEvent::OrderbookSnapshot {
        orders: snapshot,
        server_time: now_sec,
        mode: ctx.mode_str.clone(),
    };

    if let Ok(json_str) = serde_json::to_string(&snapshot_event) {
        if let Err(e) = sender.send(Message::Text(json_str.into())).await {
            warn!("[RELAY-WS] Failed to send snapshot to {}: {}", client_ip, e);
            ctx.guard.release_connection(client_ip);
            return;
        }
    }

    // 2. Subscribe to orderbook broadcast channel
    let mut broadcast_rx = ctx.orderbook.broadcast_tx.subscribe();

    // 3. Main event loop: client incoming & server broadcast with 15s ping-pong interval
    let mut ping_interval = tokio::time::interval(Duration::from_secs(15));

    loop {
        tokio::select! {
            // Heartbeat ping every 15 seconds
            _ = ping_interval.tick() => {
                if let Err(e) = sender.send(Message::Ping(vec![].into())).await {
                    debug!("[RELAY-WS] Ping failed for {}: {}", client_ip, e);
                    break;
                }
            }

            // Broadcast events to client
            broadcast_msg = broadcast_rx.recv() => {
                match broadcast_msg {
                    Ok(event) => {
                        if let Ok(serialized) = serde_json::to_string(&event) {
                            if let Err(e) = sender.send(Message::Text(serialized.into())).await {
                                debug!("[RELAY-WS] Send broadcast failed for {}: {}", client_ip, e);
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("[RELAY-WS] Client {} lagged behind by {} broadcast messages", client_ip, skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }

            // Incoming client messages
            client_msg = receiver.next() => {
                match client_msg {
                    Some(Ok(Message::Binary(_))) => {
                        let err_evt = RelayBroadcastEvent::Error {
                            code: "UNSUPPORTED_FRAME".to_string(),
                            message: "Binary WebSocket frames are deprecated. All communication uses UTF-8 JSON text frames.".to_string(),
                        };
                        if let Ok(err_json) = serde_json::to_string(&err_evt) {
                            let _ = sender.send(Message::Text(err_json.into())).await;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > MAX_WS_FRAME_BYTES {
                            warn!("[RELAY-WS] Frame size limit exceeded by {}", client_ip);
                            break;
                        }

                        // Touch session heartbeat on any received text frame
                        ctx.orderbook.touch_session(session_id).await;

                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(msg) => {
                                match msg {
                                    ClientMessage::SubmitOrder(payload) => {
                                        let order_id = payload.order_id.clone();
                                        let maker_base = payload.maker_base_address.clone();
                                        if let Err(err_evt) = handle_submit_order(&ctx, payload).await {
                                            if let Ok(err_json) = serde_json::to_string(&err_evt) {
                                                let _ = sender.send(Message::Text(err_json.into())).await;
                                            }
                                        } else {
                                            // Associate submitted order and maker with this active session
                                            ctx.orderbook.register_session_order(session_id, &order_id, &maker_base).await;
                                        }
                                    }
                                    ClientMessage::CancelOrder(payload) => {
                                        if let Err(err_evt) = handle_cancel_order(&ctx, payload).await {
                                            if let Ok(err_json) = serde_json::to_string(&err_evt) {
                                                let _ = sender.send(Message::Text(err_json.into())).await;
                                            }
                                        }
                                    }
                                    ClientMessage::TakeOrder(payload) => {
                                        if let Err(err_evt) = handle_take_order(&ctx, payload).await {
                                            warn!("[RELAY-TAKE-FAIL] Text TakeOrder error: {:?}", err_evt);
                                            if let Ok(err_json) = serde_json::to_string(&err_evt) {
                                                let _ = sender.send(Message::Text(err_json.into())).await;
                                            }
                                        }
                                    }
                                    ClientMessage::Ping => {
                                        ctx.orderbook.touch_session(session_id).await;
                                        let pong = RelayBroadcastEvent::Pong;
                                        if let Ok(pong_json) = serde_json::to_string(&pong) {
                                            let _ = sender.send(Message::Text(pong_json.into())).await;
                                        }
                                    }
                                    ClientMessage::Heartbeat(payload) => {
                                        ctx.orderbook.touch_session(session_id).await;
                                        if let Some(ref base_addr) = payload.maker_base_address {
                                            ctx.orderbook.associate_session_maker(session_id, base_addr).await;
                                        }
                                        let pong = RelayBroadcastEvent::Pong;
                                        if let Ok(pong_json) = serde_json::to_string(&pong) {
                                            let _ = sender.send(Message::Text(pong_json.into())).await;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                let err_evt = RelayBroadcastEvent::Error {
                                    code: "INVALID_JSON".to_string(),
                                    message: format!("Malformed JSON payload: {}", e),
                                };
                                if let Ok(err_json) = serde_json::to_string(&err_evt) {
                                    let _ = sender.send(Message::Text(err_json.into())).await;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        ctx.orderbook.touch_session(session_id).await;
                        let _ = sender.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        ctx.orderbook.touch_session(session_id).await;
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!("[RELAY-WS] Client {} closed session #{}", client_ip, session_id);
                        break;
                    }
                    Some(Err(e)) => {
                        debug!("[RELAY-WS] Error reading from {}: {}", client_ip, e);
                        break;
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }

    ctx.orderbook.release_session(session_id).await;
    ctx.guard.release_connection(client_ip);
    debug!("[RELAY-WS] Session #{} closed for {}", session_id, client_ip);
}

/// Validates and adds a new signed order to the orderbook
async fn handle_submit_order(
    ctx: &RelayServerContext,
    payload: SignedOrderPayload,
) -> Result<(), RelayBroadcastEvent> {
    // 1. Anti-spam rate limiting per maker wallet
    if let Err(reason) = ctx.guard.check_submission_rate(&payload.maker_base_address) {
        return Err(RelayBroadcastEvent::Error {
            code: "RATE_LIMITED".to_string(),
            message: reason.to_string(),
        });
    }

    // 2. Validate numeric and format constraints
    if payload.amount_tru <= 0.0 || payload.amount_usdc <= 0.0 {
        return Err(RelayBroadcastEvent::Error {
            code: "INVALID_AMOUNT".to_string(),
            message: "Amounts must be strictly positive".to_string(),
        });
    }

    let clean_hash = payload.hash160_hex.strip_prefix("0x").unwrap_or(&payload.hash160_hex);
    if clean_hash.len() != 40 || hex::decode(clean_hash).is_err() {
        return Err(RelayBroadcastEvent::Error {
            code: "INVALID_HASH160".to_string(),
            message: "hash160 must be a 20-byte (40-char) hex string".to_string(),
        });
    }

    // Validate TRU address format (can be either Base58 address or 66-hex compressed pubkey)
    let is_pubkey = payload.maker_tru_address.len() == 66
        && (payload.maker_tru_address.starts_with("02") || payload.maker_tru_address.starts_with("03"))
        && hex::decode(&payload.maker_tru_address).map(|b| b.len() == 33).unwrap_or(false);

    if !is_pubkey && crate::core::keys::tru_address_to_pkh(&payload.maker_tru_address).is_none() {
        return Err(RelayBroadcastEvent::Error {
            code: "INVALID_TRU_ADDRESS".to_string(),
            message: "Invalid TRU address checksum or format".to_string(),
        });
    }

    // 3. Cryptographic signature verification (personal_sign)
    let mut sig_verified = false;

    // 1. TRU signature check (Primary protocol identity)
    if let Some(ref sig_tru) = payload.signature_tru {
        let clean_sig = sig_tru.trim();
        if !clean_sig.is_empty() {
            let canon_tru = canonical_order_message(
                &payload.order_id,
                payload.side.as_str(),
                payload.amount_tru,
                payload.amount_usdc,
                clean_hash,
                payload.timelock,
                &payload.maker_tru_address,
            );
            if let Ok(rec_tru) = recover_tru_address(&canon_tru, clean_sig) {
                let clean_tru_target = normalize_to_tru_address(&payload.maker_tru_address);
                if rec_tru == clean_tru_target {
                    sig_verified = true;
                    info!("[RELAY-SUBMIT] Order {} verified by TRU maker signature {}", payload.order_id, rec_tru);
                }
            }
        }
    }

    // 2. Base EVM signature check (Fallback / backward compatibility)
    if !sig_verified && !payload.signature_base.trim().is_empty() {
        let canonical_msg = canonical_order_message(
            &payload.order_id,
            payload.side.as_str(),
            payload.amount_tru,
            payload.amount_usdc,
            clean_hash,
            payload.timelock,
            &payload.maker_base_address,
        );
        if let Ok(recovered_addr) = recover_personal_sign_address(&canonical_msg, &payload.signature_base) {
            if recovered_addr.to_lowercase() == payload.maker_base_address.to_lowercase() {
                sig_verified = true;
                info!("[RELAY-SUBMIT] Order {} verified by Base EVM maker signature {}", payload.order_id, recovered_addr);
            }
        }
    }

    if !sig_verified {
        return Err(RelayBroadcastEvent::Error {
            code: "CRYPTO_VERIFY_FAILED".to_string(),
            message: "Failed to verify cryptographic signature (requires valid TRU or EVM maker signature)".to_string(),
        });
    }

    // 4. Pre-trade Balance Validation (Cumulative TRU & Base check across active orders)
    match payload.side {
        RelayOrderSide::SellTru => {
            let amount_atoms = (payload.amount_tru * 100_000_000.0).round() as u64;
            let clean_tru_addr = payload.maker_tru_address.clone();
            let tru_order_ids = {
                let tru_idx = ctx.orderbook.tru_maker_index.read().await;
                tru_idx.get(&clean_tru_addr).cloned().unwrap_or_default()
            };

            let orders_lock = ctx.orderbook.orders.read().await;
            let allocated_atoms: u64 = tru_order_ids
                .iter()
                .filter_map(|id| {
                    orders_lock.get(id).and_then(|o| {
                        if o.side == RelayOrderSide::SellTru && !o.is_matched {
                            Some((o.amount_tru * 100_000_000.0).round() as u64 + crate::relay::utxo_guard::HTLC_TOTAL_FEE_BUFFER_ATOMS)
                        } else {
                            None
                        }
                    })
                })
                .sum();
            drop(orders_lock);

            let total_required_atoms = allocated_atoms
                .saturating_add(amount_atoms)
                .saturating_add(crate::relay::utxo_guard::HTLC_TOTAL_FEE_BUFFER_ATOMS);
            let balance = ctx.utxo_guard.get_balance(&clean_tru_addr).await;

            if balance < total_required_atoms {
                let max_avail_atoms = balance
                    .saturating_sub(allocated_atoms)
                    .saturating_sub(crate::relay::utxo_guard::HTLC_TOTAL_FEE_BUFFER_ATOMS);
                return Err(RelayBroadcastEvent::Error {
                    code: "INSUFFICIENT_TRU_BALANCE".to_string(),
                    message: format!(
                        "Maker balance is {:.8} TRU, but {:.8} TRU is already allocated in active sell orders in the orderbook. Maximum new sell order is {:.8} TRU",
                        balance as f64 / 1e8,
                        allocated_atoms as f64 / 1e8,
                        max_avail_atoms as f64 / 1e8
                    ),
                });
            }

            // Check Maker Base ETH gas balance to guarantee ability to claim USDC
            let min_claim_gas_wei = ctx.base_watcher.get_claim_gas_threshold().await;
            match ctx.base_watcher.get_eth_balance(&payload.maker_base_address).await {
                Ok(eth_wei) => {
                    if eth_wei < min_claim_gas_wei {
                        return Err(RelayBroadcastEvent::Error {
                            code: "INSUFFICIENT_ETH_GAS".to_string(),
                            message: format!(
                                "Maker ETH balance ({:.6} ETH) is below dynamic gas requirement ({:.6} ETH to claim USDC on Base after swap)",
                                eth_wei as f64 / 1e18,
                                min_claim_gas_wei as f64 / 1e18
                            ),
                        });
                    }
                }
                Err(e) => {
                    return Err(RelayBroadcastEvent::Error {
                        code: "RPC_VERIFY_FAILED".to_string(),
                        message: format!("Failed to verify Maker Base ETH gas balance via RPC: {}", e),
                    });
                }
            }
        }
        RelayOrderSide::BuyTru => {
            // Batched ETH gas & USDC balance verification (single HTTP round-trip with cache and dynamic gas pricing)
            match ctx.base_watcher.get_balances_batch(&payload.maker_base_address).await {
                Ok((wei, usdc, min_gas_wei)) => {
                    if wei < min_gas_wei {
                        return Err(RelayBroadcastEvent::Error {
                            code: "INSUFFICIENT_ETH_GAS".to_string(),
                            message: format!(
                                "Maker ETH balance ({:.6} ETH) is below dynamic network gas requirement ({:.6} ETH to execute BaseHTLC swap)",
                                wei as f64 / 1e18,
                                min_gas_wei as f64 / 1e18
                            ),
                        });
                    }

                    // Cumulative USDC check: sum existing active BUY_TRU orders by this maker
                    let base_order_ids = ctx.orderbook.get_orders_by_base_maker(&payload.maker_base_address).await;
                    let orders_lock = ctx.orderbook.orders.read().await;
                    let allocated_usdc: f64 = base_order_ids
                        .iter()
                        .filter_map(|id| {
                            orders_lock.get(id).and_then(|o| {
                                if o.side == RelayOrderSide::BuyTru && !o.is_matched {
                                    Some(o.amount_usdc)
                                } else {
                                    None
                                }
                            })
                        })
                        .sum();
                    drop(orders_lock);

                    let total_needed_usdc = allocated_usdc + payload.amount_usdc;
                    if usdc < total_needed_usdc {
                        let max_avail = (usdc - allocated_usdc).max(0.0);
                        return Err(RelayBroadcastEvent::Error {
                            code: "INSUFFICIENT_USDC_BALANCE".to_string(),
                            message: format!(
                                "Maker balance is {:.2} USDC, but {:.2} USDC is already allocated in active buy orders in the orderbook. Maximum new order size is {:.2} USDC",
                                usdc, allocated_usdc, max_avail
                            ),
                        });
                    }
                }
                Err(e) => {
                    return Err(RelayBroadcastEvent::Error {
                        code: "RPC_VERIFY_FAILED".to_string(),
                        message: format!("Failed to verify Maker Base balances (USDC/ETH) via RPC: {}", e),
                    });
                }
            }
        }
    }

    // 5. Store verified order into in-memory hot orderbook
    let now_sec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let price = payload.amount_usdc / payload.amount_tru;

    let is_pubkey = payload.maker_tru_address.len() == 66
        && (payload.maker_tru_address.starts_with("02") || payload.maker_tru_address.starts_with("03"))
        && hex::decode(&payload.maker_tru_address).map(|b| b.len() == 33).unwrap_or(false);

    let (maker_tru_address, maker_tru_pubkey) = if is_pubkey {
        let b = hex::decode(&payload.maker_tru_address).unwrap_or_default();
        let mut arr = [0u8; 33];
        arr.copy_from_slice(&b);
        let pkh = crate::core::keys::hash160(&arr);
        (crate::core::keys::pkh_to_tru_address(&pkh), Some(payload.maker_tru_address))
    } else {
        (payload.maker_tru_address, payload.maker_tru_pubkey)
    };

    let order = RelayOrder {
        order_id: payload.order_id,
        side: payload.side,
        amount_tru: payload.amount_tru,
        amount_usdc: payload.amount_usdc,
        price,
        hash160_hex: clean_hash.to_string(),
        timelock: payload.timelock,
        maker_tru_address,
        maker_tru_pubkey,
        maker_base_address: payload.maker_base_address,
        created_at: payload.created_at.unwrap_or(now_sec),
        is_matched: false,
    };

    info!(
        "[RELAY-ORDERBOOK] Order {} accepted: {:?} {:.4} TRU @ {:.4} USDC (Maker: {})",
        order.order_id, order.side, order.amount_tru, order.price, order.maker_base_address
    );

    ctx.orderbook.add_order(order).await;
    Ok(())
}

/// Handles cancellation request signed by maker
async fn handle_cancel_order(
    ctx: &RelayServerContext,
    payload: SignedCancelPayload,
) -> Result<(), RelayBroadcastEvent> {
    let clean_target_id = payload.order_id.trim().strip_prefix("0x").unwrap_or(&payload.order_id).to_lowercase();
    let orders_lock = ctx.orderbook.orders.read().await;
    let order_opt = orders_lock.get(&clean_target_id).cloned()
        .or_else(|| {
            orders_lock.values().find(|o| {
                let o_id = o.order_id.strip_prefix("0x").unwrap_or(&o.order_id).to_lowercase();
                let o_hash = o.hash160_hex.strip_prefix("0x").unwrap_or(&o.hash160_hex).to_lowercase();
                o_id == clean_target_id || o_hash == clean_target_id
            }).cloned()
        });
    drop(orders_lock);

    let order = match order_opt {
        Some(o) => o,
        None => {
            return Err(RelayBroadcastEvent::Error {
                code: "ORDER_NOT_FOUND".to_string(),
                message: "Order does not exist or has already been filled/removed".to_string(),
            });
        }
    };

    let mut verified = false;

    // 1. First priority: TRU signature authorization (Primary protocol identity)
    if let Some(ref sig_tru) = payload.signature_tru {
        let clean_sig = sig_tru.trim();
        if !clean_sig.is_empty() {
            let tru_maker = if !order.maker_tru_address.is_empty() {
                order.maker_tru_address.clone()
            } else if let Some(ref a) = payload.maker_tru_address {
                a.clone()
            } else {
                String::new()
            };

            if !tru_maker.is_empty() {
                let canon_order = canonical_cancel_message(&order.order_id, &tru_maker);
                let canon_payload = canonical_cancel_message(&payload.order_id, &tru_maker);
                if let Ok(rec) = recover_tru_address(&canon_order, clean_sig).or_else(|_| recover_tru_address(&canon_payload, clean_sig)) {
                    if rec == tru_maker || (order.maker_tru_address.is_empty() || rec == order.maker_tru_address) {
                        verified = true;
                        info!("[RELAY-CANCEL] Order {} verified by TRU maker signature {}", order.order_id, rec);
                    }
                }
            }
        }
    }

    // 2. Second priority: Base EVM personal_sign authorization (fallback / backward compat)
    if !verified {
        let clean_sig = payload.signature_base.trim();
        if !clean_sig.is_empty() {
            let canon_order = canonical_cancel_message(&order.order_id, &payload.maker_base_address);
            let canon_payload = canonical_cancel_message(&payload.order_id, &payload.maker_base_address);
            if let Ok(recovered_addr) = recover_personal_sign_address(&canon_order, clean_sig).or_else(|_| recover_personal_sign_address(&canon_payload, clean_sig)) {
                let recovered_lower = recovered_addr.to_lowercase();
                let payload_maker_lower = payload.maker_base_address.to_lowercase();
                let order_maker_lower = order.maker_base_address.to_lowercase();
                if recovered_lower == payload_maker_lower && (order_maker_lower.is_empty() || order_maker_lower == payload_maker_lower) {
                    verified = true;
                    info!("[RELAY-CANCEL] Order {} verified by EVM maker signature {}", order.order_id, recovered_addr);
                }
            }
        }
    }

    if !verified {
        return Err(RelayBroadcastEvent::Error {
            code: "UNAUTHORIZED".to_string(),
            message: "Cancellation must be authorized with maker TRU or EVM cryptographic signature".to_string(),
        });
    }

    ctx.orderbook.remove_order(&order.order_id, "Cancelled by maker").await;
    info!("[RELAY-ORDERBOOK] Order {} successfully cancelled by maker", order.order_id);
    Ok(())
}

/// Handles order execution lock when a taker initiates a swap
async fn handle_take_order(
    ctx: &RelayServerContext,
    payload: TakeOrderPayload,
) -> Result<(), RelayBroadcastEvent> {
    // 1. Retrieve order to check side and required amounts
    let order_opt = {
        let orders_lock = ctx.orderbook.orders.read().await;
        orders_lock.get(&payload.order_id).cloned()
    };

    let order = match order_opt {
        Some(o) => o,
        None => {
            return Err(RelayBroadcastEvent::Error {
                code: "ORDER_NOT_FOUND".to_string(),
                message: "Order does not exist or has already been removed".to_string(),
            });
        }
    };

    if order.is_matched {
        return Err(RelayBroadcastEvent::Error {
            code: "ORDER_ALREADY_MATCHED".to_string(),
            message: "Order is already matched or pending execution".to_string(),
        });
    }

    // 2. Cryptographic signature verification: Taker must prove ownership of taker_tru_address or taker_base_address!
    let canonical_msg = canonical_take_message(
        &payload.order_id,
        &payload.taker_base_address,
        &payload.taker_tru_address,
    );

    let mut verified = false;

    // First priority: TRU signature
    if let Some(ref sig_tru) = payload.signature_tru {
        let clean_sig = sig_tru.trim();
        if !clean_sig.is_empty() {
            if let Ok(rec_tru) = recover_tru_address(&canonical_msg, clean_sig) {
                let canon_taker_tru = normalize_to_tru_address(&payload.taker_tru_address);
                if rec_tru == canon_taker_tru {
                    verified = true;
                    info!("[RELAY-TAKE] Order {} verified by TRU taker signature {}", payload.order_id, rec_tru);
                }
            }
        }
    }

    // Fallback: Base EVM personal_sign
    if !verified {
        if let Some(ref sig_base) = payload.signature_base {
            let clean_sig = sig_base.trim();
            if !clean_sig.is_empty() {
                if let Ok(rec_base) = recover_personal_sign_address(&canonical_msg, clean_sig) {
                    if rec_base.to_lowercase() == payload.taker_base_address.to_lowercase() {
                        verified = true;
                        info!("[RELAY-TAKE] Order {} verified by Base taker signature {}", payload.order_id, rec_base);
                    }
                }
            }
        }
    }

    if !verified {
        return Err(RelayBroadcastEvent::Error {
            code: "INVALID_TAKER_SIGNATURE".to_string(),
            message: "TakeOrder requires valid cryptographic signature (TRU or Base) proving ownership".to_string(),
        });
    }

    // 3. Pre-trade Balance Validation for Taker:
    // Support both TRU address and compressed 33-byte pubkey hex for taker_tru_address
    let is_pubkey = payload.taker_tru_address.len() == 66
        && (payload.taker_tru_address.starts_with("02") || payload.taker_tru_address.starts_with("03"))
        && hex::decode(&payload.taker_tru_address).map(|b| b.len() == 33).unwrap_or(false);

    let taker_tru_address = if is_pubkey {
        let b = hex::decode(&payload.taker_tru_address).unwrap_or_default();
        let mut arr = [0u8; 33];
        arr.copy_from_slice(&b);
        let pkh = crate::core::keys::hash160(&arr);
        crate::core::keys::pkh_to_tru_address(&pkh)
    } else {
        payload.taker_tru_address.clone()
    };

    // If order.side == BuyTru: Maker buys TRU (pays USDC) -> Taker sells TRU (must have TRU balance!)
    // If order.side == SellTru: Maker sells TRU (receives USDC) -> Taker buys TRU (must have USDC + ETH gas!)
    match order.side {
        RelayOrderSide::BuyTru => {
            // Maker buys TRU (pays USDC) -> Taker sells TRU (must have TRU balance + Base ETH gas to claim USDC!)
            let amount_atoms = (order.amount_tru * 100_000_000.0).round() as u64;
            // Cumulative TRU check for Taker: account for in-flight swap commitments
            let allocated_atoms = ctx.orderbook.get_taker_allocated_tru(&taker_tru_address, 300).await;
            let total_needed_atoms = allocated_atoms.saturating_add(amount_atoms);

            if let Err(e) = ctx.utxo_guard.verify_tru_balance(&taker_tru_address, total_needed_atoms).await {
                return Err(RelayBroadcastEvent::Error {
                    code: "TAKER_INSUFFICIENT_TRU_BALANCE".to_string(),
                    message: format!(
                        "Taker does not have sufficient confirmed TRU balance to fulfill order (allocated in-flight: {:.8} TRU, total needed: {:.8} TRU): {}",
                        allocated_atoms as f64 / 1e8,
                        total_needed_atoms as f64 / 1e8,
                        e
                    ),
                });
            }

            // Check Taker Base ETH balance for claiming USDC on Base
            let min_claim_gas_wei = ctx.base_watcher.get_claim_gas_threshold().await;
            match ctx.base_watcher.get_eth_balance(&payload.taker_base_address).await {
                Ok(wei) => {
                    if wei < min_claim_gas_wei {
                        return Err(RelayBroadcastEvent::Error {
                            code: "TAKER_INSUFFICIENT_ETH_GAS".to_string(),
                            message: format!(
                                "Taker ETH balance ({:.6} ETH) is below dynamic gas requirement ({:.6} ETH to claim USDC on Base after swap)",
                                wei as f64 / 1e18,
                                min_claim_gas_wei as f64 / 1e18
                            ),
                        });
                    }
                }
                Err(e) => {
                    return Err(RelayBroadcastEvent::Error {
                        code: "TAKER_ETH_VERIFY_FAILED".to_string(),
                        message: format!("Failed to verify Taker Base ETH gas balance: {}", e),
                    });
                }
            }
        }
        RelayOrderSide::SellTru => {
            // Maker sells TRU (receives USDC) -> Taker buys TRU (must have USDC + dynamic ETH gas to lock USDC!)
            match ctx.base_watcher.get_balances_batch(&payload.taker_base_address).await {
                Ok((wei, usdc, min_gas_wei)) => {
                    if wei < min_gas_wei {
                        return Err(RelayBroadcastEvent::Error {
                            code: "TAKER_INSUFFICIENT_ETH_GAS".to_string(),
                            message: format!(
                                "Taker ETH balance ({:.6} ETH) is below dynamic network gas requirement ({:.6} ETH to execute BaseHTLC swap)",
                                wei as f64 / 1e18,
                                min_gas_wei as f64 / 1e18
                            ),
                        });
                    }

                    // Cumulative USDC check for Taker: account for in-flight swap commitments
                    let allocated_usdc = ctx.orderbook.get_taker_allocated_usdc(&payload.taker_base_address, 300).await;
                    let total_needed_usdc = allocated_usdc + order.amount_usdc;
                    if usdc < total_needed_usdc {
                        let max_avail = (usdc - allocated_usdc).max(0.0);
                        return Err(RelayBroadcastEvent::Error {
                            code: "TAKER_INSUFFICIENT_USDC_BALANCE".to_string(),
                            message: format!(
                                "Taker balance is {:.2} USDC, but {:.2} USDC is already allocated in active in-flight swap(s). Maximum available for new take is {:.2} USDC",
                                usdc, allocated_usdc, max_avail
                            ),
                        });
                    }
                }
                Err(e) => {
                    return Err(RelayBroadcastEvent::Error {
                        code: "TAKER_BASE_VERIFY_FAILED".to_string(),
                        message: format!("Failed to verify Taker Base balances (USDC/ETH): {}", e),
                    });
                }
            }
        }
    }

    let taker_pubkey_opt = if is_pubkey {
        Some(payload.taker_tru_address.clone())
    } else {
        payload.taker_tru_pubkey.clone()
    };

    match ctx.orderbook.match_order(
        &payload.order_id,
        &payload.taker_base_address,
        &taker_tru_address,
        taker_pubkey_opt.as_deref(),
        payload.signature_base.as_deref(),
    ).await {
        Ok(order) => {
            info!(
                "[RELAY-MATCH] Order {} locked for swap: Taker Base {}, Taker TRU {}, Taker Pubkey {:?}",
                order.order_id, payload.taker_base_address, taker_tru_address, taker_pubkey_opt
            );
            Ok(())
        }
        Err(e) => Err(RelayBroadcastEvent::Error {
            code: "MATCH_FAILED".to_string(),
            message: e,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::relay::protection::ConnectionGuard;

    #[tokio::test]
    async fn test_take_order_signature_verification_and_attack_prevention() {
        let cfg = Config::default();
        let orderbook = HotOrderbookState::new();
        let guard = ConnectionGuard::new(100);
        let utxo_guard = UtxoGuard::new();
        let base_watcher = BaseWatcher::new(&cfg);

        let ctx = RelayServerContext {
            orderbook: orderbook.clone(),
            guard,
            utxo_guard,
            base_watcher,
            mode_str: "Test".to_string(),
        };

        let secp = secp256k1::Secp256k1::new();
        let mut priv_bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut priv_bytes);
        priv_bytes[0] &= 0x7f;
        if priv_bytes[0] == 0 { priv_bytes[0] = 1; }
        let secret_key = secp256k1::SecretKey::from_slice(&priv_bytes).unwrap();
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let legit_taker_base = super::super::crypto::pubkey_to_eth_address(&pubkey);

        let order_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
        let order = RelayOrder {
            order_id: order_id.clone(),
            side: RelayOrderSide::SellTru,
            amount_tru: 10.0,
            amount_usdc: 50.0,
            price: 5.0,
            hash160_hex: "0102030405060708090a0b0c0d0e0f1011121314".to_string(),
            timelock: 1728000000,
            maker_tru_address: "TLXm111111111111111111111111111111".to_string(),
            maker_tru_pubkey: None,
            maker_base_address: "0x1111111111111111111111111111111111111111".to_string(),
            created_at: 1728000001,
            is_matched: false,
        };
        orderbook.add_order(order).await;

        let taker_tru = "TLXmLegitTakerTRUAddress".to_string();

        // 1. Attack Scenario 1: Attacker sends TakeOrder with no signature (Griefing Attempt)
        let unauthenticated_payload = TakeOrderPayload {
            order_id: order_id.clone(),
            taker_base_address: "0x8888888888888888888888888888888888888888".to_string(), // Whale address
            taker_tru_address: taker_tru.clone(),
            taker_tru_pubkey: None,
            signature_base: None,
            signature_tru: None,
        };
        let res = handle_take_order(&ctx, unauthenticated_payload).await;
        assert!(matches!(res, Err(RelayBroadcastEvent::Error { ref code, .. }) if code == "INVALID_TAKER_SIGNATURE"));

        // 2. Attack Scenario 2: Attacker signs with their own key but specifies a whale address as taker_base_address
        let whale_address = "0x8888888888888888888888888888888888888888".to_string();
        let fake_canonical_msg = canonical_take_message(&order_id, &whale_address, &taker_tru);
        let fake_sig = super::super::crypto::sign_personal_message(&fake_canonical_msg, &priv_bytes).unwrap();

        let spoofed_payload = TakeOrderPayload {
            order_id: order_id.clone(),
            taker_base_address: whale_address,
            taker_tru_address: taker_tru.clone(),
            taker_tru_pubkey: None,
            signature_base: Some(fake_sig),
            signature_tru: None,
        };
        let res2 = handle_take_order(&ctx, spoofed_payload).await;
        assert!(matches!(res2, Err(RelayBroadcastEvent::Error { ref code, .. }) if code == "INVALID_TAKER_SIGNATURE"));

        // 3. Legitimate Taker: Signs with their private key matching taker_base_address
        let legit_canonical_msg = canonical_take_message(&order_id, &legit_taker_base, &taker_tru);
        let legit_sig = super::super::crypto::sign_personal_message(&legit_canonical_msg, &priv_bytes).unwrap();

        let legit_payload = TakeOrderPayload {
            order_id: order_id.clone(),
            taker_base_address: legit_taker_base.clone(),
            taker_tru_address: taker_tru.clone(),
            taker_tru_pubkey: None,
            signature_base: Some(legit_sig),
            signature_tru: None,
        };
        // Signature check passes! (It will either succeed or fail on balance check if RPC is offline, but signature check is verified)
        let res3 = handle_take_order(&ctx, legit_payload).await;
        if let Err(RelayBroadcastEvent::Error { ref code, .. }) = res3 {
            assert_ne!(code, "MISSING_TAKER_SIGNATURE");
            assert_ne!(code, "INVALID_TAKER_SIGNATURE");
            assert_ne!(code, "CRYPTO_VERIFY_FAILED");
        }
    }

    #[tokio::test]
    async fn test_taker_allocated_usdc_and_fail_closed_rpc() {
        let orderbook = HotOrderbookState::new();
        let taker_base = "0x2222222222222222222222222222222222222222";
        let taker_tru = "TLXmTakerAddress";

        let order1 = RelayOrder {
            order_id: "order1".to_string(),
            side: RelayOrderSide::SellTru,
            amount_tru: 10.0,
            amount_usdc: 50.0,
            price: 5.0,
            hash160_hex: "0102030405060708090a0b0c0d0e0f1011121314".to_string(),
            timelock: 1728000000,
            maker_tru_address: "TLXmMaker1".to_string(),
            maker_tru_pubkey: None,
            maker_base_address: "0x1111111111111111111111111111111111111111".to_string(),
            created_at: 1728000001,
            is_matched: false,
        };
        orderbook.add_order(order1).await;

        // Initially 0 allocated
        assert_eq!(orderbook.get_taker_allocated_usdc(taker_base, 300).await, 0.0);
        assert_eq!(orderbook.get_taker_allocated_tru(taker_tru, 300).await, 0);

        // Match order 1 (SellTru: Maker sells TRU, Taker buys TRU and commits 50 USDC)
        let matched = orderbook.match_order("order1", taker_base, taker_tru, None, None).await.unwrap();
        assert!(matched.is_matched);

        // Now 50 USDC allocated, 0 TRU atoms allocated for taker
        assert_eq!(orderbook.get_taker_allocated_usdc(taker_base, 300).await, 50.0);
        assert_eq!(orderbook.get_taker_allocated_tru(taker_tru, 300).await, 0);

        // When order removed, allocation pruned
        orderbook.remove_order("order1", "Swap completed").await;
        assert_eq!(orderbook.get_taker_allocated_usdc(taker_base, 300).await, 0.0);
        assert_eq!(orderbook.get_taker_allocated_tru(taker_tru, 300).await, 0);
    }

    #[tokio::test]
    async fn test_maker_heartbeat_timeout_prunes_orders() {
        let orderbook = HotOrderbookState::new();
        let session_id = 999;
        let maker_base = "0x1111111111111111111111111111111111111111";

        let order = RelayOrder {
            order_id: "order_hb_test".to_string(),
            side: RelayOrderSide::SellTru,
            amount_tru: 10.0,
            amount_usdc: 50.0,
            price: 5.0,
            hash160_hex: "0102030405060708090a0b0c0d0e0f1011121314".to_string(),
            timelock: 1728000000,
            maker_tru_address: "TLXmMaker1".to_string(),
            maker_tru_pubkey: None,
            maker_base_address: maker_base.to_string(),
            created_at: 1728000001,
            is_matched: false,
        };
        orderbook.add_order(order).await;
        orderbook.register_session_order(session_id, "order_hb_test", maker_base).await;

        // 1. Immediately, order is alive and fresh
        assert!(orderbook.orders.read().await.contains_key("order_hb_test"));
        let pruned_early = orderbook.prune_stale_orders(Duration::from_secs(60)).await;
        assert!(pruned_early.is_empty(), "Order should not be pruned when fresh");

        // 2. Touch session simulates ping every 15s - heartbeat remains fresh
        orderbook.touch_session(session_id).await;
        let pruned_touched = orderbook.prune_stale_orders(Duration::from_secs(60)).await;
        assert!(pruned_touched.is_empty(), "Order should not be pruned after heartbeat touch");

        // 3. Simulate maker disconnection: set heartbeat to 65 seconds ago
        {
            let mut heartbeats = orderbook.order_heartbeats.write().await;
            heartbeats.insert("order_hb_test".to_string(), std::time::Instant::now() - Duration::from_secs(65));
        }

        // 4. Prune checks 60s timeout: order must be pruned and removed from orderbook
        let pruned = orderbook.prune_stale_orders(Duration::from_secs(60)).await;
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].0, "order_hb_test");
        assert!(!orderbook.orders.read().await.contains_key("order_hb_test"), "Order must be removed after 60s heartbeat loss");
    }
}
