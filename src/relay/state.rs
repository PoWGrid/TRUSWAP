use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use super::protocol::{RelayBroadcastEvent, RelayOrder, RelayOrderSide};

#[derive(Debug, Clone)]
pub struct TakerAllocation {
    pub order_id: String,
    pub amount_usdc: f64,
    pub amount_atoms: u64,
    pub matched_at: u64,
}

pub struct HotOrderbookState {
    /// Primary storage: order_id -> RelayOrder
    pub orders: RwLock<HashMap<String, RelayOrder>>,
    /// Index: Base Maker Address (lowercase) -> List of active Order IDs
    pub base_maker_index: RwLock<HashMap<String, Vec<String>>>,
    /// Index: TRU Maker Address -> List of active Order IDs
    pub tru_maker_index: RwLock<HashMap<String, Vec<String>>>,
    /// Inverted O(1) set of all active EVM maker addresses for lightning block scanning
    pub active_evm_addresses: RwLock<HashSet<String>>,
    /// In-flight allocations committed by Takers for Base USDC (lowercase taker_base -> allocations)
    pub taker_allocations_base: RwLock<HashMap<String, Vec<TakerAllocation>>>,
    /// In-flight allocations committed by Takers for TRU (taker_tru -> allocations)
    pub taker_allocations_tru: RwLock<HashMap<String, Vec<TakerAllocation>>>,
    /// Broadcast channel to stream live updates to all connected WebSockets
    pub broadcast_tx: broadcast::Sender<RelayBroadcastEvent>,
    /// Tracks last communication timestamp from maker: order_id -> Instant
    pub order_heartbeats: RwLock<HashMap<String, std::time::Instant>>,
    /// Tracks which orders belong to which WebSocket session: session_id -> Set of order_ids
    pub session_orders: RwLock<HashMap<u64, HashSet<String>>>,
    /// Tracks which maker addresses belong to which WebSocket session: session_id -> Set of maker_base_address (lowercase)
    pub session_makers: RwLock<HashMap<u64, HashSet<String>>>,
}

impl HotOrderbookState {
    pub fn new() -> Arc<Self> {
        let (broadcast_tx, _) = broadcast::channel(4096);
        let state = Arc::new(Self {
            orders: RwLock::new(HashMap::new()),
            base_maker_index: RwLock::new(HashMap::new()),
            tru_maker_index: RwLock::new(HashMap::new()),
            active_evm_addresses: RwLock::new(HashSet::new()),
            taker_allocations_base: RwLock::new(HashMap::new()),
            taker_allocations_tru: RwLock::new(HashMap::new()),
            broadcast_tx,
            order_heartbeats: RwLock::new(HashMap::new()),
            session_orders: RwLock::new(HashMap::new()),
            session_makers: RwLock::new(HashMap::new()),
        });

        // Spawn background heartbeat reaper: if no connection from maker for 60s, prune all their orders
        let reaper_weak = Arc::downgrade(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                interval.tick().await;
                if let Some(strong) = reaper_weak.upgrade() {
                    let pruned = strong.prune_stale_orders(std::time::Duration::from_secs(60)).await;
                    if !pruned.is_empty() {
                        tracing::info!(
                            "[RELAY-HEARTBEAT] Pruned {} stale order(s) due to 60s maker heartbeat timeout: {:?}",
                            pruned.len(),
                            pruned
                        );
                    }
                } else {
                    break;
                }
            }
        });

        state
    }

    /// Adds a verified order into the in-memory orderbook and updates all indices
    pub async fn add_order(&self, order: RelayOrder) {
        let order_id = order.order_id.clone();
        let base_maker = order.maker_base_address.to_lowercase();
        let tru_maker = order.maker_tru_address.clone();

        {
            let mut orders_lock = self.orders.write().await;
            orders_lock.insert(order_id.clone(), order.clone());
        }

        {
            let mut base_index_lock = self.base_maker_index.write().await;
            base_index_lock
                .entry(base_maker.clone())
                .or_default()
                .push(order_id.clone());
        }

        {
            let mut tru_index_lock = self.tru_maker_index.write().await;
            tru_index_lock
                .entry(tru_maker)
                .or_default()
                .push(order_id.clone());
        }

        {
            let mut evm_set_lock = self.active_evm_addresses.write().await;
            evm_set_lock.insert(base_maker);
        }

        {
            let mut heartbeats = self.order_heartbeats.write().await;
            heartbeats.insert(order_id, std::time::Instant::now());
        }

        // Notify all subscribers about the new order
        let _ = self
            .broadcast_tx
            .send(RelayBroadcastEvent::OrderAdded(order));
    }

    /// Removes an order by ID and purges it from all index structures
    pub async fn remove_order(&self, order_id: &str, reason: &str) -> Option<RelayOrder> {
        let removed = {
            let mut orders_lock = self.orders.write().await;
            orders_lock.remove(order_id)
        };

        if let Some(ref order) = removed {
            let base_maker = order.maker_base_address.to_lowercase();
            let tru_maker = &order.maker_tru_address;

            // Prune from heartbeats
            {
                let mut heartbeats = self.order_heartbeats.write().await;
                heartbeats.remove(order_id);
            }

            // Prune from base index
            {
                let mut base_index_lock = self.base_maker_index.write().await;
                if let Some(list) = base_index_lock.get_mut(&base_maker) {
                    list.retain(|id| id != order_id);
                    if list.is_empty() {
                        base_index_lock.remove(&base_maker);
                        let mut evm_set_lock = self.active_evm_addresses.write().await;
                        evm_set_lock.remove(&base_maker);
                    }
                }
            }

            // Prune from TRU index
            {
                let mut tru_index_lock = self.tru_maker_index.write().await;
                if let Some(list) = tru_index_lock.get_mut(tru_maker) {
                    list.retain(|id| id != order_id);
                    if list.is_empty() {
                        tru_index_lock.remove(tru_maker);
                    }
                }
            }

            // Prune from Taker allocation indices
            {
                let mut base_allocs = self.taker_allocations_base.write().await;
                for list in base_allocs.values_mut() {
                    list.retain(|a| a.order_id != order_id);
                }
                base_allocs.retain(|_, list| !list.is_empty());

                let mut tru_allocs = self.taker_allocations_tru.write().await;
                for list in tru_allocs.values_mut() {
                    list.retain(|a| a.order_id != order_id);
                }
                tru_allocs.retain(|_, list| !list.is_empty());
            }

            // Notify all subscribers
            let _ = self.broadcast_tx.send(RelayBroadcastEvent::OrderRemoved {
                order_id: order_id.to_string(),
                reason: reason.to_string(),
            });
        }

        removed
    }

    /// Registers a newly submitted order with a specific WebSocket session
    pub async fn register_session_order(&self, session_id: u64, order_id: &str, maker_base: &str) {
        let clean_base = maker_base.to_lowercase();
        {
            let mut session_orders = self.session_orders.write().await;
            session_orders
                .entry(session_id)
                .or_default()
                .insert(order_id.to_string());
        }
        {
            let mut session_makers = self.session_makers.write().await;
            session_makers
                .entry(session_id)
                .or_default()
                .insert(clean_base);
        }
        {
            let mut heartbeats = self.order_heartbeats.write().await;
            heartbeats.insert(order_id.to_string(), std::time::Instant::now());
        }
    }

    /// Associates a maker address with an existing WebSocket session (e.g. on client reconnect / heartbeat)
    pub async fn associate_session_maker(&self, session_id: u64, maker_base: &str) {
        let clean_base = maker_base.to_lowercase();
        {
            let mut session_makers = self.session_makers.write().await;
            session_makers
                .entry(session_id)
                .or_default()
                .insert(clean_base);
        }
        self.touch_session(session_id).await;
    }

    /// Touches the heartbeat timestamp for all orders associated with a WebSocket session
    pub async fn touch_session(&self, session_id: u64) {
        let now = std::time::Instant::now();
        let direct_orders = {
            let s_orders = self.session_orders.read().await;
            s_orders.get(&session_id).cloned().unwrap_or_default()
        };

        let makers = {
            let s_makers = self.session_makers.read().await;
            s_makers.get(&session_id).cloned().unwrap_or_default()
        };

        let mut all_orders_to_touch = direct_orders;
        if !makers.is_empty() {
            let base_idx = self.base_maker_index.read().await;
            for maker in makers {
                if let Some(orders) = base_idx.get(&maker) {
                    for o in orders {
                        all_orders_to_touch.insert(o.clone());
                    }
                }
            }
        }

        if !all_orders_to_touch.is_empty() {
            let mut heartbeats = self.order_heartbeats.write().await;
            for id in all_orders_to_touch {
                heartbeats.insert(id, now);
            }
        }
    }

    /// Releases a closed WebSocket session from the tracking structures (orders remain subject to 60s timeout)
    pub async fn release_session(&self, session_id: u64) {
        let mut session_orders = self.session_orders.write().await;
        session_orders.remove(&session_id);
        let mut session_makers = self.session_makers.write().await;
        session_makers.remove(&session_id);
    }

    /// Prunes all orders whose makers have had no communication for longer than `timeout`
    pub async fn prune_stale_orders(&self, timeout: std::time::Duration) -> Vec<(String, String)> {
        let now = std::time::Instant::now();
        let now_sec = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let stale_order_ids: Vec<(String, String)> = {
            let heartbeats = self.order_heartbeats.read().await;
            let orders = self.orders.read().await;
            orders
                .iter()
                .filter(|(id, o)| {
                    if o.is_matched {
                        return false;
                    }
                    if let Some(last) = heartbeats.get(*id) {
                        now.duration_since(*last) >= timeout
                    } else {
                        // Fallback: if order has no explicit heartbeat, check created_at age
                        now_sec.saturating_sub(o.created_at) >= timeout.as_secs()
                    }
                })
                .map(|(id, o)| (id.clone(), o.maker_base_address.clone()))
                .collect()
        };

        let mut pruned = Vec::new();
        for (order_id, maker_base) in stale_order_ids {
            if self.remove_order(&order_id, "Maker disconnected (no communication for 60s)").await.is_some() {
                pruned.push((order_id, maker_base));
            }
        }
        pruned
    }

    /// Marks an order as matched/locked for an active atomic swap
    pub async fn match_order(
        &self,
        order_id: &str,
        taker_base: &str,
        taker_tru: &str,
        taker_tru_pubkey: Option<&str>,
        signature_base: Option<&str>,
    ) -> Result<RelayOrder, String> {
        let mut orders_lock = self.orders.write().await;
        if let Some(order) = orders_lock.get_mut(order_id) {
            if order.is_matched {
                return Err("Order is already matched or pending execution".into());
            }
            order.is_matched = true;
            let matched_order = order.clone();

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Record active taker commitments to prevent reuse in parallel takes
            let clean_base = taker_base.strip_prefix("0x").unwrap_or(taker_base).to_lowercase();
            let amount_atoms = (matched_order.amount_tru * 100_000_000.0).round() as u64;

            match matched_order.side {
                RelayOrderSide::SellTru => {
                    // Maker sells TRU -> Taker buys TRU (commits USDC on Base)
                    let mut base_allocs = self.taker_allocations_base.write().await;
                    let list = base_allocs.entry(clean_base).or_default();
                    list.retain(|a| now.saturating_sub(a.matched_at) < 300);
                    list.push(TakerAllocation {
                        order_id: order_id.to_string(),
                        amount_usdc: matched_order.amount_usdc,
                        amount_atoms: 0,
                        matched_at: now,
                    });
                    if base_allocs.len() > 1000 {
                        base_allocs.retain(|_, allocs| {
                            allocs.retain(|a| now.saturating_sub(a.matched_at) < 300);
                            !allocs.is_empty()
                        });
                    }
                }
                RelayOrderSide::BuyTru => {
                    // Maker buys TRU -> Taker sells TRU (commits TRU atoms on TRU network)
                    let mut tru_allocs = self.taker_allocations_tru.write().await;
                    let list = tru_allocs.entry(taker_tru.to_string()).or_default();
                    list.retain(|a| now.saturating_sub(a.matched_at) < 300);
                    list.push(TakerAllocation {
                        order_id: order_id.to_string(),
                        amount_usdc: 0.0,
                        amount_atoms,
                        matched_at: now,
                    });
                    if tru_allocs.len() > 1000 {
                        tru_allocs.retain(|_, allocs| {
                            allocs.retain(|a| now.saturating_sub(a.matched_at) < 300);
                            !allocs.is_empty()
                        });
                    }
                }
            }

            let _ = self.broadcast_tx.send(RelayBroadcastEvent::OrderMatched {
                order_id: order_id.to_string(),
                taker_base: taker_base.to_string(),
                taker_tru: taker_tru.to_string(),
                taker_tru_pubkey: taker_tru_pubkey.map(|s| s.to_string()),
                signature_base: signature_base.map(|s| s.to_string()),
            });

            Ok(matched_order)
        } else {
            Err("Order not found".into())
        }
    }

    /// Returns the total USDC currently committed by this taker in active in-flight swaps (TTL protected)
    pub async fn get_taker_allocated_usdc(&self, taker_base: &str, ttl_secs: u64) -> f64 {
        let clean = taker_base.strip_prefix("0x").unwrap_or(taker_base).to_lowercase();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let alloc_lock = self.taker_allocations_base.read().await;
        if let Some(allocs) = alloc_lock.get(&clean) {
            allocs
                .iter()
                .filter(|a| now.saturating_sub(a.matched_at) < ttl_secs)
                .map(|a| a.amount_usdc)
                .sum()
        } else {
            0.0
        }
    }

    /// Returns the total TRU atoms currently committed by this taker in active in-flight swaps (TTL protected)
    pub async fn get_taker_allocated_tru(&self, taker_tru: &str, ttl_secs: u64) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let alloc_lock = self.taker_allocations_tru.read().await;
        if let Some(allocs) = alloc_lock.get(taker_tru) {
            allocs
                .iter()
                .filter(|a| now.saturating_sub(a.matched_at) < ttl_secs)
                .map(|a| a.amount_atoms)
                .sum()
        } else {
            0
        }
    }

    /// Returns all active, unmatched orders currently held in memory
    pub async fn get_snapshot(&self) -> Vec<RelayOrder> {
        let orders_lock = self.orders.read().await;
        orders_lock.values().filter(|o| !o.is_matched).cloned().collect()
    }

    /// Retrieves all active order IDs associated with a specific EVM maker address
    pub async fn get_orders_by_base_maker(&self, base_address: &str) -> Vec<String> {
        let base_index_lock = self.base_maker_index.read().await;
        base_index_lock
            .get(&base_address.to_lowercase())
            .cloned()
            .unwrap_or_default()
    }
}
