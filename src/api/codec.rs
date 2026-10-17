//! WebSocket frame codec for binary packet serialization.

use crate::tru_p2p::scanner::{DiscoveredTruSwap, MempoolEntry, SwapStatus};

// Downlink (Server -> Client)
pub const OP_INIT_STATE: u8 = 0x01;
pub const OP_BALANCE_UPDATE: u8 = 0x02;
pub const OP_ORDERBOOK_ENTRY: u8 = 0x03;
pub const OP_SWAP_PROGRESS: u8 = 0x04;
pub const OP_PONG: u8 = 0x05;
pub const OP_VAULT_STATUS: u8 = 0x11;
pub const OP_WITHDRAW_RESP: u8 = 0x15;
pub const OP_SWAPS_LEDGER_RESP: u8 = 0x17;
pub const OP_MEMPOOL_RESP: u8 = 0x19;
pub const OP_VAULT_EXPORT_RESP: u8 = 0x21;

// Uplink (Client -> Server)
pub const CMD_PING: u8 = 0x05;
pub const CMD_REQ_VAULT_STATUS: u8 = 0x10;
pub const CMD_CREATE_OFFER: u8 = 0x12;
pub const CMD_TAKE_ORDER: u8 = 0x13;
pub const CMD_REQ_WITHDRAW: u8 = 0x14;
pub const CMD_REQ_SWAPS_LEDGER: u8 = 0x16;
pub const CMD_REQ_MEMPOOL: u8 = 0x18;
pub const CMD_REQ_VAULT_EXPORT: u8 = 0x20;

#[derive(Clone, Debug, Default)]
pub struct RawVaultStatus {
    pub address: String,
    pub confirmed_atoms: u64,
    pub unconfirmed_atoms: u64,
    pub mempool_count: u32,
    pub utxos: Vec<RawUtxo>,
}

#[derive(Clone, Debug, Default)]
pub struct RawUtxo {
    pub txid: [u8; 32],
    pub vout: u32,
    pub amount_atoms: u64,
    pub height: u64,
}

/// Encodes initial state into a compact binary frame.
pub fn encode_init_state(
    block_height: u32,
    tru_atoms: u64,
    usdc_atomic: u64,
    swap_address: &str,
    p2p_peers: u64,
) -> Vec<u8> {
    let addr_bytes = swap_address.as_bytes();
    let addr_len = addr_bytes.len().min(255) as u8;

    let flags = if p2p_peers > 0 {
        0x80 | (p2p_peers.min(127) as u8)
    } else {
        0x00
    };

    let mut buf = Vec::with_capacity(23 + addr_len as usize);
    buf.push(OP_INIT_STATE);
    buf.push(flags); // flags: bit 7 = P2P mode, bits 0-6 = peer count
    buf.extend_from_slice(&block_height.to_le_bytes());
    buf.extend_from_slice(&tru_atoms.to_le_bytes());
    buf.extend_from_slice(&usdc_atomic.to_le_bytes());
    buf.push(addr_len);
    buf.extend_from_slice(&addr_bytes[..addr_len as usize]);
    buf
}

/// Encodes balance update.
pub fn encode_balance_update(chain_id: u8, amount: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(10);
    buf.push(OP_BALANCE_UPDATE);
    buf.push(chain_id);
    buf.extend_from_slice(&amount.to_le_bytes());
    buf
}

/// Encodes orderbook entry.
pub fn encode_orderbook_entry(
    side: u8,
    tru_amount: u64,
    usdc_amount: u64,
    hash160: &[u8; 20],
    timelock: u32,
    is_live: bool,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(43);
    buf.push(OP_ORDERBOOK_ENTRY);
    buf.push(side);
    buf.extend_from_slice(&tru_amount.to_le_bytes());
    buf.extend_from_slice(&usdc_amount.to_le_bytes());
    buf.extend_from_slice(hash160);
    buf.extend_from_slice(&timelock.to_le_bytes());
    buf.push(if is_live { 1 } else { 0 });
    buf
}

/// Encodes swap execution lifecycle progress.
pub fn encode_swap_progress(step: u8, status: u8, id_bytes: &[u8; 32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(35);
    buf.push(OP_SWAP_PROGRESS);
    buf.push(step);
    buf.push(status);
    buf.extend_from_slice(id_bytes);
    buf
}

/// Encodes complete vault status with UTXOs into a single binary frame.
/// [0]: 0x11
/// [1]: addr_len (u8)
/// [2..2+addr_len]: address (ASCII/UTF-8)
/// [offset..offset+8]: confirmed_atoms (u64 LE)
/// [offset+8..offset+16]: unconfirmed_atoms (u64 LE)
/// [offset+16..offset+20]: mempool_count (u32 LE)
/// [offset+20..offset+22]: utxo_count (u16 LE)
/// For each UTXO (52 bytes):
///   [0..32]: txid (raw 32B)
///   [32..36]: vout (u32 LE)
///   [36..44]: amount_atoms (u64 LE)
///   [44..52]: height (u64 LE)
pub fn encode_vault_status(status: &RawVaultStatus) -> Vec<u8> {
    let addr_bytes = status.address.as_bytes();
    let addr_len = addr_bytes.len().min(255) as u8;
    let mut buf = Vec::with_capacity(1 + 1 + addr_len as usize + 8 + 8 + 4 + 2 + status.utxos.len() * 52);
    buf.push(OP_VAULT_STATUS);
    buf.push(addr_len);
    buf.extend_from_slice(&addr_bytes[..addr_len as usize]);
    buf.extend_from_slice(&status.confirmed_atoms.to_le_bytes());
    buf.extend_from_slice(&status.unconfirmed_atoms.to_le_bytes());
    buf.extend_from_slice(&status.mempool_count.to_le_bytes());
    buf.extend_from_slice(&(status.utxos.len().min(65535) as u16).to_le_bytes());
    for u in &status.utxos {
        buf.extend_from_slice(&u.txid);
        buf.extend_from_slice(&u.vout.to_le_bytes());
        buf.extend_from_slice(&u.amount_atoms.to_le_bytes());
        buf.extend_from_slice(&u.height.to_le_bytes());
    }
    buf
}

/// Encodes withdrawal execution response into a binary frame.
/// [0]: 0x15
/// [1..5]: req_id (u32 LE)
/// [5]: status (0 = Success, 1 = Error)
/// [6..38]: txid (raw 32B)
/// [38..40]: msg_len (u16 LE)
/// [40..40+msg_len]: msg (UTF-8)
pub fn encode_withdraw_response(
    req_id: u32,
    success: bool,
    txid: &str,
    msg: &str,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64 + msg.len());
    buf.push(OP_WITHDRAW_RESP);
    buf.extend_from_slice(&req_id.to_le_bytes());
    buf.push(if success { 0 } else { 1 });
    let mut txid_raw = [0u8; 32];
    if let Ok(b) = hex::decode(txid.trim()) {
        if b.len() == 32 {
            txid_raw.copy_from_slice(&b);
        }
    }
    buf.extend_from_slice(&txid_raw);
    let msg_bytes = msg.as_bytes();
    let msg_len = msg_bytes.len().min(65535) as u16;
    buf.extend_from_slice(&msg_len.to_le_bytes());
    buf.extend_from_slice(&msg_bytes[..msg_len as usize]);
    buf
}

/// Encodes swaps ledger into a binary frame.
/// [0]: 0x17
/// [1..9]: scanned_height (u64 LE)
/// [9..17]: verified_height (u64 LE)
/// [17..25]: target_height (u64 LE)
/// [25..27]: swaps_count (u16 LE)
/// For each swap (101 bytes):
///   [0..20]: hash160
///   [20..28]: amount_tru (u64 LE atoms)
///   [28]: status (0=Active, 1=Claimed, 2=Refunded, 3=Expired, 4=ExpiringSoon)
///   [29..33]: timelock (u32 LE)
///   [33..37]: block_height (u32 LE)
///   [37..69]: tru_txid (32B)
///   [69..101]: spent_by_txid (32B)
pub fn encode_swaps_ledger(
    scanned: u64,
    verified: u64,
    target: u64,
    swaps: &[DiscoveredTruSwap],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(27 + swaps.len() * 101);
    buf.push(OP_SWAPS_LEDGER_RESP);
    buf.extend_from_slice(&scanned.to_le_bytes());
    buf.extend_from_slice(&verified.to_le_bytes());
    buf.extend_from_slice(&target.to_le_bytes());
    buf.extend_from_slice(&(swaps.len().min(65535) as u16).to_le_bytes());
    for s in swaps {
        let mut h160 = [0u8; 20];
        if let Ok(b) = hex::decode(&s.hash160_hex) {
            if b.len() == 20 {
                h160.copy_from_slice(&b);
            }
        }
        buf.extend_from_slice(&h160);
        buf.extend_from_slice(&s.amount_atoms.to_le_bytes());
        let status_byte = match s.status {
            SwapStatus::Active => 0u8,
            SwapStatus::Claimed => 1u8,
            SwapStatus::Refunded => 2u8,
            SwapStatus::Expired => 3u8,
            SwapStatus::ExpiringSoon => 4u8,
        };
        buf.push(status_byte);
        buf.extend_from_slice(&s.timelock.to_le_bytes());
        buf.extend_from_slice(&s.block_height.to_le_bytes());

        let mut tru_tx = [0u8; 32];
        if let Ok(b) = hex::decode(&s.txid) {
            if b.len() == 32 {
                tru_tx.copy_from_slice(&b);
            }
        }
        buf.extend_from_slice(&tru_tx);

        let mut spent_tx = [0u8; 32];
        if let Some(t) = &s.spent_by_txid {
            if let Ok(b) = hex::decode(t) {
                if b.len() == 32 {
                    spent_tx.copy_from_slice(&b);
                }
            }
        }
        buf.extend_from_slice(&spent_tx);
    }
    buf
}

/// Encodes mempool transactions into a binary frame.
/// [0]: 0x19
/// [1..3]: tx_count (u16 LE)
/// For each tx (48 bytes):
///   [0..32]: txid (32B)
///   [32..34]: vin_count (u16 LE)
///   [34..36]: vout_count (u16 LE)
///   [36..44]: total_output_atoms (u64 LE)
///   [44..48]: age_secs (u32 LE)
pub fn encode_mempool_list(txs: &[MempoolEntry]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(3 + txs.len() * 48);
    buf.push(OP_MEMPOOL_RESP);
    buf.extend_from_slice(&(txs.len().min(65535) as u16).to_le_bytes());
    let now_sec = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for tx in txs {
        let mut txid_raw = [0u8; 32];
        if let Ok(b) = hex::decode(&tx.txid) {
            if b.len() == 32 {
                txid_raw.copy_from_slice(&b);
            }
        }
        buf.extend_from_slice(&txid_raw);
        buf.extend_from_slice(&(tx.inputs_count.min(65535) as u16).to_le_bytes());
        buf.extend_from_slice(&(tx.outputs_count.min(65535) as u16).to_le_bytes());
        buf.extend_from_slice(&tx.total_out_atoms.to_le_bytes());
        let age_secs = now_sec.saturating_sub(tx.first_seen_sec).min(u32::MAX as u64) as u32;
        buf.extend_from_slice(&age_secs.to_le_bytes());
    }
    buf
}

#[derive(Debug)]
pub enum ClientCommand {
    Ping,
    ReqVaultStatus,
    ReqSwapsLedger,
    ReqMempool,
    ReqVaultExport {
        token: String,
    },
    ReqWithdraw {
        req_id: u32,
        amount_atoms: u64,
        target_address: String,
    },
    CreateOffer {
        tru_atoms: u64,
        usdc_atomic: u64,
        timelock_hours: u16,
    },
    TakeOrder {
        hash160: [u8; 20],
    },
    Unknown(u8),
}

/// Encodes vault export credentials response into binary frame
pub fn encode_vault_export_response(
    success: bool,
    address: &str,
    priv_hex: &str,
    priv_wif: &str,
    mnemonic: &str,
    err_msg: &str,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    buf.push(OP_VAULT_EXPORT_RESP);
    if success {
        buf.push(1);
        let a_b = address.as_bytes();
        buf.push(a_b.len().min(255) as u8);
        buf.extend_from_slice(&a_b[..a_b.len().min(255)]);

        let h_b = priv_hex.as_bytes();
        buf.push(h_b.len().min(255) as u8);
        buf.extend_from_slice(&h_b[..h_b.len().min(255)]);

        let w_b = priv_wif.as_bytes();
        buf.push(w_b.len().min(255) as u8);
        buf.extend_from_slice(&w_b[..w_b.len().min(255)]);

        let m_b = mnemonic.as_bytes();
        buf.extend_from_slice(&(m_b.len().min(65535) as u16).to_le_bytes());
        buf.extend_from_slice(&m_b[..m_b.len().min(65535)]);
    } else {
        buf.push(0);
        let e_b = err_msg.as_bytes();
        buf.extend_from_slice(&(e_b.len().min(65535) as u16).to_le_bytes());
        buf.extend_from_slice(&e_b[..e_b.len().min(65535)]);
    }
    buf
}

/// Decodes binary command from the browser.
pub fn decode_client_command(payload: &[u8]) -> Option<ClientCommand> {
    if payload.is_empty() {
        return None;
    }
    let opcode = payload[0];
    match opcode {
        CMD_PING => Some(ClientCommand::Ping),
        CMD_REQ_VAULT_STATUS => Some(ClientCommand::ReqVaultStatus),
        CMD_REQ_SWAPS_LEDGER => Some(ClientCommand::ReqSwapsLedger),
        CMD_REQ_MEMPOOL => Some(ClientCommand::ReqMempool),
        CMD_REQ_VAULT_EXPORT => {
            if payload.len() < 2 {
                return None;
            }
            let tok_len = payload[1] as usize;
            if payload.len() < 2 + tok_len {
                return None;
            }
            let token = String::from_utf8_lossy(&payload[2..2 + tok_len]).to_string();
            Some(ClientCommand::ReqVaultExport { token })
        }
        CMD_REQ_WITHDRAW => {
            if payload.len() < 14 {
                return None;
            }
            let req_id = u32::from_le_bytes(payload[1..5].try_into().ok()?);
            let amount_atoms = u64::from_le_bytes(payload[5..13].try_into().ok()?);
            let addr_len = payload[13] as usize;
            if payload.len() < 14 + addr_len {
                return None;
            }
            let target_address = String::from_utf8_lossy(&payload[14..14 + addr_len]).trim().to_string();
            Some(ClientCommand::ReqWithdraw {
                req_id,
                amount_atoms,
                target_address,
            })
        }
        CMD_CREATE_OFFER => {
            if payload.len() < 19 {
                return None;
            }
            let tru_atoms = u64::from_le_bytes(payload[1..9].try_into().ok()?);
            let usdc_atomic = u64::from_le_bytes(payload[9..17].try_into().ok()?);
            let timelock_hours = u16::from_le_bytes(payload[17..19].try_into().ok()?);
            Some(ClientCommand::CreateOffer {
                tru_atoms,
                usdc_atomic,
                timelock_hours,
            })
        }
        CMD_TAKE_ORDER => {
            if payload.len() < 21 {
                return None;
            }
            let mut hash160 = [0u8; 20];
            hash160.copy_from_slice(&payload[1..21]);
            Some(ClientCommand::TakeOrder { hash160 })
        }
        other => Some(ClientCommand::Unknown(other)),
    }
}
