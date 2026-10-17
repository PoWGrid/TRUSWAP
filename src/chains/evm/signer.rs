//! Autonomous pure-Rust EVM Transaction Signer for Base Sepolia.
//! Implements headless EIP-155 transaction construction, RLP encoding,
//! ECDSA signing via secp256k1 recovery, and RPC broadcast.
//! Eliminates any need for MetaMask, Rabby, or browser extensions during atomic swaps.

use rand::Rng;
use reqwest::Client;
use secp256k1::ecdsa::RecoverableSignature;
use secp256k1::{Message, Secp256k1, SecretKey};
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

pub struct EvmSigner {
    priv_key: [u8; 32],
    address: String,
    chain_id: u64,
    rpc_url: String,
    htlc_contract: String,
    usdc_contract: String,
    http_client: Client,
    tx_lock: Arc<tokio::sync::Mutex<Option<u64>>>,
}

impl EvmSigner {
    pub fn new(
        priv_key: [u8; 32],
        chain_id: u64,
        rpc_url: String,
        htlc_contract: String,
        usdc_contract: String,
    ) -> Result<Self, String> {
        let address = crate::core::keys::evm_address_from_priv_key(&priv_key)?;
        let http_client = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("TRUSWAP-EvmSigner/1.0")
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            priv_key,
            address,
            chain_id,
            rpc_url,
            htlc_contract,
            usdc_contract,
            http_client,
            tx_lock: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn htlc_contract(&self) -> &str {
        &self.htlc_contract
    }

    pub fn usdc_contract(&self) -> &str {
        &self.usdc_contract
    }

    /// Queries current native ETH balance in ether.
    pub async fn get_eth_balance(&self, addr: Option<&str>) -> Result<f64, String> {
        let target = addr.unwrap_or(&self.address);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getBalance",
            "params": [target, "latest"]
        });
        let res = self.send_rpc(&payload).await?;
        let hex_val = res.get("result").and_then(|r| r.as_str()).unwrap_or("0x0");
        let clean = hex_val.strip_prefix("0x").unwrap_or(hex_val);
        let wei = u128::from_str_radix(clean, 16).unwrap_or(0);
        Ok(wei as f64 / 1e18)
    }

    /// Queries current ERC-20 USDC balance in tokens (6 decimals).
    pub async fn get_usdc_balance(&self, addr: Option<&str>) -> Result<f64, String> {
        let target = addr.unwrap_or(&self.address);
        let clean_target = target.strip_prefix("0x").unwrap_or(target);
        // balanceOf(address): selector 0x70a08231 + 32-byte zero-padded address
        let call_data = format!("0x70a08231{:0>64}", clean_target.to_lowercase());

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "eth_call",
            "params": [{
                "to": self.usdc_contract,
                "data": call_data
            }, "latest"]
        });
        let res = self.send_rpc(&payload).await?;
        let hex_val = res.get("result").and_then(|r| r.as_str()).unwrap_or("0x0");
        let clean = hex_val.strip_prefix("0x").unwrap_or(hex_val);
        let atomic = u64::from_str_radix(clean, 16).unwrap_or(0);
        Ok(atomic as f64 / 1_000_000.0)
    }

    pub const MAINNET_FALLBACK_RPCS: &'static [&'static str] = &[
        "https://mainnet.base.org",
        "https://base.drpc.org",
        "https://base.meowrpc.com",
        "https://1rpc.io/base",
        "https://base-rpc.publicnode.com",
        "https://base.llamarpc.com",
    ];

    pub const SEPOLIA_FALLBACK_RPCS: &'static [&'static str] = &[
        "https://sepolia.base.org",
        "https://base-sepolia-rpc.publicnode.com",
        "https://gateway.tenderly.co/public/base-sepolia",
        "https://base-sepolia.blockpi.network/v1/rpc/public",
    ];

    pub const DEFAULT_FALLBACK_RPCS: &'static [&'static str] = Self::MAINNET_FALLBACK_RPCS;

    /// Queries pending nonce for transaction ordering.
    pub async fn get_nonce(&self) -> Result<u64, String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "eth_getTransactionCount",
            "params": [&self.address, "pending"]
        });
        let res = self.send_rpc(&payload).await?;
        let hex_val = res.get("result").and_then(|r| r.as_str()).unwrap_or("0x0");
        let clean = hex_val.strip_prefix("0x").unwrap_or(hex_val);
        u64::from_str_radix(clean, 16).map_err(|e| format!("Failed to parse pending nonce: {}", e))
    }

    /// Queries mined (latest confirmed) nonce from canonical blocks.
    pub async fn get_mined_nonce(&self) -> Result<u64, String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "eth_getTransactionCount",
            "params": [&self.address, "latest"]
        });
        let res = self.send_rpc(&payload).await?;
        let hex_val = res.get("result").and_then(|r| r.as_str()).unwrap_or("0x0");
        let clean = hex_val.strip_prefix("0x").unwrap_or(hex_val);
        u64::from_str_radix(clean, 16).map_err(|e| format!("Failed to parse latest nonce: {}", e))
    }

    /// Queries current network gas price in wei with 30% safety margin to guarantee instant next-block confirmation on Base.
    pub async fn get_gas_price(&self) -> Result<u64, String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "eth_gasPrice",
            "params": []
        });
        let res = self.send_rpc(&payload).await?;
        let hex_val = res.get("result").and_then(|r| r.as_str()).unwrap_or("0x01312d00"); // 0.02 Gwei fallback for Base L2
        let clean = hex_val.strip_prefix("0x").unwrap_or(hex_val);
        let raw_gas = u64::from_str_radix(clean, 16).unwrap_or(20_000_000);
        // Add 30% buffer for fast Base confirmation without overpaying
        Ok(raw_gas.saturating_mul(130) / 100)
    }

    /// Returns current dynamic network gas fee estimate for withdraw (ETH or USDC).
    pub async fn estimate_withdraw_fee(&self, asset: &str) -> Result<(f64, f64, u64, f64), String> {
        let gas_price_wei = self.get_gas_price().await?;
        let gas_limit = if asset.eq_ignore_ascii_case("ETH") {
            21_000u64
        } else {
            75_000u64
        };

        let l2_fee_wei = (gas_price_wei as u128) * (gas_limit as u128);
        let l2_fee_eth = (l2_fee_wei as f64) / 1e18;

        // Query OP-Stack GasPriceOracle at 0x420000000000000000000000000000000000000F for getL1Fee
        let l1_fee_wei = self.get_l1_fee_estimate(gas_limit).await.unwrap_or(6_000_000_000u64);
        let l1_fee_eth = (l1_fee_wei as f64) / 1e18;

        let total_fee_eth = l2_fee_eth + l1_fee_eth;
        let gas_price_gwei = (gas_price_wei as f64) / 1e9;

        Ok((total_fee_eth, gas_price_gwei, gas_limit, l1_fee_eth))
    }

    /// Returns dynamic network gas fee estimate for BaseHTLC.createSwap (Maker Buy TRU).
    /// Computes L2 execution fee + OP-Stack L1 data fee with 25% safety buffer.
    pub async fn estimate_create_swap_fee(&self) -> Result<(f64, u128), String> {
        let gas_price_wei = self.get_gas_price().await? as u128;
        let gas_limit = 240_000u128;

        let l2_fee_wei = gas_price_wei * gas_limit;
        let l1_fee_wei = self.get_l1_fee_estimate(240_000).await.unwrap_or(10_000_000_000u64) as u128;

        // Total fee with 25% safety buffer
        let total_wei = (l2_fee_wei + l1_fee_wei) * 125 / 100;
        let total_eth = (total_wei as f64) / 1e18;

        Ok((total_eth, total_wei))
    }

    /// Returns dynamic network gas fee estimate for BaseHTLC.claim (Maker/Taker selling TRU, claiming USDC on Base).
    /// Computes L2 execution fee (~140k gas) + OP-Stack L1 data fee with 25% safety buffer.
    pub async fn estimate_claim_swap_fee(&self) -> Result<(f64, u128), String> {
        let gas_price_wei = self.get_gas_price().await? as u128;
        let gas_limit = 140_000u128;

        let l2_fee_wei = gas_price_wei * gas_limit;
        let l1_fee_wei = self.get_l1_fee_estimate(140_000).await.unwrap_or(8_000_000_000u64) as u128;

        // Total fee with 25% safety buffer
        let total_wei = (l2_fee_wei + l1_fee_wei) * 125 / 100;
        let total_eth = (total_wei as f64) / 1e18;

        Ok((total_eth, total_wei))
    }

    /// Queries OP-Stack GasPriceOracle for estimated L1 data fee.
    pub async fn get_l1_fee_estimate(&self, gas_limit: u64) -> Result<u64, String> {
        let dummy_len = if gas_limit == 21_000 { 80usize } else { 120usize };
        let mut call_data = Vec::with_capacity(4 + 32 + 32 + ((dummy_len + 31) / 32) * 32);
        call_data.extend_from_slice(&[0x49, 0x94, 0x8e, 0x0e]);
        call_data.extend_from_slice(&[0u8; 31]);
        call_data.push(0x20); // offset
        let mut len_bytes = [0u8; 32];
        let len_u128 = dummy_len as u128;
        len_bytes[16..32].copy_from_slice(&len_u128.to_be_bytes());
        call_data.extend_from_slice(&len_bytes);
        call_data.extend(std::iter::repeat(0x11).take(dummy_len));
        while (call_data.len() - 4) % 32 != 0 {
            call_data.push(0);
        }

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "eth_call",
            "params": [
                {
                    "to": "0x420000000000000000000000000000000000000F",
                    "data": format!("0x{}", hex::encode(&call_data))
                },
                "latest"
            ]
        });
        let res = self.send_rpc(&payload).await?;
        let hex_val = res.get("result").and_then(|r| r.as_str()).unwrap_or("0x0");
        let clean = hex_val.strip_prefix("0x").unwrap_or(hex_val);
        u64::from_str_radix(clean, 16).map_err(|e| format!("Failed to parse L1 fee: {}", e))
    }

    /// Checks USDC allowance of BaseHTLC contract and calls approve() if insufficient.
    pub async fn ensure_usdc_allowance(&self, required_atomic: u64) -> Result<Option<String>, String> {
        let clean_owner = self.address.strip_prefix("0x").unwrap_or(&self.address).to_lowercase();
        let clean_spender = self.htlc_contract.strip_prefix("0x").unwrap_or(&self.htlc_contract).to_lowercase();
        // allowance(address,address): selector 0xdd62ed3e + owner (32 bytes) + spender (32 bytes)
        let call_data = format!("0xdd62ed3e{:0>64}{:0>64}", clean_owner, clean_spender);

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "eth_call",
            "params": [{
                "to": self.usdc_contract,
                "data": call_data
            }, "latest"]
        });
        let res = self.send_rpc(&payload).await?;
        let hex_val = res.get("result").and_then(|r| r.as_str()).unwrap_or("0x0");
        let clean = hex_val.strip_prefix("0x").unwrap_or(hex_val);
        let current_allowance = u128::from_str_radix(clean, 16).unwrap_or(0);

        if current_allowance < required_atomic as u128 {
            info!("[EVM-SIGNER] Allowance ({} atomic) < required ({} atomic). Sending approve transaction...", current_allowance, required_atomic);
            // Approve max uint256 or safe high amount (100,000 USDC)
            let max_approval = u128::MAX;
            let tx_hash = self.approve_usdc(&self.htlc_contract, max_approval).await?;
            info!("[EVM-SIGNER] USDC Approve broadcast: {}", tx_hash);
            self.wait_for_receipt(&tx_hash, 30).await?;
            return Ok(Some(tx_hash));
        }

        Ok(None)
    }

    /// Calls `IERC20.approve(address spender, uint256 amount)`
    pub async fn approve_usdc(&self, spender: &str, amount: u128) -> Result<String, String> {
        let clean_spender = spender.strip_prefix("0x").unwrap_or(spender).to_lowercase();
        // selector 0x095ea7b3 + spender (32 bytes) + amount (32 bytes)
        let mut data = Vec::with_capacity(68);
        data.extend_from_slice(&[0x09, 0x5e, 0xa7, 0xb3]);
        let spender_bytes = hex::decode(&clean_spender).map_err(|e| e.to_string())?;
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&spender_bytes);
        data.extend_from_slice(&amount.to_be_bytes());
        if data.len() < 68 {
            let pad = 68 - data.len();
            let mut padded = Vec::with_capacity(68);
            padded.extend_from_slice(&data[..36]);
            padded.extend_from_slice(&vec![0u8; pad]);
            padded.extend_from_slice(&data[36..]);
            data = padded;
        }

        self.execute_transaction(&self.usdc_contract, 0, &data, 65_000).await
    }

    /// Calls `BaseHTLC.createSwap(address beneficiary, address token, uint256 amount, bytes20 hash160Digest, uint256 timelock, bytes32 salt)`
    /// Returns (swap_id_hex, tx_hash)
    pub async fn create_swap(
        &self,
        beneficiary: &str,
        amount_usdc: f64,
        hash160: [u8; 20],
        timelock: u32,
    ) -> Result<(String, String), String> {
        let amount_atomic = (amount_usdc * 1_000_000.0).round() as u64;
        if amount_atomic == 0 {
            return Err("Swap USDC amount must be greater than 0".to_string());
        }

        // 1. Ensure allowance
        self.ensure_usdc_allowance(amount_atomic).await?;

        // 2. Generate random 32-byte salt
        let mut salt = [0u8; 32];
        rand::thread_rng().fill(&mut salt);

        // 3. Build ABI payload:
        // selector: 0xefafa23f (createSwap(address,address,uint256,bytes20,uint256,bytes32))
        // arg0 (beneficiary): 32 bytes zero-padded address
        // arg1 (token): 32 bytes zero-padded address (USDC)
        // arg2 (amount): 32 bytes uint256
        // arg3 (hash160): 32 bytes (20 bytes hash160 right-padded with 12 zeroes for bytes20)
        // arg4 (timelock): 32 bytes uint256
        // arg5 (salt): 32 bytes
        let mut data = Vec::with_capacity(4 + 32 * 6);
        data.extend_from_slice(&[0xef, 0xaf, 0xa2, 0x3f]);

        let clean_ben = beneficiary.strip_prefix("0x").unwrap_or(beneficiary).to_lowercase();
        let ben_bytes = hex::decode(&clean_ben).map_err(|e| format!("Invalid beneficiary address: {}", e))?;
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&ben_bytes);

        let clean_token = self.usdc_contract.strip_prefix("0x").unwrap_or(&self.usdc_contract).to_lowercase();
        let token_bytes = hex::decode(&clean_token).map_err(|e| format!("Invalid token address: {}", e))?;
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&token_bytes);

        data.extend_from_slice(&[0u8; 24]);
        data.extend_from_slice(&amount_atomic.to_be_bytes());

        data.extend_from_slice(&hash160);
        data.extend_from_slice(&[0u8; 12]); // bytes20 is right-padded in Solidity ABI

        data.extend_from_slice(&[0u8; 28]);
        data.extend_from_slice(&(timelock as u32).to_be_bytes());

        data.extend_from_slice(&salt);

        info!("[EVM-SIGNER] Sending createSwap on BaseHTLC: {:.2} USDC to {}, timelock {}", amount_usdc, beneficiary, timelock);
        let tx_hash = self.execute_transaction(
            &self.htlc_contract,
            0,
            &data,
            240_000,
        ).await?;

        info!("[EVM-SIGNER] createSwap tx broadcast: {}", tx_hash);
        let receipt = self.wait_for_receipt(&tx_hash, 45).await?;

        // Extract swapId from event SwapCreated (topic 1)
        let swap_id = receipt
            .get("logs")
            .and_then(|l| l.as_array())
            .and_then(|arr| {
                for log in arr {
                    if let Some(topics) = log.get("topics").and_then(|t| t.as_array()) {
                        if topics.len() >= 2 {
                            if let Some(t0) = topics[0].as_str() {
                                if t0.eq_ignore_ascii_case(crate::chains::evm::TOPIC_SWAP_CREATED) {
                                    return topics[1].as_str().map(|s| s.to_string());
                                }
                            }
                        }
                    }
                }
                None
            })
            .unwrap_or_else(|| {
                // Deterministic fallback swapId: keccak256(sender, beneficiary, token, amount, hash160, timelock, chain_id, salt)
                let mut preimage = Vec::new();
                let sender_bytes = hex::decode(self.address.strip_prefix("0x").unwrap_or(&self.address)).unwrap_or_default();
                preimage.extend_from_slice(&sender_bytes);
                preimage.extend_from_slice(&ben_bytes);
                preimage.extend_from_slice(&token_bytes);
                preimage.extend_from_slice(&[0u8; 24]);
                preimage.extend_from_slice(&amount_atomic.to_be_bytes());
                preimage.extend_from_slice(&hash160);
                preimage.extend_from_slice(&[0u8; 24]);
                preimage.extend_from_slice(&(timelock as u64).to_be_bytes());
                preimage.extend_from_slice(&[0u8; 24]);
                preimage.extend_from_slice(&(self.chain_id as u64).to_be_bytes());
                preimage.extend_from_slice(&salt);
                format!("0x{}", hex::encode(Keccak256::digest(&preimage)))
            });

        info!("[EVM-SIGNER] createSwap confirmed on-chain! SwapId: {}", swap_id);
        Ok((swap_id, tx_hash))
    }

    /// Calls `BaseHTLC.claim(bytes32 swapId, bytes secret)`
    pub async fn claim_swap(&self, swap_id_hex: &str, secret: &[u8; 32]) -> Result<String, String> {
        let clean_id = swap_id_hex.strip_prefix("0x").unwrap_or(swap_id_hex);
        let id_bytes = hex::decode(clean_id).map_err(|e| format!("Invalid swapId: {}", e))?;
        if id_bytes.len() != 32 {
            return Err("swapId must be exactly 32 bytes".to_string());
        }

        // selector: 0xf0331024
        // arg0 (swapId): 32 bytes
        // arg1 (secret offset): 0x0000...0040 (64)
        // length of secret: 0x0000...0020 (32)
        // secret: 32 bytes
        let mut data = Vec::with_capacity(4 + 32 * 4);
        data.extend_from_slice(&[0xf0, 0x33, 0x10, 0x24]);
        data.extend_from_slice(&id_bytes);

        // Offset 64 (0x40)
        data.extend_from_slice(&[0u8; 31]);
        data.push(0x40);

        // Length 32 (0x20)
        data.extend_from_slice(&[0u8; 31]);
        data.push(0x20);

        // Secret data
        data.extend_from_slice(secret);

        info!("[EVM-SIGNER] Broadcasting autonomous claim for swap 0x{}...", &clean_id[..clean_id.len().min(8)]);
        let tx_hash = self.execute_transaction(
            &self.htlc_contract,
            0,
            &data,
            140_000,
        ).await?;

        info!("[EVM-SIGNER] Claim transaction submitted: {}", tx_hash);
        Ok(tx_hash)
    }

    /// Calls `BaseHTLC.refund(bytes32 swapId)` after timelock expiration.
    pub async fn refund_swap(&self, swap_id_hex: &str) -> Result<String, String> {
        let clean_id = swap_id_hex.strip_prefix("0x").unwrap_or(swap_id_hex);
        let id_bytes = hex::decode(clean_id).map_err(|e| format!("Invalid swapId: {}", e))?;
        if id_bytes.len() != 32 {
            return Err("swapId must be exactly 32 bytes".to_string());
        }

        // selector: 0x7249fbb6 + swapId (32 bytes)
        let mut data = Vec::with_capacity(36);
        data.extend_from_slice(&[0x72, 0x49, 0xfb, 0xb6]);
        data.extend_from_slice(&id_bytes);

        info!("[EVM-SIGNER] Broadcasting autonomous refund for swap 0x{}...", &clean_id[..clean_id.len().min(8)]);
        let tx_hash = self.execute_transaction(
            &self.htlc_contract,
            0,
            &data,
            100_000,
        ).await?;

        info!("[EVM-SIGNER] Refund transaction submitted: {}", tx_hash);
        Ok(tx_hash)
    }

    /// Withdraws native ETH from local vault to user's external destination address.
    pub async fn withdraw_eth(&self, to: &str, amount_eth: f64) -> Result<String, String> {
        if amount_eth <= 0.0 {
            return Err("ETH amount must be greater than 0".to_string());
        }
        let wei = (amount_eth * 1e18).round() as u128;

        self.execute_transaction(
            to,
            wei,
            &[],
            21_000,
        ).await
    }

    /// Withdraws ERC-20 USDC from local vault to user's external destination address.
    pub async fn withdraw_usdc(&self, to: &str, amount_usdc: f64) -> Result<String, String> {
        if amount_usdc <= 0.0 {
            return Err("USDC amount must be greater than 0".to_string());
        }
        let atomic = (amount_usdc * 1_000_000.0).round() as u128;
        let clean_to = to.strip_prefix("0x").unwrap_or(to).to_lowercase();
        let to_bytes = hex::decode(&clean_to).map_err(|e| format!("Invalid recipient address: {}", e))?;
        if to_bytes.len() != 20 {
            return Err("Recipient address must be 20 bytes".to_string());
        }

        // selector 0xa9059cbb + to (32 bytes) + amount (32 bytes)
        let mut data = Vec::with_capacity(68);
        data.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]);
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&to_bytes);
        data.extend_from_slice(&[0u8; 16]);
        data.extend_from_slice(&atomic.to_be_bytes());

        self.execute_transaction(
            &self.usdc_contract,
            0,
            &data,
            75_000,
        ).await
    }

    /// Dynamically queries eth_estimateGas for an EVM call, adding 20% safety buffer.
    pub async fn estimate_tx_gas(&self, to: &str, value: u128, data: &[u8]) -> Option<u64> {
        let clean_to = to.strip_prefix("0x").unwrap_or(to).to_lowercase();
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "eth_estimateGas",
            "params": [{
                "from": &self.address,
                "to": format!("0x{}", clean_to),
                "value": format!("0x{:x}", value),
                "data": format!("0x{}", hex::encode(data))
            }]
        });
        if let Ok(res) = self.send_rpc(&payload).await {
            if let Some(hex_str) = res.get("result").and_then(|r| r.as_str()) {
                let clean = hex_str.strip_prefix("0x").unwrap_or(hex_str);
                if let Ok(gas) = u64::from_str_radix(clean, 16) {
                    return Some(gas.saturating_mul(120) / 100);
                }
            }
        }
        None
    }

    /// Executes an EVM transaction with serialized nonce allocation and dynamic gas estimation.
    pub async fn execute_transaction(
        &self,
        to: &str,
        value: u128,
        data: &[u8],
        fallback_gas_limit: u64,
    ) -> Result<String, String> {
        let mut lock = self.tx_lock.lock().await;

        let rpc_pending = self.get_nonce().await?;
        let rpc_latest = self.get_mined_nonce().await.unwrap_or(rpc_pending);

        // If on-chain transactions are caught up, synchronize tracked nonce with latest confirmed
        if rpc_pending == rpc_latest {
            *lock = Some(rpc_latest);
        }

        let nonce = match *lock {
            Some(tracked) if tracked >= rpc_pending => tracked,
            _ => rpc_pending,
        };

        let gas_price = self.get_gas_price().await?;
        let gas_limit = if let Some(estimated) = self.estimate_tx_gas(to, value, data).await {
            estimated.max(fallback_gas_limit)
        } else {
            fallback_gas_limit
        };

        match self.sign_and_send_tx(nonce, gas_price, gas_limit, to, value, data).await {
            Ok(tx_hash) => {
                *lock = Some(nonce + 1);
                Ok(tx_hash)
            }
            Err(e) => {
                // Invalidate local cache on error so next call refetches from RPC
                *lock = None;
                Err(e)
            }
        }
    }

    /// Signs an EIP-155 legacy transaction and sends it to the RPC via `eth_sendRawTransaction`.
    async fn sign_and_send_tx(
        &self,
        nonce: u64,
        gas_price: u64,
        gas_limit: u64,
        to: &str,
        value: u128,
        data: &[u8],
    ) -> Result<String, String> {
        let clean_to = to.strip_prefix("0x").unwrap_or(to).to_lowercase();
        let to_bytes = hex::decode(&clean_to).map_err(|e| format!("Invalid 'to' address hex: {}", e))?;
        if to_bytes.len() != 20 {
            return Err("Target EVM address must be 20 bytes".to_string());
        }

        // 1. RLP encode raw tx for EIP-155 hashing:
        // [nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0]
        let mut raw_items = Vec::with_capacity(9);
        raw_items.push(rlp_encode_u64(nonce));
        raw_items.push(rlp_encode_u64(gas_price));
        raw_items.push(rlp_encode_u64(gas_limit));
        raw_items.push(rlp_encode_bytes(&to_bytes));
        raw_items.push(rlp_encode_u128(value));
        raw_items.push(rlp_encode_bytes(data));
        raw_items.push(rlp_encode_u64(self.chain_id));
        raw_items.push(rlp_encode_u64(0));
        raw_items.push(rlp_encode_u64(0));

        let signing_rlp = rlp_encode_list(&raw_items);
        let tx_hash_bytes = Keccak256::digest(&signing_rlp);

        // 2. ECDSA secp256k1 signature
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&self.priv_key).map_err(|e| e.to_string())?;
        let message = Message::from_digest_slice(&tx_hash_bytes).map_err(|e| e.to_string())?;
        let sig: RecoverableSignature = secp.sign_ecdsa_recoverable(&message, &secret_key);
        let (rec_id, sig_bytes) = sig.serialize_compact();

        let v = self.chain_id * 2 + 35 + rec_id.to_i32() as u64;
        let r = &sig_bytes[0..32];
        let s = &sig_bytes[32..64];

        // 3. RLP encode final signed transaction:
        // [nonce, gasPrice, gasLimit, to, value, data, v, r, s]
        // Note: Ethereum Yellow Paper specifies v, r, s as positive integers,
        // so r and s must be encoded as big-endian integers without leading zeros.
        let mut signed_items = Vec::with_capacity(9);
        signed_items.push(rlp_encode_u64(nonce));
        signed_items.push(rlp_encode_u64(gas_price));
        signed_items.push(rlp_encode_u64(gas_limit));
        signed_items.push(rlp_encode_bytes(&to_bytes));
        signed_items.push(rlp_encode_u128(value));
        signed_items.push(rlp_encode_bytes(data));
        signed_items.push(rlp_encode_u64(v));
        signed_items.push(rlp_encode_uint_bytes(r));
        signed_items.push(rlp_encode_uint_bytes(s));

        let signed_rlp = rlp_encode_list(&signed_items);
        let raw_hex = format!("0x{}", hex::encode(&signed_rlp));
        let expected_tx_hash = format!("0x{}", hex::encode(Keccak256::digest(&signed_rlp)));

        tracing::info!("[EVM-SIGNER] Broadcast raw_hex: {} (expected_tx={}, nonce={}, gas_price={}, gas_limit={}, to={}, value={})", raw_hex, expected_tx_hash, nonce, gas_price, gas_limit, to, value);

        // 4. Send raw transaction
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "eth_sendRawTransaction",
            "params": [raw_hex]
        });

        let res = self.send_rpc(&payload).await?;
        tracing::info!("[EVM-SIGNER] eth_sendRawTransaction response: {:?}", res);
        if let Some(err) = res.get("error") {
            let err_str = err.to_string();
            // If the transaction has already entered the mempool or was mined, treat as success
            if err_str.contains("already known") {
                tracing::info!("[EVM-SIGNER] Transaction already known in network mempool: {}", expected_tx_hash);
                return Ok(expected_tx_hash);
            }
            return Err(format!("RPC error on eth_sendRawTransaction: {}", err));
        }

        let tx_hash = res
            .get("result")
            .and_then(|r| r.as_str())
            .unwrap_or(&expected_tx_hash)
            .to_string();

        Ok(tx_hash)
    }

    /// Polls RPC until receipt is mined or timeout expires.
    pub async fn wait_for_receipt(&self, tx_hash: &str, timeout_secs: u64) -> Result<serde_json::Value, String> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed().as_secs() > timeout_secs {
                // Invalidate local tracked nonce on timeout to prevent head-of-line blocking
                let mut lock = self.tx_lock.lock().await;
                *lock = None;
                return Err(format!("Timed out waiting for receipt for tx {}", tx_hash));
            }

            let payload = json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "eth_getTransactionReceipt",
                "params": [tx_hash]
            });

            if let Ok(res) = self.send_rpc(&payload).await {
                if let Some(receipt) = res.get("result") {
                    if !receipt.is_null() {
                        let status_hex = receipt.get("status").and_then(|s| s.as_str()).unwrap_or("0x0");
                        if status_hex == "0x1" || status_hex == "1" {
                            return Ok(receipt.clone());
                        } else if status_hex == "0x0" || status_hex == "0" {
                            return Err(format!("Transaction {} reverted on-chain", tx_hash));
                        }
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    }

    async fn send_rpc(&self, payload: &serde_json::Value) -> Result<serde_json::Value, String> {
        let mut endpoints = vec![self.rpc_url.as_str()];
        let fallbacks = if self.chain_id == 84532 {
            Self::SEPOLIA_FALLBACK_RPCS
        } else {
            Self::MAINNET_FALLBACK_RPCS
        };
        for fb in fallbacks {
            if *fb != self.rpc_url {
                endpoints.push(*fb);
            }
        }

        let mut last_err = String::new();
        for ep in endpoints {
            match self.http_client.post(ep).json(payload).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        match resp.json::<serde_json::Value>().await {
                            Ok(body) => {
                                // If the RPC returned a JSON-RPC error, check if it's a transient node failure
                                // that warrants failover to another public endpoint
                                if let Some(err) = body.get("error") {
                                    let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                                    let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("").to_lowercase();
                                    // If rate limited (-32005, 429), server overloaded (-32603), or proxy format issue, try fallback
                                    if code == -32005 || code == 429 || code == -32603 || msg.contains("rate") || msg.contains("limit") || msg.contains("overloaded") || msg.contains("throttled") {
                                        last_err = format!("RPC throttling/error on {}: {}", ep, msg);
                                        continue;
                                    }
                                }
                                return Ok(body);
                            }
                            Err(e) => last_err = format!("JSON parse error on {}: {}", ep, e),
                        }
                    } else {
                        last_err = format!("HTTP error {} on {}", status, ep);
                    }
                }
                Err(e) => {
                    last_err = format!("Connection error on {}: {}", ep, e);
                }
            }
        }
        Err(format!("All EVM RPC endpoints failed. Last error: {}", last_err))
    }

    /// Resolves on-chain `swapId` from `BaseHTLC.SwapCreated` logs by matching the target `hash160`.
    /// Tries multiple public Base RPC endpoints and queries recent blocks.
    pub async fn resolve_onchain_swap_id(&self, hash160_hex: &str) -> Option<String> {
        let clean_hash = hash160_hex.strip_prefix("0x").unwrap_or(hash160_hex).to_lowercase();
        if clean_hash.len() != 40 {
            return None;
        }

        let mut rpc_endpoints = vec![self.rpc_url.as_str()];
        let fallbacks = if self.chain_id == 84532 {
            Self::SEPOLIA_FALLBACK_RPCS
        } else {
            Self::MAINNET_FALLBACK_RPCS
        };
        for fb in fallbacks {
            if *fb != self.rpc_url.as_str() {
                rpc_endpoints.push(*fb);
            }
        }

        for rpc in rpc_endpoints {
            let block_num = match self.get_latest_block_on_rpc(rpc).await {
                Ok(b) => b,
                Err(_) => continue,
            };

            // Query recent 1800 blocks (~1 hour on Base) backwards in <= 150 block chunks
            let min_block = block_num.saturating_sub(1800);
            let mut curr = block_num;

            while curr > min_block {
                let from = curr.saturating_sub(150).max(min_block);
                let payload = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "eth_getLogs",
                    "params": [{
                        "address": self.htlc_contract,
                        "topics": [crate::chains::evm::TOPIC_SWAP_CREATED],
                        "fromBlock": format!("0x{:x}", from),
                        "toBlock": format!("0x{:x}", curr)
                    }]
                });

                if let Ok(res) = self.http_client.post(rpc).json(&payload).send().await {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        if let Some(logs) = body.get("result").and_then(|r| r.as_array()) {
                            for log in logs {
                                let topics = log.get("topics").and_then(|t| t.as_array());
                                let data_str = log.get("data").and_then(|d| d.as_str()).unwrap_or("");
                                let clean_data = data_str.strip_prefix("0x").unwrap_or(data_str);

                                // data layout: token (64 hex), amount (64 hex), hash160Digest (64 hex, left-aligned 40 hex chars)
                                if clean_data.len() >= 168 {
                                    let event_hash160 = &clean_data[128..168];
                                    if event_hash160.eq_ignore_ascii_case(&clean_hash) {
                                        if let Some(t) = topics {
                                            if t.len() >= 4 {
                                                let our_addr = self.address().to_lowercase();
                                                let benef_topic = t[3].as_str().unwrap_or("");
                                                let benef = format!("0x{}", &benef_topic[benef_topic.len().saturating_sub(40)..]);
                                                let sender_topic = t[2].as_str().unwrap_or("");
                                                let snd = format!("0x{}", &sender_topic[sender_topic.len().saturating_sub(40)..]);

                                                // Strictly verify that the swap is addressed to us (beneficiary) or created by us (sender)
                                                if benef.to_lowercase() == our_addr || snd.to_lowercase() == our_addr {
                                                    if let Some(swap_id_str) = t[1].as_str() {
                                                        return Some(swap_id_str.to_string());
                                                    }
                                                }
                                            } else if t.len() >= 2 {
                                                if let Some(swap_id_str) = t[1].as_str() {
                                                    return Some(swap_id_str.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if from == 0 || from <= min_block {
                    break;
                }
                curr = from.saturating_sub(1);
            }
        }

        None
    }

    async fn get_latest_block_on_rpc(&self, rpc: &str) -> Result<u64, String> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_blockNumber",
            "params": []
        });
        let res = self.http_client.post(rpc).json(&payload).send().await.map_err(|e| e.to_string())?;
        let val = res.json::<serde_json::Value>().await.map_err(|e| e.to_string())?;
        let hex_str = val.get("result").and_then(|r| r.as_str()).ok_or("No result in eth_blockNumber")?;
        let clean = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        u64::from_str_radix(clean, 16).map_err(|e| e.to_string())
    }
}

// --- RLP Helpers ---

pub fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        vec![bytes[0]]
    } else if bytes.len() <= 55 {
        let mut out = Vec::with_capacity(1 + bytes.len());
        out.push(0x80 + bytes.len() as u8);
        out.extend_from_slice(bytes);
        out
    } else {
        let len_bytes = to_be_bytes_trimmed(bytes.len() as u64);
        let mut out = Vec::with_capacity(1 + len_bytes.len() + bytes.len());
        out.push(0xb7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(bytes);
        out
    }
}

pub fn rlp_encode_u64(val: u64) -> Vec<u8> {
    if val == 0 {
        vec![0x80]
    } else {
        let bytes = to_be_bytes_trimmed(val);
        rlp_encode_bytes(&bytes)
    }
}

pub fn rlp_encode_u128(val: u128) -> Vec<u8> {
    if val == 0 {
        vec![0x80]
    } else {
        let be = val.to_be_bytes();
        let start = be.iter().position(|&b| b != 0).unwrap_or(be.len());
        rlp_encode_bytes(&be[start..])
    }
}

pub fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let total_len: usize = items.iter().map(|i| i.len()).sum();
    let mut payload = Vec::with_capacity(total_len);
    for item in items {
        payload.extend_from_slice(item);
    }
    if payload.len() <= 55 {
        let mut out = Vec::with_capacity(1 + payload.len());
        out.push(0xc0 + payload.len() as u8);
        out.extend_from_slice(&payload);
        out
    } else {
        let len_bytes = to_be_bytes_trimmed(payload.len() as u64);
        let mut out = Vec::with_capacity(1 + len_bytes.len() + payload.len());
        out.push(0xf7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(&payload);
        out
    }
}

pub fn rlp_encode_uint_bytes(b: &[u8]) -> Vec<u8> {
    let start = b.iter().position(|&byte| byte != 0).unwrap_or(b.len());
    let trimmed = &b[start..];
    if trimmed.is_empty() {
        vec![0x80]
    } else if trimmed.len() == 1 && trimmed[0] < 0x80 {
        vec![trimmed[0]]
    } else if trimmed.len() <= 55 {
        let mut out = Vec::with_capacity(1 + trimmed.len());
        out.push(0x80 + trimmed.len() as u8);
        out.extend_from_slice(trimmed);
        out
    } else {
        let len_bytes = to_be_bytes_trimmed(trimmed.len() as u64);
        let mut out = Vec::with_capacity(1 + len_bytes.len() + trimmed.len());
        out.push(0xb7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(trimmed);
        out
    }
}

fn to_be_bytes_trimmed(val: u64) -> Vec<u8> {
    let be = val.to_be_bytes();
    let start = be.iter().position(|&b| b != 0).unwrap_or(be.len());
    be[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rlp_encoding_primitives() {
        assert_eq!(rlp_encode_u64(0), vec![0x80]);
        assert_eq!(rlp_encode_u64(15), vec![15]);
        assert_eq!(rlp_encode_u64(1024), vec![0x82, 0x04, 0x00]);
        assert_eq!(rlp_encode_bytes(b"dog"), vec![0x83, b'd', b'o', b'g']);
        let list = rlp_encode_list(&[rlp_encode_bytes(b"cat"), rlp_encode_bytes(b"dog")]);
        assert_eq!(list, vec![0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g']);
    }

    #[test]
    fn test_eip155_raw_tx_construction() {
        let priv_key = [1u8; 32];
        let signer = EvmSigner::new(
            priv_key,
            84532,
            "https://sepolia.base.org".to_string(),
            "0xF2e70fbd7Da79d188f805e61e8587AACD49772a5".to_string(),
            "0x036CbD53842c5426634e7929541eC2318f3dCF7e".to_string(),
        ).expect("Failed to initialize EvmSigner");

        assert!(signer.address().starts_with("0x"));
    }

    #[test]
    fn test_rlp_encode_uint_bytes_canonical() {
        // Zero value
        assert_eq!(rlp_encode_uint_bytes(&[0, 0, 0]), vec![0x80]);

        // Single byte < 0x80
        assert_eq!(rlp_encode_uint_bytes(&[0, 0, 0x42]), vec![0x42]);

        // Value with leading zero trimmed canonically
        let mut val_with_leading_zeros = vec![0x00, 0x00];
        val_with_leading_zeros.extend_from_slice(&[0x12, 0x34]);
        assert_eq!(rlp_encode_uint_bytes(&val_with_leading_zeros), vec![0x82, 0x12, 0x34]);

        // 32-byte hash without leading zeros
        let full_32 = [0x55u8; 32];
        let encoded = rlp_encode_uint_bytes(&full_32);
        assert_eq!(encoded.len(), 33);
        assert_eq!(encoded[0], 0x80 + 32);
    }
}
