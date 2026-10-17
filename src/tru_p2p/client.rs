use super::wire::{
    create_framed_message, deserialize_frame, Addr, BlockProto, GetBlockRequest, GetData,
    GetHeightRequest, HeightMessage, Inv, MessageType, Ping, Pong, TxProto, Version,
    MAINNET_NETWORK_ID,
};
use prost::Message;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{debug, info, warn};

pub struct TruP2PNode {
    pub seeds: Vec<String>,
    known_peers: Arc<Mutex<HashSet<String>>>,
    active_endpoints: Arc<Mutex<HashSet<String>>>,
    active_peers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>,
    current_height: Arc<AtomicU64>,
    pub peer_count: Arc<AtomicU64>,
    pub running: Arc<AtomicBool>,
    pub wake_supervisor: Arc<tokio::sync::Notify>,
    pub tip_notify: Arc<tokio::sync::Notify>,
    pub data_dir: Option<std::path::PathBuf>,
    tx_broadcast_tx: broadcast::Sender<TxProto>,
    block_broadcast_tx: broadcast::Sender<BlockProto>,
}

impl TruP2PNode {
    pub fn new(seeds: Vec<String>) -> Self {
        Self::new_with_data_dir(seeds, None)
    }

    pub fn new_with_data_dir(seeds: Vec<String>, data_dir: Option<std::path::PathBuf>) -> Self {
        let (tx_broadcast_tx, _) = broadcast::channel(256);
        let (block_broadcast_tx, _) = broadcast::channel(4096);

        let initial_seeds = if seeds.is_empty() {
            vec![
                "137.184.68.43:21833".to_string(),
                "193.181.216.10:21833".to_string(),
            ]
        } else {
            seeds
        };

        let mut known = HashSet::new();
        for s in &initial_seeds {
            known.insert(s.clone());
        }

        // Load previously persisted peers from compressed peers.dat
        if let Some(ref dir) = data_dir {
            let cached_peers = super::peer_cache::load_cached_peers(dir);
            for p in cached_peers {
                known.insert(p);
            }
        }

        Self {
            seeds: initial_seeds,
            known_peers: Arc::new(Mutex::new(known)),
            active_endpoints: Arc::new(Mutex::new(HashSet::new())),
            active_peers: Arc::new(Mutex::new(Vec::new())),
            current_height: Arc::new(AtomicU64::new(0)),
            peer_count: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(false)),
            wake_supervisor: Arc::new(tokio::sync::Notify::new()),
            tip_notify: Arc::new(tokio::sync::Notify::new()),
            data_dir,
            tx_broadcast_tx,
            block_broadcast_tx,
        }
    }

    pub fn current_height(&self) -> u64 {
        self.current_height.load(Ordering::Relaxed)
    }

    pub fn peer_count(&self) -> u64 {
        self.peer_count.load(Ordering::Relaxed)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn subscribe_txs(&self) -> broadcast::Receiver<TxProto> {
        self.tx_broadcast_tx.subscribe()
    }

    pub fn subscribe_blocks(&self) -> broadcast::Receiver<BlockProto> {
        self.block_broadcast_tx.subscribe()
    }

    /// Broadcast a transaction to all connected P2P peers
    pub async fn broadcast_tx(&self, txid: &str, raw: &[u8]) -> Result<(), String> {
        let tx_proto = TxProto {
            txid: txid.to_string(),
            raw: raw.to_vec(),
        };

        // Notify local mempool scanner immediately
        let _ = self.tx_broadcast_tx.send(tx_proto.clone());

        let frame = create_framed_message(MessageType::Tx, &tx_proto);
        let peers = self.active_peers.lock().await;

        if peers.is_empty() {
            return Err("No active TRU P2P peers connected to relay transaction".to_string());
        }

        let mut sent = 0;
        for sender in peers.iter() {
            if sender.send(frame.clone()).await.is_ok() {
                sent += 1;
            }
        }

        info!(
            "[TRU-P2P-LE] Broadcasted tx {} to {} peers",
            txid, sent
        );
        Ok(())
    }

    /// Sends a GetBlock request for the specified height to all connected active peers
    pub async fn send_get_block(&self, height: u64) -> bool {
        let req = GetBlockRequest { height };
        let frame = create_framed_message(MessageType::GetBlock, &req);
        let peers = self.active_peers.lock().await;
        if peers.is_empty() {
            return false;
        }
        let mut sent = false;
        for peer in peers.iter() {
            if peer.send(frame.clone()).await.is_ok() {
                sent = true;
            }
        }
        sent
    }

    /// Sends a GetBlock request with a peer index offset (broadcasts to all peers for fastest response)
    pub async fn send_get_block_with_offset(&self, height: u64, _offset: usize) -> bool {
        self.send_get_block(height).await
    }

    /// Sends a batch of GetBlock requests efficiently holding the peer lock once
    pub async fn send_get_blocks_batch(&self, heights: &[u64]) -> usize {
        let peers = self.active_peers.lock().await;
        if peers.is_empty() {
            return 0;
        }

        let mut sent = 0;
        // For realtime/small batches (<= 8 blocks), broadcast to ALL active peers
        // so whichever peer responds fastest delivers the block without delay.
        if heights.len() <= 8 {
            for &h in heights {
                let req = GetBlockRequest { height: h };
                let frame = create_framed_message(MessageType::GetBlock, &req);
                for peer in peers.iter() {
                    if peer.send(frame.clone()).await.is_ok() {
                        sent += 1;
                    }
                }
            }
            return sent;
        }

        const CHUNK_SIZE: u64 = 32;
        for &h in heights {
            let req = GetBlockRequest { height: h };
            let frame = create_framed_message(MessageType::GetBlock, &req);
            let idx = ((h / CHUNK_SIZE) as usize) % peers.len();
            if let Some(peer) = peers.get(idx) {
                if peer.send(frame).await.is_ok() {
                    sent += 1;
                }
            }
        }
        sent
    }

    /// Broadcasts GetBlock requests to ALL active peers unconditionally (for stall recovery & reorg defense)
    pub async fn broadcast_get_blocks(&self, heights: &[u64]) -> usize {
        let peers = self.active_peers.lock().await;
        if peers.is_empty() {
            return 0;
        }
        let mut sent = 0;
        for &h in heights {
            let req = GetBlockRequest { height: h };
            let frame = create_framed_message(MessageType::GetBlock, &req);
            for peer in peers.iter() {
                if peer.send(frame.clone()).await.is_ok() {
                    sent += 1;
                }
            }
        }
        sent
    }

    /// Queries tip height from all connected peers to keep tip up-to-date
    pub async fn query_height_from_peers(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let req = GetHeightRequest {
            timestamp: now,
            requester_id: "truswap-le".to_string(),
        };
        let frame = create_framed_message(MessageType::GetHeight, &req);
        let peers = self.active_peers.lock().await;
        for peer in peers.iter() {
            let _ = peer.send(frame.clone()).await;
        }
    }

    /// Start the TRU P2P supervisor
    pub fn start(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        self.running.store(true, Ordering::SeqCst);
        let node = self.clone();

        tokio::spawn(async move {
            info!("[TRU-P2P] Starting TRU P2P client...");

            const TARGET_ACTIVE_PEERS: usize = 8;

            while node.running.load(Ordering::Relaxed) {
                let to_connect = {
                    let known = node.known_peers.lock().await;
                    let active = node.active_endpoints.lock().await;
                    if active.len() < TARGET_ACTIVE_PEERS {
                        let mut candidates = Vec::new();
                        // 1. Always prioritize configured hardcoded seeds first!
                        for s in &node.seeds {
                            if !active.contains(s) && !candidates.contains(s) {
                                candidates.push(s.clone());
                            }
                        }
                        // 2. Then fill with other known gossiped peers
                        for p in known.iter() {
                            if candidates.len() >= TARGET_ACTIVE_PEERS {
                                break;
                            }
                            if !active.contains(p) && !candidates.contains(p) {
                                candidates.push(p.clone());
                            }
                        }
                        candidates
                    } else {
                        Vec::new()
                    }
                };

                for target in to_connect {
                    let mut active = node.active_endpoints.lock().await;
                    if active.len() >= TARGET_ACTIVE_PEERS {
                        break;
                    }
                    if active.insert(target.clone()) {
                        let node_clone = node.clone();
                        let ep = target.clone();
                        tokio::spawn(async move {
                            if let Err(e) = node_clone.run_peer_session(ep.clone()).await {
                                debug!("[TRU-P2P] Peer {} session ended: {}", ep, e);
                                // Evict failed gossiped peers so they don't block reconnects to seeds
                                if !node_clone.seeds.contains(&ep) {
                                    let mut k = node_clone.known_peers.lock().await;
                                    k.remove(&ep);
                                }
                            }
                            let mut act = node_clone.active_endpoints.lock().await;
                            act.remove(&ep);
                        });
                    }
                }

                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {}
                    _ = node.wake_supervisor.notified() => {}
                }
            }

            info!("[TRU-P2P] P2P client stopped.");
        })
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    async fn run_peer_session(&self, endpoint: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("[TRU-P2P-LE] Connecting to seed/peer: {}", endpoint);
        let stream = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            TcpStream::connect(&endpoint),
        )
        .await??;

        let (mut read_half, mut write_half) = stream.into_split();
        let (outbound_tx, mut outbound_rx) = mpsc::channel::<Vec<u8>>(1024);

        // Register peer writer
        {
            let mut peers = self.active_peers.lock().await;
            peers.push(outbound_tx.clone());
            self.peer_count.store(peers.len() as u64, Ordering::SeqCst);
        }

        info!("[TRU-P2P-LE] Connected to {}", endpoint);

        // Send local VERSION message
        let now_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let version_msg = Version {
            version: 2025,
            sub_version: "TRUSWAP-LE/1.0.0".to_string(),
            services: 0,
            timestamp: now_sec,
            addr_recv: endpoint.clone(),
            addr_from: "0.0.0.0:21833".to_string(),
            nonce: rand::random::<u64>(),
            user_agent: "TRUSWAP-LE/1.0.0".to_string(),
            start_height: self.current_height.load(Ordering::Relaxed) as u32,
            future_flag: "QuantumEncrypted=Yes;AIIntegration=High".to_string(),
            network_id: MAINNET_NETWORK_ID.to_string(),
        };

        let version_frame = create_framed_message(MessageType::Version, &version_msg);
        write_half.write_all(&version_frame).await?;

        // Background writer task
        let mut writer_task = tokio::spawn(async move {
            while let Some(data) = outbound_rx.recv().await {
                if write_half.write_all(&data).await.is_err() {
                    break;
                }
            }
        });

        // Reader loop
        let mut read_buf = Vec::with_capacity(131072);
        let mut tmp_chunk = [0u8; 65536];
        let mut ping_ticker = tokio::time::interval(tokio::time::Duration::from_secs(25));
        let mut height_ticker = tokio::time::interval(tokio::time::Duration::from_secs(2));

        'peer_loop: loop {
            tokio::select! {
                writer_res = &mut writer_task => {
                    debug!("[TRU-P2P-LE] Writer task ended for {}: {:?}", endpoint, writer_res);
                    break;
                }
                res = tokio::time::timeout(tokio::time::Duration::from_secs(35), read_half.read(&mut tmp_chunk)) => {
                    let n = match res {
                        Ok(Ok(n)) if n > 0 => n,
                        _ => {
                            debug!("[TRU-P2P-LE] Read timeout or error on {}", endpoint);
                            break;
                        }
                    };
                    read_buf.extend_from_slice(&tmp_chunk[..n]);

                    if read_buf.len() > super::wire::HEADER_SIZE + super::wire::MAX_FRAME_PAYLOAD {
                        warn!("[TRU-P2P-LE] Read buffer exceeded maximum allowed frame size (64MB) for {}. Dropping peer.", endpoint);
                        break 'peer_loop;
                    }

                    // Process complete frames
                    loop {
                        let (base_msg, consumed) = match deserialize_frame(&read_buf) {
                            Ok(Some(res)) => res,
                            Ok(None) => break,
                            Err(e) => {
                                warn!("[TRU-P2P-LE] Wire framing error from {}: {}. Dropping peer.", endpoint, e);
                                break 'peer_loop;
                            }
                        };
                        let _ = read_buf.drain(..consumed);

                        // If read buffer allocated a large chunk during block transfer, reclaim memory back to OS
                        if read_buf.capacity() > 1024 * 1024 && read_buf.len() < 131072 {
                            read_buf.shrink_to(131072);
                        }

                        if let Some(msg_type) = MessageType::from_i32(base_msg.r#type) {
                            match msg_type {
                                MessageType::Version => {
                                    if let Ok(remote_ver) = Version::decode(&base_msg.payload[..]) {
                                        debug!(
                                            "[TRU-P2P-LE] Received VERSION from {}: subver={}, height={}",
                                            endpoint, remote_ver.sub_version, remote_ver.start_height
                                        );
                                        // Update height if higher
                                        self.current_height.fetch_max(remote_ver.start_height as u64, Ordering::Relaxed);

                                        // Reply with VERACK
                                        let verack = super::wire::VerAck {
                                            quantum_ack: "ack".to_string(),
                                        };
                                        let frame = create_framed_message(MessageType::VerAck, &verack);
                                        let _ = outbound_tx.send(frame).await;

                                        // Immediately ask for current height
                                        let req = GetHeightRequest {
                                            timestamp: now_sec,
                                            requester_id: "truswap-le".to_string(),
                                        };
                                        let frame = create_framed_message(MessageType::GetHeight, &req);
                                        let _ = outbound_tx.send(frame).await;
                                    }
                                }
                                MessageType::VerAck => {
                                    info!("[TRU-P2P-LE] Handshake established with {}", endpoint);
                                }
                                MessageType::Ping => {
                                    if let Ok(ping) = Ping::decode(&base_msg.payload[..]) {
                                        let pong = Pong { nonce: ping.nonce };
                                        let frame = create_framed_message(MessageType::Pong, &pong);
                                        let _ = outbound_tx.send(frame).await;
                                    }
                                }
                                MessageType::Height => {
                                    if let Ok(h) = HeightMessage::decode(&base_msg.payload[..]) {
                                        let old = self.current_height.fetch_max(h.height, Ordering::Relaxed);
                                        if h.height > old {
                                            info!("[TRU-P2P-LE] Tip height updated from {}: #{}", endpoint, h.height);
                                            self.tip_notify.notify_waiters();
                                        }
                                    }
                                }
                                MessageType::Inv => {
                                    if let Ok(inv) = Inv::decode(&base_msg.payload[..]) {
                                        let has_block = inv.inventory.iter().any(|item| item.starts_with("b:"));
                                        if has_block {
                                            let req = GetHeightRequest {
                                                timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                                                requester_id: "truswap-le".to_string(),
                                            };
                                            let frame = create_framed_message(MessageType::GetHeight, &req);
                                            let _ = outbound_tx.send(frame).await;
                                        }

                                        // Request both newly announced block items and tx items directly via GETDATA
                                        let items: Vec<String> = inv.inventory
                                            .iter()
                                            .filter(|item| item.starts_with("b:") || item.starts_with("t:"))
                                            .take(128)
                                            .cloned()
                                            .collect();
                                        if !items.is_empty() {
                                            let get_data = GetData { inventory: items };
                                            let frame = create_framed_message(MessageType::GetData, &get_data);
                                            let _ = outbound_tx.send(frame).await;
                                        }
                                    }
                                }
                                MessageType::Addr => {
                                    if let Ok(addr_msg) = Addr::decode(&base_msg.payload[..]) {
                                        let mut new_discovered = Vec::new();
                                        {
                                             let mut known = self.known_peers.lock().await;
                                             for addr in addr_msg.addresses {
                                                 let trimmed = addr.trim().to_string();
                                                 if !trimmed.is_empty() && known.len() < 2000 && !known.contains(&trimmed) {
                                                     known.insert(trimmed.clone());
                                                     new_discovered.push(trimmed);
                                                 }
                                             }
                                        }
                                        if !new_discovered.is_empty() {
                                            info!(
                                                "[TRU-P2P-LE] Auto-discovered {} new peer endpoint(s) via ADDR gossip from {}: {:?}",
                                                new_discovered.len(),
                                                endpoint,
                                                new_discovered
                                            );
                                            // Wake up supervisor to immediately dial newly discovered nodes
                                            self.wake_supervisor.notify_one();

                                            // Persist updated peer pool to compressed peers.dat
                                            if let Some(ref dir) = self.data_dir {
                                                let dir_clone = dir.clone();
                                                let known_peers_clone = self.known_peers.clone();
                                                tokio::spawn(async move {
                                                    let set = known_peers_clone.lock().await;
                                                    let _ = super::peer_cache::save_cached_peers(&dir_clone, &set);
                                                });
                                            }
                                        }
                                    }
                                }
                                MessageType::Tx => {
                                    if let Ok(tx) = TxProto::decode(&base_msg.payload[..]) {
                                        debug!("[TRU-P2P-LE] Incoming TX: {}", tx.txid);
                                        let _ = self.tx_broadcast_tx.send(tx);
                                    }
                                }
                                MessageType::Block => {
                                    if let Ok(blk) = BlockProto::decode(&base_msg.payload[..]) {
                                        debug!("[TRU-P2P-LE] Incoming BLOCK: #{} {}", blk.height, blk.hash);
                                        self.current_height.fetch_max(blk.height as u64, Ordering::Relaxed);
                                        let _ = self.block_broadcast_tx.send(blk);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ = height_ticker.tick() => {
                    let req = GetHeightRequest {
                        timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                        requester_id: "truswap-le".to_string(),
                    };
                    let frame = create_framed_message(MessageType::GetHeight, &req);
                    if outbound_tx.send(frame).await.is_err() {
                        break;
                    }
                }
                _ = ping_ticker.tick() => {
                    let ping = Ping { nonce: rand::random::<u64>() };
                    let frame = create_framed_message(MessageType::Ping, &ping);
                    if outbound_tx.send(frame).await.is_err() {
                        break;
                    }
                }
            }
        }

        writer_task.abort();

        // Deregister peer
        {
            let mut peers = self.active_peers.lock().await;
            peers.retain(|p| !p.same_channel(&outbound_tx));
            self.peer_count.store(peers.len() as u64, Ordering::SeqCst);
        }

        warn!("[TRU-P2P-LE] Disconnected from peer: {}", endpoint);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires live TRU seed node"]
    async fn test_live_handshake_with_seed() {
        let node = Arc::new(TruP2PNode::new(vec!["137.184.68.43:21833".to_string()]));
        let handle = node.clone().start();

        // Give it up to 4 seconds to connect and complete handshake
        for _ in 0..8 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if node.peer_count() > 0 && node.current_height() > 0 {
                break;
            }
        }

        let height = node.current_height();
        let peers = node.peer_count();
        node.stop();
        handle.abort();

        println!("Connected peers: {}, Chain height: {}", peers, height);
        assert!(peers >= 1, "Should connect to at least 1 seed");
        assert!(height > 0, "Should receive chain height > 0 from mainnet seed");
    }

    #[tokio::test]
    #[ignore = "requires live TRU seed node"]
    async fn test_live_get_block() {
        let node = Arc::new(TruP2PNode::new(vec!["137.184.68.43:21833".to_string()]));
        let handle = node.clone().start();

        // Wait for connection
        for _ in 0..10 {
            tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
            if node.peer_count() > 0 {
                break;
            }
        }

        let mut rx = node.subscribe_blocks();
        // Request block #1
        let req = GetBlockRequest { height: 1 };
        let frame = create_framed_message(MessageType::GetBlock, &req);
        let peers = node.active_peers.lock().await;
        if let Some(peer) = peers.first() {
            let _ = peer.send(frame).await;
        }
        drop(peers);

        let received = tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await;
        node.stop();
        handle.abort();

        match received {
            Ok(Ok(blk)) => {
                println!("SUCCESS received block #{} hash={} txs_count={}", blk.height, blk.hash, blk.transactions.len());
                if let Some(first_tx) = blk.transactions.first() {
                    println!("First TX raw: {}", first_tx);
                }
            }
            e => println!("Block fetch error or timeout: {:?}", e),
        }
    }
}
