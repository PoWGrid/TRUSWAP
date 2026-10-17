use prost::Message;
use sha2::{Digest, Sha256};

pub const MAINNET_P2P_MAGIC: [u8; 4] = [0xec, 0x7c, 0x8f, 0xb9];
pub const MAINNET_NETWORK_ID: &str = "tru-mainnet";
pub const DEFAULT_P2P_PORT: u16 = 21833;
pub const HEADER_SIZE: usize = 4 + 4 + 4 + 32; // Magic(4) + Type(4) + Len(4) + Checksum(32) = 44 bytes
pub const MAX_FRAME_PAYLOAD: usize = 64 * 1024 * 1024; // 64 MB matching TRU consensus max block capacity

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum MessageType {
    Version = 0,
    VerAck = 1,
    Addr = 2,
    Inv = 3,
    GetData = 4,
    Block = 5,
    Tx = 6,
    Ping = 7,
    Pong = 8,
    Chat = 9,
    Height = 10,
    GetBlock = 11,
    GetHeight = 12,
    BlockNotFound = 13,
}

impl MessageType {
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Version),
            1 => Some(Self::VerAck),
            2 => Some(Self::Addr),
            3 => Some(Self::Inv),
            4 => Some(Self::GetData),
            5 => Some(Self::Block),
            6 => Some(Self::Tx),
            7 => Some(Self::Ping),
            8 => Some(Self::Pong),
            9 => Some(Self::Chat),
            10 => Some(Self::Height),
            11 => Some(Self::GetBlock),
            12 => Some(Self::GetHeight),
            13 => Some(Self::BlockNotFound),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BaseMessage {
    #[prost(int32, tag = "1")]
    pub r#type: i32,
    #[prost(bytes = "vec", tag = "2")]
    pub payload: Vec<u8>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Version {
    #[prost(uint32, tag = "1")]
    pub version: u32,
    #[prost(string, tag = "2")]
    pub sub_version: String,
    #[prost(uint64, tag = "3")]
    pub services: u64,
    #[prost(uint64, tag = "4")]
    pub timestamp: u64,
    #[prost(string, tag = "5")]
    pub addr_recv: String,
    #[prost(string, tag = "6")]
    pub addr_from: String,
    #[prost(uint64, tag = "7")]
    pub nonce: u64,
    #[prost(string, tag = "8")]
    pub user_agent: String,
    #[prost(uint32, tag = "9")]
    pub start_height: u32,
    #[prost(string, tag = "10")]
    pub future_flag: String,
    #[prost(string, tag = "11")]
    pub network_id: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct VerAck {
    #[prost(string, tag = "1")]
    pub quantum_ack: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Addr {
    #[prost(string, repeated, tag = "1")]
    pub addresses: Vec<String>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Inv {
    #[prost(string, repeated, tag = "1")]
    pub inventory: Vec<String>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetData {
    #[prost(string, repeated, tag = "1")]
    pub inventory: Vec<String>,
}

#[derive(Clone, PartialEq, ::prost::Message, serde::Serialize, serde::Deserialize)]
pub struct BlockProto {
    #[prost(string, tag = "1")]
    pub hash: String,
    #[prost(string, tag = "2")]
    pub header: String,
    #[prost(string, repeated, tag = "3")]
    pub transactions: Vec<String>,
    #[prost(uint64, tag = "4")]
    pub chain_work: u64,
    #[prost(uint32, tag = "5")]
    pub height: u32,
    #[prost(string, tag = "6")]
    pub token_metadata: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TxProto {
    #[prost(string, tag = "1")]
    pub txid: String,
    #[prost(bytes = "vec", tag = "2")]
    pub raw: Vec<u8>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Ping {
    #[prost(uint64, tag = "1")]
    pub nonce: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Pong {
    #[prost(uint64, tag = "1")]
    pub nonce: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct HeightMessage {
    #[prost(uint64, tag = "1")]
    pub height: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockRequest {
    #[prost(uint64, tag = "1")]
    pub height: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetHeightRequest {
    #[prost(uint64, tag = "1")]
    pub timestamp: u64,
    #[prost(string, tag = "2")]
    pub requester_id: String,
}

/// Computes SHA256 checksum of data
pub fn compute_sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Encodes a BaseMessage into a TRU Patch 15B.1 wire frame:
/// [4B Magic][4B LE Type][4B LE Length][32B SHA256][Payload bytes (serialized BaseMessage)]
pub fn serialize_frame(msg: &BaseMessage) -> Vec<u8> {
    let mut payload = Vec::new();
    msg.encode(&mut payload).expect("Protobuf encoding cannot fail");

    let checksum = compute_sha256(&payload);
    let msg_type = msg.r#type as u32;
    let length = payload.len() as u32;

    let mut frame = Vec::with_capacity(HEADER_SIZE + payload.len());
    frame.extend_from_slice(&MAINNET_P2P_MAGIC);
    frame.extend_from_slice(&msg_type.to_le_bytes());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&checksum);
    frame.extend_from_slice(&payload);
    frame
}

/// Helper to wrap an inner protobuf message into a BaseMessage and frame it.
pub fn create_framed_message<T: ::prost::Message>(msg_type: MessageType, inner: &T) -> Vec<u8> {
    let mut inner_bytes = Vec::new();
    inner.encode(&mut inner_bytes).expect("Protobuf encode fail");
    let base = BaseMessage {
        r#type: msg_type as i32,
        payload: inner_bytes,
    };
    serialize_frame(&base)
}

/// Tries to parse a complete frame from buffer.
/// Returns Ok(Some((msg, bytes_consumed))) when complete,
/// Ok(None) when more data is needed,
/// Err(e) when magic or checksum is invalid.
pub fn deserialize_frame(buf: &[u8]) -> Result<Option<(BaseMessage, usize)>, String> {
    if buf.len() < 4 {
        return Ok(None);
    }

    if buf[0..4] != MAINNET_P2P_MAGIC {
        return Err(format!(
            "Invalid TRU P2P magic: {:02x?}",
            &buf[0..4]
        ));
    }

    if buf.len() < HEADER_SIZE {
        return Ok(None);
    }

    let raw_type = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let length = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;

    if length > MAX_FRAME_PAYLOAD {
        return Err(format!("Frame declared oversized payload: {} bytes (max allowed: {})", length, MAX_FRAME_PAYLOAD));
    }

    let total_size = HEADER_SIZE + length;
    if buf.len() < total_size {
        return Ok(None);
    }

    let received_checksum = &buf[12..44];
    let payload = &buf[HEADER_SIZE..total_size];
    let computed_checksum = compute_sha256(payload);

    if received_checksum != computed_checksum {
        return Err("SHA-256 frame checksum mismatch".to_string());
    }

    let base_msg = BaseMessage::decode(payload)
        .map_err(|e| format!("BaseMessage decode error: {}", e))?;

    if base_msg.r#type as u32 != raw_type {
        return Err(format!(
            "Type mismatch: header declared {}, inner declared {}",
            raw_type, base_msg.r#type
        ));
    }

    Ok(Some((base_msg, total_size)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_framing_roundtrip() {
        let version = Version {
            version: 2025,
            sub_version: "TRUSWAP-LE/1.0.0".to_string(),
            services: 1,
            timestamp: 1745982427,
            addr_recv: "127.0.0.1:21833".to_string(),
            addr_from: "0.0.0.0:0".to_string(),
            nonce: 999888777,
            user_agent: "TRUSWAP-LE".to_string(),
            start_height: 105,
            future_flag: "TestFlag=1".to_string(),
            network_id: "tru-mainnet".to_string(),
        };

        let frame = create_framed_message(MessageType::Version, &version);
        assert!(frame.len() > HEADER_SIZE);
        assert_eq!(&frame[0..4], &MAINNET_P2P_MAGIC);

        let parsed = deserialize_frame(&frame).expect("Should deserialize frame");
        assert!(parsed.is_some());
        let (base_msg, consumed) = parsed.unwrap();
        assert_eq!(consumed, frame.len());
        assert_eq!(base_msg.r#type, MessageType::Version as i32);

        let decoded_version = Version::decode(&base_msg.payload[..]).expect("Decode version");
        assert_eq!(decoded_version.version, 2025);
        assert_eq!(decoded_version.network_id, "tru-mainnet");
        assert_eq!(decoded_version.start_height, 105);
    }

    #[test]
    fn test_corrupted_checksum_fails() {
        let ping = Ping { nonce: 12345 };
        let mut frame = create_framed_message(MessageType::Ping, &ping);
        // Corrupt payload
        let last = frame.len() - 1;
        frame[last] ^= 0xff;

        let res = deserialize_frame(&frame);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("checksum mismatch"));
    }
}
