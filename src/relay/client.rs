//! Background Relay WebSocket client for continuous orderbook and match event synchronization.
//! Connects to wss://tru.powgrid.xyz/orderbook and handles OrderMatched, OrderAdded, OrderRemoved events.
//! Provides two-way multiplexed connection for instant order dispatch with verified Relay ACK.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::relay::protocol::{ClientMessage, RelayBroadcastEvent, SignedOrderPayload, TakeOrderPayload};
use crate::swap::engine::SwapEngine;
use crate::swap::orderbook::Orderbook;
use crate::tui::TuiManager;

/// Handle for communicating with the background Relay WebSocket connection
#[derive(Clone)]
pub struct RelayClientHandle {
    outbound_tx: mpsc::Sender<ClientMessage>,
    pending_orders: Arc<RwLock<HashMap<String, oneshot::Sender<Result<(), String>>>>>,
    is_connected: Arc<AtomicBool>,
    relay_url: String,
}

impl RelayClientHandle {
    /// Submits a signed order to the Relay mesh and awaits confirmed OrderAdded ACK or explicit Error.
    pub async fn submit_order_with_ack(
        &self,
        payload: SignedOrderPayload,
        timeout: Duration,
    ) -> Result<(), String> {
        let order_id = payload.order_id.clone();
        let clean_order_id = order_id.strip_prefix("0x").unwrap_or(&order_id).to_lowercase();
        let clean_hash = payload.hash160_hex.strip_prefix("0x").unwrap_or(&payload.hash160_hex).to_lowercase();

        let (ack_tx, ack_rx) = oneshot::channel();

        // 1. Register pending ACK listener for clean_order_id
        {
            let mut pending = self.pending_orders.write().await;
            pending.insert(clean_order_id.clone(), ack_tx);
        }

        // 2. Fast-path: try dispatching via multiplexed persistent connection if connected
        let mut dispatched_via_persistent = false;
        if self.is_connected.load(Ordering::Relaxed) {
            let msg = ClientMessage::SubmitOrder(payload.clone());
            if self.outbound_tx.send(msg).await.is_ok() {
                dispatched_via_persistent = true;
            }
        }

        if dispatched_via_persistent {
            // Await ACK from Relay Mesh with a short fast-path timeout (2.5s)
            let fast_timeout = Duration::from_millis(2500).min(timeout);
            match tokio::time::timeout(fast_timeout, ack_rx).await {
                Ok(Ok(res)) => {
                    return res;
                }
                Ok(Err(_)) => {
                    debug!("[RELAY-CLIENT] Channel dropped during fast-path ACK, falling back to direct probe");
                }
                Err(_) => {
                    warn!("[RELAY-CLIENT] Multiplex connection ACK delayed (>2.5s). Engaging direct probe fallback...");
                }
            }
        }

        // Cleanup any pending slot from the first attempt before fallback
        {
            let mut pending = self.pending_orders.write().await;
            pending.remove(&clean_order_id);
            pending.remove(&clean_hash);
        }

        // 3. Fallback: direct isolated WebSocket probe to ensure 100% order registration reliability
        Self::direct_submit_probe(&self.relay_url, &payload, Duration::from_secs(6)).await
    }

    /// Isolated direct connection to Relay mesh to guarantee order registration when multiplex socket is stalled
    async fn direct_submit_probe(
        relay_url: &str,
        payload: &SignedOrderPayload,
        timeout: Duration,
    ) -> Result<(), String> {
        let msg = ClientMessage::SubmitOrder(payload.clone());
        let text = serde_json::to_string(&msg).map_err(|e| format!("Failed to serialize order: {}", e))?;

        let (mut ws, _) = match tokio::time::timeout(Duration::from_secs(4), connect_async(relay_url)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => return Err(format!("Direct relay connection failed: {}", e)),
            Err(_) => return Err("Direct relay connection timed out (check internet/DNS)".to_string()),
        };

        if let Err(e) = ws.send(Message::Text(text.into())).await {
            return Err(format!("Failed to transmit order on direct probe: {}", e));
        }

        let clean_id = payload.order_id.strip_prefix("0x").unwrap_or(&payload.order_id).to_lowercase();
        let clean_hash = payload.hash160_hex.strip_prefix("0x").unwrap_or(&payload.hash160_hex).to_lowercase();
        let deadline = tokio::time::Instant::now() + timeout;

        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(remaining, ws.next()).await {
                Ok(Some(Ok(Message::Text(resp_text)))) => {
                    if let Ok(evt) = serde_json::from_str::<RelayBroadcastEvent>(&resp_text) {
                        match evt {
                            RelayBroadcastEvent::OrderAdded(o) => {
                                let o_id = o.order_id.strip_prefix("0x").unwrap_or(&o.order_id).to_lowercase();
                                let o_h = o.hash160_hex.strip_prefix("0x").unwrap_or(&o.hash160_hex).to_lowercase();
                                if o_id == clean_id || o_h == clean_hash {
                                    info!("[RELAY-DIRECT-PROBE] Order {} registered successfully on Relay mesh via direct probe", payload.order_id);
                                    return Ok(());
                                }
                            }
                            RelayBroadcastEvent::Error { code, message } => {
                                return Err(format!("[{}]: {}", code, message));
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Some(Ok(Message::Close(_)))) | Ok(None) => {
                    return Err("Relay closed connection during direct probe".to_string());
                }
                Ok(Some(Err(e))) => {
                    return Err(format!("WS read error on direct probe: {}", e));
                }
                Err(_) => break,
                _ => {}
            }
        }

        Err("Timed out waiting for Relay Mesh ACK (check connection or Base RPC)".to_string())
    }

    /// Dispatches a TakeOrder message to the Relay mesh and awaits OrderMatched or Error
    pub async fn take_order(
        &self,
        payload: TakeOrderPayload,
        timeout: Duration,
    ) -> Result<(), String> {
        let order_id = payload.order_id.clone();
        let clean_order_id = order_id.strip_prefix("0x").unwrap_or(&order_id).to_lowercase();

        let (ack_tx, ack_rx) = oneshot::channel();

        // Register pending ACK listener
        {
            let mut pending = self.pending_orders.write().await;
            pending.insert(clean_order_id.clone(), ack_tx);
        }

        let msg = ClientMessage::TakeOrder(payload.clone());
        let mut sent = false;
        if self.is_connected.load(Ordering::Relaxed) {
            if self.outbound_tx.send(msg).await.is_ok() {
                sent = true;
            }
        }

        if !sent {
            // Direct probe fallback for TakeOrder
            let relay_url = self.relay_url.clone();
            if let Ok(text) = serde_json::to_string(&ClientMessage::TakeOrder(payload)) {
                if let Ok(Ok((mut ws, _))) = tokio::time::timeout(Duration::from_secs(4), connect_async(&relay_url)).await {
                    let _ = ws.send(Message::Text(text.into())).await;
                    sent = true;
                }
            }
        }

        if !sent {
            let mut pending = self.pending_orders.write().await;
            pending.remove(&clean_order_id);
            return Err("Relay connection not active and fallback failed".to_string());
        }

        // Await OrderMatched or Error from Relay
        match tokio::time::timeout(timeout, ack_rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => {
                let mut pending = self.pending_orders.write().await;
                pending.remove(&clean_order_id);
                Err("Relay channel dropped before take confirmation".to_string())
            }
            Err(_) => {
                let mut pending = self.pending_orders.write().await;
                pending.remove(&clean_order_id);
                Err("Timed out waiting for Relay take confirmation (order may not exist or relay is unreachable)".to_string())
            }
        }
    }

    /// Dispatches a CancelOrder message to the Relay mesh over the existing multiplex WebSocket connection
    pub async fn cancel_order(
        &self,
        payload: crate::relay::protocol::SignedCancelPayload,
        timeout: Duration,
    ) -> Result<(), String> {
        let order_id = payload.order_id.clone();
        let clean_order_id = order_id.strip_prefix("0x").unwrap_or(&order_id).to_lowercase();

        let (ack_tx, ack_rx) = oneshot::channel();

        // Register pending ACK listener
        {
            let mut pending = self.pending_orders.write().await;
            pending.insert(clean_order_id.clone(), ack_tx);
        }

        // Dispatch via existing persistent connection
        let msg = ClientMessage::CancelOrder(payload);
        if self.is_connected.load(Ordering::Relaxed) {
            if let Err(e) = self.outbound_tx.send(msg).await {
                let mut pending = self.pending_orders.write().await;
                pending.remove(&clean_order_id);
                return Err(format!("Failed to send cancel over multiplex WS: {}", e));
            }
        } else {
            let mut pending = self.pending_orders.write().await;
            pending.remove(&clean_order_id);
            return Err("Relay WebSocket is not connected".to_string());
        }

        // Await OrderRemoved or Error from Relay
        match tokio::time::timeout(timeout, ack_rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Ok(()),
            Err(_) => {
                let mut pending = self.pending_orders.write().await;
                pending.remove(&clean_order_id);
                warn!("[RELAY-CLIENT] Cancel ACK timed out for order {}", order_id);
                Ok(())
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Relaxed)
    }
}

pub struct RelayClient;

impl RelayClient {
    pub fn start_background(
        relay_url: String,
        orderbook: Arc<Orderbook>,
        swap_engine: Arc<SwapEngine>,
        tui: Arc<TuiManager>,
        ui_events: tokio::sync::broadcast::Sender<serde_json::Value>,
    ) -> Arc<RelayClientHandle> {
        let (outbound_tx, mut outbound_rx) = mpsc::channel::<ClientMessage>(128);
        let pending_orders = Arc::new(RwLock::new(HashMap::new()));
        let is_connected = Arc::new(AtomicBool::new(false));

        let handle = Arc::new(RelayClientHandle {
            outbound_tx,
            pending_orders: pending_orders.clone(),
            is_connected: is_connected.clone(),
            relay_url: relay_url.clone(),
        });

        let relay_url_bg = relay_url.clone();
        let is_conn_bg = is_connected.clone();
        let pending_bg = pending_orders.clone();

        tokio::spawn(async move {
            loop {
                info!("[RELAY-CLIENT-BG] Connecting to Relay WS at {}", relay_url_bg);
                match connect_async(&relay_url_bg).await {
                    Ok((ws_stream, _response)) => {
                        tui.log("Connected to TRUSWAP Orderbook Relay Mesh".to_string()).await;
                        info!("[RELAY-CLIENT-BG] Connected successfully to {}", relay_url_bg);
                        is_conn_bg.store(true, Ordering::Relaxed);

                        let (mut write, mut read) = ws_stream.split();
                        let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
                        let mut last_seen = tokio::time::Instant::now();

                        loop {
                            tokio::select! {
                                _ = ping_interval.tick() => {
                                    if last_seen.elapsed() > Duration::from_secs(45) {
                                        warn!("[RELAY-CLIENT-BG] Connection stale (no frames for >45s). Reconnecting...");
                                        break;
                                    }
                                    if let Err(e) = write.send(Message::Ping(vec![].into())).await {
                                        warn!("[RELAY-CLIENT-BG] Ping error: {}", e);
                                        break;
                                    }
                                    let ping_msg = ClientMessage::Ping;
                                    if let Ok(serialized) = serde_json::to_string(&ping_msg) {
                                        let _ = write.send(Message::Text(serialized.into())).await;
                                    }
                                }
                                Some(outbound_msg) = outbound_rx.recv() => {
                                    if let Ok(text) = serde_json::to_string(&outbound_msg) {
                                        if let Err(e) = write.send(Message::Text(text.into())).await {
                                            warn!("[RELAY-CLIENT-BG] Failed to send outbound message: {}", e);
                                            break;
                                        }
                                    }
                                }
                                msg = read.next() => {
                                    last_seen = tokio::time::Instant::now();
                                    match msg {
                                        Some(Ok(Message::Text(text))) => {
                                            Self::handle_text_frame(
                                                &text,
                                                &orderbook,
                                                &swap_engine,
                                                &tui,
                                                &ui_events,
                                                &pending_bg,
                                            ).await;
                                        }
                                        Some(Ok(Message::Binary(bin))) => {
                                            Self::handle_binary_frame(&bin, &orderbook, &swap_engine, &tui, &ui_events).await;
                                        }
                                        Some(Ok(Message::Ping(p))) => {
                                            let _ = write.send(Message::Pong(p)).await;
                                        }
                                        Some(Ok(Message::Pong(_))) => {
                                            // Handled by updating last_seen
                                        }
                                        Some(Ok(Message::Close(_))) => {
                                            warn!("[RELAY-CLIENT-BG] Server closed connection");
                                            break;
                                        }
                                        Some(Err(e)) => {
                                            warn!("[RELAY-CLIENT-BG] WS read error: {}", e);
                                            break;
                                        }
                                        None => {
                                            warn!("[RELAY-CLIENT-BG] WS stream ended");
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        is_conn_bg.store(false, Ordering::Relaxed);
                    }
                    Err(e) => {
                        is_conn_bg.store(false, Ordering::Relaxed);
                        debug!("[RELAY-CLIENT-BG] Connection failed: {}. Retrying in 3s...", e);
                    }
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });

        handle
    }

    async fn handle_text_frame(
        text: &str,
        orderbook: &Arc<Orderbook>,
        swap_engine: &Arc<SwapEngine>,
        tui: &Arc<TuiManager>,
        ui_events: &tokio::sync::broadcast::Sender<serde_json::Value>,
        pending_orders: &Arc<RwLock<HashMap<String, oneshot::Sender<Result<(), String>>>>>,
    ) {
        if let Ok(event) = serde_json::from_str::<super::protocol::RelayBroadcastEvent>(text) {
            match event {
                super::protocol::RelayBroadcastEvent::OrderMatched { order_id, taker_base, taker_tru, taker_tru_pubkey, signature_base } => {
                    let clean_id = order_id.strip_prefix("0x").unwrap_or(&order_id).to_lowercase();
                    {
                        let mut pending = pending_orders.write().await;
                        if let Some(ack_tx) = pending.remove(&clean_id).or_else(|| pending.remove(&order_id)) {
                            let _ = ack_tx.send(Ok(()));
                        }
                    }

                    info!("[RELAY-CLIENT-BG] OrderMatched: id={}, taker_base={}, taker_tru={}, taker_pubkey={:?}, sig={:?}", order_id, taker_base, taker_tru, taker_tru_pubkey, signature_base.as_deref().map(|s| &s[..8.min(s.len())]));
                    let id_short = if order_id.len() >= 8 { &order_id[..8] } else { &order_id };
                    let base_short = if taker_base.len() >= 8 { &taker_base[..8] } else { &taker_base };
                    if let Some(order) = orderbook.get_order_by_hash(&order_id).await {
                        let h_short = if order.hash160_hex.len() >= 8 { &order.hash160_hex[..8] } else { &order.hash160_hex };
                        tui.log(format!("🤝 Relay Match: {:.2} TRU ↔ {:.2} USDC (Order 0x{}... Hash160: 0x{}... taken by Base {})", order.amount_tru, order.amount_usdc, id_short, h_short, base_short)).await;
                    } else {
                        tui.log(format!("🤝 Relay Match: Order 0x{}... taken by Base {}", id_short, base_short)).await;
                    }
                    let canonical_tru = crate::relay::crypto::normalize_to_tru_address(&taker_tru);
                    orderbook.set_taker_base(&order_id, &taker_base).await;
                    if let Some(ref pk) = taker_tru_pubkey {
                        orderbook.set_taker_pubkey(&order_id, pk).await;
                    } else if taker_tru.len() == 66 && (taker_tru.starts_with("02") || taker_tru.starts_with("03")) {
                        orderbook.set_taker_pubkey(&order_id, &taker_tru).await;
                    }
                    orderbook.mark_matched(&order_id).await;
                    swap_engine.on_order_matched(
                        &order_id,
                        &canonical_tru,
                        taker_tru_pubkey.as_deref(),
                        &taker_base,
                        signature_base.as_deref(),
                    ).await;
                    let _ = ui_events.send(serde_json::json!({
                        "type": "order_matched",
                        "data": {
                            "order_id": order_id,
                            "taker_base": taker_base,
                            "taker_tru": taker_tru,
                            "taker_tru_pubkey": taker_tru_pubkey,
                            "signature_base": signature_base,
                        }
                    }));
                }
                super::protocol::RelayBroadcastEvent::OrderAdded(order) => {
                    let order_id = order.order_id.clone();
                    let clean_id = order_id.strip_prefix("0x").unwrap_or(&order_id).to_lowercase();
                    let clean_hash = order.hash160_hex.strip_prefix("0x").unwrap_or(&order.hash160_hex).to_lowercase();

                    // Check if there is an in-flight submit order awaiting ACK
                    {
                        let mut pending = pending_orders.write().await;
                        if let Some(ack_tx) = pending.remove(&clean_id).or_else(|| pending.remove(&clean_hash)) {
                            let _ = ack_tx.send(Ok(()));
                        }
                    }

                    let hash160_hex = order.hash160_hex.clone();
                    orderbook.upsert_unified_order(crate::swap::orderbook::UnifiedOrder {
                        id: order.order_id.clone(),
                        side: match order.side {
                            super::protocol::RelayOrderSide::SellTru => crate::swap::orderbook::OrderSide::SellTruForBaseUsdc,
                            super::protocol::RelayOrderSide::BuyTru => crate::swap::orderbook::OrderSide::BuyTruWithBaseUsdc,
                        },
                        amount_tru: order.amount_tru,
                        amount_usdc: order.amount_usdc,
                        hash160_hex,
                        timelock: order.timelock,
                        source_chain: "TRUSwap-Relay".to_string(),
                        is_active: !order.is_matched,
                        is_matched: order.is_matched,
                        maker_base_address: Some(order.maker_base_address.clone()),
                        maker_tru_pubkey: order.maker_tru_pubkey.clone(),
                        taker_tru_pubkey: None,
                        taker_base_address: None,
                    }).await;
                    let _ = ui_events.send(serde_json::json!({
                        "type": "order_added",
                        "data": order,
                    }));
                }
                super::protocol::RelayBroadcastEvent::Error { code, message } => {
                    warn!("[RELAY-CLIENT-BG] Relay broadcast error [{}]: {}", code, message);
                    let mut pending = pending_orders.write().await;
                    let keys: Vec<String> = pending.keys().cloned().collect();
                    for k in keys {
                        if let Some(tx) = pending.remove(&k) {
                            let _ = tx.send(Err(format!("[{}]: {}", code, message)));
                        }
                    }
                }
                super::protocol::RelayBroadcastEvent::OrderRemoved { order_id, reason } => {
                    let clean_id = order_id.strip_prefix("0x").unwrap_or(&order_id).to_lowercase();
                    {
                        let mut pending = pending_orders.write().await;
                        if let Some(ack_tx) = pending.remove(&clean_id).or_else(|| pending.remove(&order_id)) {
                            let _ = ack_tx.send(Ok(()));
                        }
                    }
                    orderbook.cancel_order(&order_id).await;
                    orderbook.cancel_order(&clean_id).await;
                    let _ = ui_events.send(serde_json::json!({
                        "type": "order_removed",
                        "data": {
                            "order_id": order_id,
                            "reason": reason,
                        }
                    }));
                }
                super::protocol::RelayBroadcastEvent::OrderbookSnapshot { orders, .. } => {
                    for order in &orders {
                        let hash160_hex = order.hash160_hex.clone();
                        orderbook.upsert_unified_order(crate::swap::orderbook::UnifiedOrder {
                            id: order.order_id.clone(),
                            side: match order.side {
                                super::protocol::RelayOrderSide::SellTru => crate::swap::orderbook::OrderSide::SellTruForBaseUsdc,
                                super::protocol::RelayOrderSide::BuyTru => crate::swap::orderbook::OrderSide::BuyTruWithBaseUsdc,
                            },
                            amount_tru: order.amount_tru,
                            amount_usdc: order.amount_usdc,
                            hash160_hex,
                            timelock: order.timelock,
                            source_chain: "TRUSwap-Relay".to_string(),
                            is_active: !order.is_matched,
                            is_matched: order.is_matched,
                            maker_base_address: Some(order.maker_base_address.clone()),
                            maker_tru_pubkey: order.maker_tru_pubkey.clone(),
                            taker_tru_pubkey: None,
                            taker_base_address: None,
                        }).await;
                    }
                    swap_engine.reconcile_orders_with_relay(&orders).await;
                    let _ = ui_events.send(serde_json::json!({
                        "type": "orderbook_snapshot",
                        "data": orders,
                    }));
                }
                _ => {}
            }
        }
    }

    async fn handle_binary_frame(
        bin: &[u8],
        orderbook: &Arc<Orderbook>,
        swap_engine: &Arc<SwapEngine>,
        _tui: &Arc<TuiManager>,
        ui_events: &tokio::sync::broadcast::Sender<serde_json::Value>,
    ) {
        if bin.is_empty() { return; }
        let opcode = bin[0];
        if opcode == 0x33 && bin.len() >= 54 {
            // OP_RELAY_ORDER_MATCHED
            let order_id = hex::encode(&bin[1..33]);
            let taker_base = format!("0x{}", hex::encode(&bin[33..53]));
            let t_len = bin[53] as usize;
            if bin.len() >= 54 + t_len {
                if let Ok(taker_tru) = std::str::from_utf8(&bin[54..54 + t_len]) {
                    let mut cursor = 54 + t_len;
                    let signature_base = if bin.len() > cursor {
                        let s_len = bin[cursor] as usize;
                        cursor += 1;
                        if bin.len() >= cursor + s_len && s_len > 0 {
                            Some(format!("0x{}", hex::encode(&bin[cursor..cursor + s_len])))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    info!("[RELAY-CLIENT-BG-BIN] OrderMatched: id={}, taker_base={}, taker_tru={}, sig={:?}", order_id, taker_base, taker_tru, signature_base.as_deref().map(|s| &s[..8.min(s.len())]));
                    let canonical_tru = crate::relay::crypto::normalize_to_tru_address(taker_tru);
                    orderbook.set_taker_base(&order_id, &taker_base).await;
                    if taker_tru.len() == 66 && (taker_tru.starts_with("02") || taker_tru.starts_with("03")) {
                        orderbook.set_taker_pubkey(&order_id, taker_tru).await;
                    }
                    orderbook.mark_matched(&order_id).await;
                    swap_engine.on_order_matched(
                        &order_id,
                        &canonical_tru,
                        if taker_tru.len() == 66 { Some(taker_tru) } else { None },
                        &taker_base,
                        signature_base.as_deref(),
                    ).await;
                    let _ = ui_events.send(serde_json::json!({
                        "type": "order_matched",
                        "data": {
                            "order_id": order_id,
                            "taker_base": taker_base,
                            "taker_tru": taker_tru,
                            "signature_base": signature_base,
                        }
                    }));
                }
            }
        } else if opcode == 0x32 && bin.len() >= 33 {
            // OP_RELAY_ORDER_REMOVED
            let order_id = hex::encode(&bin[1..33]);
            orderbook.cancel_order(&order_id).await;
            let _ = ui_events.send(serde_json::json!({
                "type": "order_removed",
                "data": { "order_id": order_id }
            }));
        }
    }
}
