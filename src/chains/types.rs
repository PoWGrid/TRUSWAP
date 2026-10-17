use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChainId {
    BaseMainnet,
    BaseSepolia,
    ArbitrumSepolia,
    OptimismSepolia,
    EthereumSepolia,
    EthereumMainnet,
    Custom(u64),
}

impl ChainId {
    pub fn as_u64(&self) -> u64 {
        match self {
            ChainId::BaseMainnet => 8453,
            ChainId::BaseSepolia => 84532,
            ChainId::ArbitrumSepolia => 421614,
            ChainId::OptimismSepolia => 11155420,
            ChainId::EthereumSepolia => 11155111,
            ChainId::EthereumMainnet => 1,
            ChainId::Custom(id) => *id,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ChainId::BaseMainnet => "Base",
            ChainId::BaseSepolia => "Base",
            ChainId::ArbitrumSepolia => "Arbitrum",
            ChainId::OptimismSepolia => "Optimism",
            ChainId::EthereumSepolia => "Ethereum",
            ChainId::EthereumMainnet => "Ethereum",
            ChainId::Custom(_) => "Custom EVM",
        }
    }

    pub fn default_explorer(&self) -> &'static str {
        match self {
            ChainId::BaseMainnet => "https://basescan.org",
            ChainId::BaseSepolia => "https://sepolia.basescan.org",
            ChainId::ArbitrumSepolia => "https://sepolia.arbiscan.io",
            ChainId::OptimismSepolia => "https://sepolia-optimism.etherscan.io",
            ChainId::EthereumSepolia => "https://sepolia.etherscan.io",
            ChainId::EthereumMainnet => "https://etherscan.io",
            ChainId::Custom(_) => "https://basescan.org",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnChainHtlcEvent {
    pub contract_id: [u8; 32],
    pub hash160: [u8; 20],
    pub sender: String,
    pub receiver: String,
    pub amount: u64,
    pub timelock: u64,
    pub tx_hash: String,
    pub chain_id: ChainId,
}

#[derive(Debug)]
pub enum ChainError {
    RpcError(ChainId, String),
    ContractError(String),
    NetworkError(String),
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainError::RpcError(cid, msg) => write!(f, "RPC error on chain {cid:?}: {msg}"),
            ChainError::ContractError(msg) => write!(f, "Contract error: {msg}"),
            ChainError::NetworkError(msg) => write!(f, "Network connection failed: {msg}"),
        }
    }
}

impl std::error::Error for ChainError {}
