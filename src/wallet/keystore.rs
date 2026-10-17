//! Encrypted and compressed binary storage for TRU wallet keys and atomic swap preimages.
//! Prevents theft, inspection, or unauthorized plain-text reads of secret keys.
//! All sensitive wallet secrets and swap states are kept in a single compressed, encrypted .dat file.

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use crate::core::keys::{
    derive_base_key, derive_base_mnemonic_12, derive_base_wallet_from_mnemonic,
    evm_address_from_priv_key, generate_base_mnemonic_12, generate_keypair,
    keypair_from_bytes, priv_key_to_mnemonic, priv_key_to_wif, KeyPair,
};

const MAGIC_HEADER_V4: &[u8; 8] = b"TRUSWAP\x04";
const MAGIC_HEADER_V3: &[u8; 8] = b"TRUSWAP\x03";
const MAGIC_HEADER_V2: &[u8; 8] = b"TRUSWAP\x02";
const MAGIC_HEADER_V1: &[u8; 8] = b"TRUSWAP\x01";
const SALT_SIZE: usize = 16;
const HMAC_SIZE: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct VaultWalletData {
    pub priv_key: [u8; 32],
    pub address: String,
    pub base_priv_key: [u8; 32],
    pub base_address: String,
    pub base_mnemonic: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletExport {
    pub address: String,
    pub pub_key_hex: String,
    pub priv_key_hex: String,
    pub priv_key_wif: String,
    pub mnemonic: String,
    pub base_address: String,
    pub base_priv_key_hex: String,
    pub base_mnemonic: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwapRecord {
    pub swap_id: [u8; 32],
    pub secret: [u8; 32],
    pub hash160: [u8; 20],
    pub direction: u8, // 1 = Sell TRU -> Get USDC, 2 = Lock USDC -> Buy TRU
    pub step: u8,      // 1 = Secret Gen, 2 = Source Lock, 3 = Target Lock, 4 = Claimed
    pub status: u8,    // 0 = Pending, 1 = Claimed/Settled, 2 = Refunded
    pub tru_atoms: u64,
    pub usdc_atomic: u64,
    pub timelock: u32,
    pub created_at: u64,
    pub beneficiary: String,
}

pub struct EncryptedVault {
    file_path: PathBuf,
    master_key: [u8; 32],
    wallet: Option<VaultWalletData>,
    records: Vec<SwapRecord>,
}

impl EncryptedVault {
    /// Opens or creates the encrypted compressed vault.
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(data_dir).map_err(|e| format!("Failed to create data dir: {}", e))?;

        let key_path = data_dir.join(".vault.key");
        let master_key = Self::load_or_create_key(&key_path)?;

        let file_path = data_dir.join("vault.dat");
        let mut vault = Self {
            file_path,
            master_key,
            wallet: None,
            records: Vec::new(),
        };

        // Load existing vault.dat if present
        if vault.file_path.exists() {
            vault.load_from_disk()?;
        }

        // Automatic legacy migration:
        // 1. If legacy swaps.dat exists and no records in vault.dat, migrate them
        let legacy_swaps = data_dir.join("swaps.dat");
        if legacy_swaps.exists() && vault.records.is_empty() {
            if let Ok(records) = vault.load_legacy_swaps(&legacy_swaps) {
                vault.records = records;
                let _ = fs::remove_file(&legacy_swaps);
                info!("[VAULT] Migrated legacy swaps.dat into encrypted vault.dat");
            }
        }

        // 2. If legacy plain-text vault_key.hex exists, migrate into encrypted vault.dat and wipe plaintext files!
        let legacy_key = data_dir.join("vault_key.hex");
        let legacy_addr = data_dir.join("swap_address.txt");
        if legacy_key.exists() {
            if vault.wallet.is_none() {
                if let Ok(key_hex) = fs::read_to_string(&legacy_key) {
                    if let Ok(key_bytes) = hex::decode(key_hex.trim()) {
                        if key_bytes.len() == 32 {
                            let mut priv_arr = [0u8; 32];
                            priv_arr.copy_from_slice(&key_bytes);
                            if let Ok(kp) = keypair_from_bytes(&priv_arr) {
                                let now = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                let base_mnemonic = derive_base_mnemonic_12(&kp.priv_key);
                                let (base_priv_key, base_address) = derive_base_wallet_from_mnemonic(&base_mnemonic)
                                    .unwrap_or_else(|_| {
                                        let bk = derive_base_key(&kp.priv_key);
                                        let ba = evm_address_from_priv_key(&bk).unwrap_or_default();
                                        (bk, ba)
                                    });
                                vault.wallet = Some(VaultWalletData {
                                    priv_key: kp.priv_key,
                                    address: kp.address,
                                    base_priv_key,
                                    base_address,
                                    base_mnemonic,
                                    created_at: now,
                                });
                                info!("[VAULT] Successfully migrated legacy plaintext wallet into encrypted compressed vault.dat");
                            }
                        }
                    }
                }
            }
            // Securely clean up plaintext key file from disk
            let _ = fs::remove_file(&legacy_key);
        }
        if legacy_addr.exists() {
            // Clean up plaintext address file from disk
            let _ = fs::remove_file(&legacy_addr);
        }

        // If wallet or records were loaded/migrated, persist in v3 format
        if vault.wallet.is_some() || !vault.records.is_empty() {
            let _ = vault.save();
        }

        Ok(vault)
    }

    fn load_or_create_key(key_path: &Path) -> Result<[u8; 32], String> {
        if key_path.exists() {
            let mut key = [0u8; 32];
            let mut f = File::open(key_path)
                .map_err(|e| format!("Failed to open vault key: {}", e))?;
            f.read_exact(&mut key)
                .map_err(|e| format!("Failed to read vault key: {}", e))?;
            return Ok(key);
        }

        let key: [u8; 32] = rand::random();
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(key_path)
            .map_err(|e| format!("Failed to create vault key: {}", e))?;

        #[cfg(unix)]
        {
            let mut perms = f.metadata().map_err(|e| e.to_string())?.permissions();
            perms.set_mode(0o600);
            let _ = f.set_permissions(perms);
        }

        f.write_all(&key)
            .map_err(|e| format!("Failed to write vault key: {}", e))?;

        info!("Created new secure vault master key at {:?}", key_path);
        Ok(key)
    }

    /// Loads, decrypts, and decompresses all data from vault.dat.
    fn load_from_disk(&mut self) -> Result<(), String> {
        let raw = fs::read(&self.file_path)
            .map_err(|e| format!("Failed to read vault file: {}", e))?;

        if raw.len() < 8 + SALT_SIZE + HMAC_SIZE {
            return Err("Corrupted vault file: header too short".to_string());
        }

        let magic = &raw[..8];
        let is_v4 = magic == MAGIC_HEADER_V4;
        let is_v3 = magic == MAGIC_HEADER_V3;
        let is_v2 = magic == MAGIC_HEADER_V2;
        let is_v1 = magic == MAGIC_HEADER_V1;

        if !is_v4 && !is_v3 && !is_v2 && !is_v1 {
            return Err("Invalid vault file magic header".to_string());
        }

        let salt: [u8; SALT_SIZE] = raw[8..24].try_into().unwrap();
        let expected_hmac: [u8; HMAC_SIZE] = raw[raw.len() - HMAC_SIZE..].try_into().unwrap();
        let ciphertext = &raw[24..raw.len() - HMAC_SIZE];

        let computed_hmac = Self::compute_hmac(&self.master_key, &salt, ciphertext);
        if computed_hmac != expected_hmac {
            return Err("Vault integrity failure: HMAC verification failed (tampered data)".to_string());
        }

        let decrypted = Self::cipher_crypt(&self.master_key, &salt, ciphertext);

        if is_v4 || is_v3 || is_v2 {
            let decompressed = Self::decompress_payload(&decrypted)?;
            let (wallet, records) = Self::deserialize_payload(&decompressed)?;
            self.wallet = wallet;
            self.records = records;
        } else {
            let records = Self::deserialize_records(&decrypted)?;
            self.records = records;
        }

        Ok(())
    }

    /// Loads legacy swaps.dat format.
    fn load_legacy_swaps(&self, path: &Path) -> Result<Vec<SwapRecord>, String> {
        let raw = fs::read(path).map_err(|e| e.to_string())?;
        if raw.len() < 8 + SALT_SIZE + HMAC_SIZE || &raw[..8] != MAGIC_HEADER_V1 {
            return Err("Invalid legacy header".to_string());
        }
        let salt: [u8; SALT_SIZE] = raw[8..24].try_into().unwrap();
        let expected_hmac: [u8; HMAC_SIZE] = raw[raw.len() - HMAC_SIZE..].try_into().unwrap();
        let ciphertext = &raw[24..raw.len() - HMAC_SIZE];
        if Self::compute_hmac(&self.master_key, &salt, ciphertext) != expected_hmac {
            return Err("HMAC failed".to_string());
        }
        let decrypted = Self::cipher_crypt(&self.master_key, &salt, ciphertext);
        Self::deserialize_records(&decrypted)
    }

    /// Serializes, compresses, encrypts, and writes to disk atomically with 0o600 permissions.
    pub fn save(&self) -> Result<(), String> {
        let plaintext = Self::serialize_payload(self.wallet.as_ref(), &self.records);
        let compressed = Self::compress_payload(&plaintext)?;

        let salt: [u8; SALT_SIZE] = rand::random();
        let ciphertext = Self::cipher_crypt(&self.master_key, &salt, &compressed);
        let hmac = Self::compute_hmac(&self.master_key, &salt, &ciphertext);

        let mut output = Vec::with_capacity(8 + SALT_SIZE + ciphertext.len() + HMAC_SIZE);
        output.extend_from_slice(MAGIC_HEADER_V4);
        output.extend_from_slice(&salt);
        output.extend_from_slice(&ciphertext);
        output.extend_from_slice(&hmac);

        let tmp_path = self.file_path.with_extension("tmp");
        {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp_path)
                .map_err(|e| format!("Failed to open tmp vault: {}", e))?;

            #[cfg(unix)]
            {
                let mut perms = f.metadata().map_err(|e| e.to_string())?.permissions();
                perms.set_mode(0o600);
                let _ = f.set_permissions(perms);
            }

            f.write_all(&output)
                .map_err(|e| format!("Failed to write vault data: {}", e))?;
            f.sync_all().map_err(|e| format!("Failed to sync vault: {}", e))?;
        }

        fs::rename(&tmp_path, &self.file_path)
            .map_err(|e| format!("Failed to atomically replace vault file: {}", e))?;

        Ok(())
    }

    /// Ensures a Dual-Vault (TRU + Base) wallet exists in the encrypted vault.
    /// If not present, generates new keypairs for TRU and Base (with a 12-word BIP-39 mnemonic) and saves encrypted to disk.
    pub fn get_or_create_wallet(&mut self) -> Result<KeyPair, String> {
        let needs_base_mnem = if let Some(w) = &self.wallet {
            w.base_mnemonic.is_empty()
        } else {
            false
        };
        if needs_base_mnem {
            if let Some(w) = &mut self.wallet {
                w.base_mnemonic = derive_base_mnemonic_12(&w.priv_key);
                if let Ok((bpk, baddr)) = derive_base_wallet_from_mnemonic(&w.base_mnemonic) {
                    w.base_priv_key = bpk;
                    w.base_address = baddr;
                }
            }
            let _ = self.save();
        }

        if let Some(w) = &self.wallet {
            return keypair_from_bytes(&w.priv_key);
        }

        let kp = generate_keypair();
        let base_mnemonic = derive_base_mnemonic_12(&kp.priv_key);
        let (base_priv_key, base_address) = derive_base_wallet_from_mnemonic(&base_mnemonic)
            .map_err(|e| format!("Failed to derive base address: {}", e))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.wallet = Some(VaultWalletData {
            priv_key: kp.priv_key,
            address: kp.address.clone(),
            base_priv_key,
            base_address: base_address.clone(),
            base_mnemonic,
            created_at: now,
        });

        self.save()?;
        info!("[VAULT] Generated new Dual-Vault with independent keys (TRU: {}, Base: {}) in encrypted vault.dat", kp.address, base_address);
        Ok(kp)
    }

    /// Generates a new independent Base EVM wallet inside the vault with a 12-word seed phrase.
    pub fn regenerate_base_wallet(&mut self) -> Result<String, String> {
        let base_mnemonic = generate_base_mnemonic_12();
        let (base_priv_key, base_address) = derive_base_wallet_from_mnemonic(&base_mnemonic)?;

        self.get_or_create_wallet()?;
        if let Some(w) = &mut self.wallet {
            w.base_priv_key = base_priv_key;
            w.base_address = base_address.clone();
            w.base_mnemonic = base_mnemonic;
            self.save()?;
            info!("[VAULT] Regenerated independent Base wallet: {}", base_address);
            Ok(base_address)
        } else {
            Err("Wallet not initialized".to_string())
        }
    }

    /// Allows setting or importing a specific 12-word Base BIP-39 mnemonic phrase into the vault.
    pub fn set_base_mnemonic(&mut self, phrase: &str) -> Result<String, String> {
        let (base_priv_key, base_address) = derive_base_wallet_from_mnemonic(phrase)?;
        self.get_or_create_wallet()?;
        if let Some(w) = &mut self.wallet {
            w.base_priv_key = base_priv_key;
            w.base_address = base_address.clone();
            w.base_mnemonic = phrase.trim().to_string();
            self.save()?;
            info!("[VAULT] Imported Base 12-word mnemonic wallet: {}", base_address);
            Ok(base_address)
        } else {
            Err("Wallet not initialized".to_string())
        }
    }

    /// Allows setting or importing a specific Base EVM private key into the vault.
    pub fn set_base_priv_key(&mut self, priv_key_hex: &str) -> Result<String, String> {
        let clean = priv_key_hex.trim().strip_prefix("0x").unwrap_or(priv_key_hex.trim());
        let bytes = hex::decode(clean).map_err(|e| format!("Invalid hex: {}", e))?;
        if bytes.len() != 32 {
            return Err("Base private key must be exactly 32 bytes (64 hex characters)".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        let addr = evm_address_from_priv_key(&arr)?;

        self.get_or_create_wallet()?;
        if let Some(w) = &mut self.wallet {
            w.base_priv_key = arr;
            w.base_address = addr.clone();
            w.base_mnemonic = String::new();
            self.save()?;
            info!("[VAULT] Imported Base wallet: {}", addr);
            Ok(addr)
        } else {
            Err("Wallet not initialized".to_string())
        }
    }

    /// Exports public address, private key (Hex & WIF), 24-word phrase for TRU, and 12-word phrase for Base.
    /// Never writes to disk; strictly returns the data in memory.
    pub fn export_wallet(&mut self) -> Result<WalletExport, String> {
        let kp = self.get_or_create_wallet()?;
        let priv_key_wif = priv_key_to_wif(&kp.priv_key);
        let mnemonic = priv_key_to_mnemonic(&kp.priv_key)?;

        let (base_address, base_priv_key_hex, base_mnemonic) = if let Some(w) = &self.wallet {
            let b_mnem = if w.base_mnemonic.is_empty() {
                derive_base_mnemonic_12(&w.priv_key)
            } else {
                w.base_mnemonic.clone()
            };
            (w.base_address.clone(), format!("0x{}", hex::encode(w.base_priv_key)), b_mnem)
        } else {
            (String::new(), String::new(), String::new())
        };

        Ok(WalletExport {
            address: kp.address,
            pub_key_hex: hex::encode(kp.pub_key),
            priv_key_hex: hex::encode(kp.priv_key),
            priv_key_wif,
            mnemonic,
            base_address,
            base_priv_key_hex,
            base_mnemonic,
        })
    }

    /// Returns the raw 32-byte TRU private key of the vault.
    pub fn get_priv_key(&mut self) -> Result<[u8; 32], String> {
        let kp = self.get_or_create_wallet()?;
        Ok(kp.priv_key)
    }

    /// Returns the public TRU address of the vault.
    pub fn get_address(&mut self) -> Result<String, String> {
        let kp = self.get_or_create_wallet()?;
        Ok(kp.address)
    }

    /// Returns the raw 32-byte Base EVM private key of the vault.
    pub fn get_base_priv_key(&mut self) -> Result<[u8; 32], String> {
        self.get_or_create_wallet()?;
        if let Some(w) = &self.wallet {
            Ok(w.base_priv_key)
        } else {
            Err("No wallet in vault".to_string())
        }
    }

    /// Returns the public checksummed Base EVM address of the vault.
    pub fn get_base_address(&mut self) -> Result<String, String> {
        self.get_or_create_wallet()?;
        if let Some(w) = &self.wallet {
            Ok(w.base_address.clone())
        } else {
            Err("No wallet in vault".to_string())
        }
    }

    /// Loads all swap records.
    pub fn load_records(&self) -> Result<Vec<SwapRecord>, String> {
        Ok(self.records.clone())
    }

    /// Saves all swap records to disk encrypted and compressed.
    pub fn save_records(&mut self, records: &[SwapRecord]) -> Result<(), String> {
        self.records = records.to_vec();
        self.save()
    }

    fn compress_payload(data: &[u8]) -> Result<Vec<u8>, String> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).map_err(|e| format!("Compression error: {}", e))?;
        encoder.finish().map_err(|e| format!("Compression finish error: {}", e))
    }

    fn decompress_payload(data: &[u8]) -> Result<Vec<u8>, String> {
        let mut decoder = DeflateDecoder::new(data);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).map_err(|e| format!("Decompression error: {}", e))?;
        Ok(decompressed)
    }

    fn cipher_crypt(key: &[u8; 32], salt: &[u8; 16], input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut counter: u32 = 0;

        for chunk in input.chunks(32) {
            let mut hasher = Sha256::new();
            hasher.update(key);
            hasher.update(salt);
            hasher.update(&counter.to_le_bytes());
            let keystream = hasher.finalize();

            for (b, k) in chunk.iter().zip(keystream.iter()) {
                output.push(b ^ k);
            }
            counter += 1;
        }

        output
    }

    fn compute_hmac(key: &[u8; 32], salt: &[u8; 16], data: &[u8]) -> [u8; 32] {
        let mut inner_hasher = Sha256::new();
        let mut ipad = [0x36u8; 64];
        for (i, b) in key.iter().enumerate() {
            ipad[i] ^= b;
        }
        inner_hasher.update(&ipad);
        inner_hasher.update(salt);
        inner_hasher.update(data);
        let inner_hash = inner_hasher.finalize();

        let mut outer_hasher = Sha256::new();
        let mut opad = [0x5cu8; 64];
        for (i, b) in key.iter().enumerate() {
            opad[i] ^= b;
        }
        outer_hasher.update(&opad);
        outer_hasher.update(&inner_hash);

        let out = outer_hasher.finalize();
        let mut res = [0u8; 32];
        res.copy_from_slice(&out);
        res
    }

    fn serialize_payload(wallet: Option<&VaultWalletData>, records: &[SwapRecord]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x04); // Payload version 4 (Dual-Vault with Base 12-word mnemonic)

        if let Some(w) = wallet {
            buf.push(1);
            // TRU Keypair info
            buf.extend_from_slice(&w.priv_key);
            buf.extend_from_slice(&w.created_at.to_le_bytes());
            let addr_bytes = w.address.as_bytes();
            buf.push(addr_bytes.len().min(255) as u8);
            buf.extend_from_slice(&addr_bytes[..addr_bytes.len().min(255)]);

            // Base EVM Key info
            buf.extend_from_slice(&w.base_priv_key);
            let base_addr_bytes = w.base_address.as_bytes();
            buf.push(base_addr_bytes.len().min(255) as u8);
            buf.extend_from_slice(&base_addr_bytes[..base_addr_bytes.len().min(255)]);

            // Base 12-word Mnemonic
            let mnem_bytes = w.base_mnemonic.as_bytes();
            buf.push(mnem_bytes.len().min(255) as u8);
            buf.extend_from_slice(&mnem_bytes[..mnem_bytes.len().min(255)]);
        } else {
            buf.push(0);
        }

        buf.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for r in records {
            buf.extend_from_slice(&r.swap_id);
            buf.extend_from_slice(&r.secret);
            buf.extend_from_slice(&r.hash160);
            buf.push(r.direction);
            buf.push(r.step);
            buf.push(r.status);
            buf.extend_from_slice(&r.tru_atoms.to_le_bytes());
            buf.extend_from_slice(&r.usdc_atomic.to_le_bytes());
            buf.extend_from_slice(&r.timelock.to_le_bytes());
            buf.extend_from_slice(&r.created_at.to_le_bytes());

            let b_bytes = r.beneficiary.as_bytes();
            buf.push(b_bytes.len().min(255) as u8);
            buf.extend_from_slice(&b_bytes[..b_bytes.len().min(255)]);
        }

        buf
    }

    fn deserialize_payload(buf: &[u8]) -> Result<(Option<VaultWalletData>, Vec<SwapRecord>), String> {
        if buf.is_empty() {
            return Ok((None, Vec::new()));
        }
        let mut offset = 0;
        let version = buf[offset];
        offset += 1;

        let wallet = if version >= 3 {
            if offset >= buf.len() {
                return Err("Truncated wallet flag".to_string());
            }
            let has_wallet = buf[offset];
            offset += 1;
            if has_wallet == 1 {
                if offset + 41 > buf.len() {
                    return Err("Truncated TRU wallet data".to_string());
                }
                let mut priv_key = [0u8; 32];
                priv_key.copy_from_slice(&buf[offset..offset + 32]);
                offset += 32;

                let created_at = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
                offset += 8;

                let addr_len = buf[offset] as usize;
                offset += 1;

                if offset + addr_len > buf.len() {
                    return Err("Truncated TRU wallet address".to_string());
                }
                let address = String::from_utf8_lossy(&buf[offset..offset + addr_len]).to_string();
                offset += addr_len;

                // Base EVM Key & Address
                if offset + 33 > buf.len() {
                    return Err("Truncated Base wallet data".to_string());
                }
                let mut base_priv_key = [0u8; 32];
                base_priv_key.copy_from_slice(&buf[offset..offset + 32]);
                offset += 32;

                let base_addr_len = buf[offset] as usize;
                offset += 1;
                if offset + base_addr_len > buf.len() {
                    return Err("Truncated Base wallet address".to_string());
                }
                let base_address = String::from_utf8_lossy(&buf[offset..offset + base_addr_len]).to_string();
                offset += base_addr_len;

                // Base 12-word Mnemonic
                let base_mnemonic = if version >= 4 && offset < buf.len() {
                    let mnem_len = buf[offset] as usize;
                    offset += 1;
                    if offset + mnem_len <= buf.len() {
                        let m = String::from_utf8_lossy(&buf[offset..offset + mnem_len]).to_string();
                        offset += mnem_len;
                        m
                    } else {
                        derive_base_mnemonic_12(&priv_key)
                    }
                } else {
                    derive_base_mnemonic_12(&priv_key)
                };

                Some(VaultWalletData {
                    priv_key,
                    address,
                    base_priv_key,
                    base_address,
                    base_mnemonic,
                    created_at,
                })
            } else {
                None
            }
        } else if version == 2 {
            // Backward-compatibility: V2 format had only TRU key. Derive Base key automatically!
            if offset >= buf.len() {
                return Err("Truncated wallet flag".to_string());
            }
            let has_wallet = buf[offset];
            offset += 1;
            if has_wallet == 1 {
                if offset + 41 > buf.len() {
                    return Err("Truncated wallet data".to_string());
                }
                let mut priv_key = [0u8; 32];
                priv_key.copy_from_slice(&buf[offset..offset + 32]);
                offset += 32;

                let created_at = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
                offset += 8;

                let addr_len = buf[offset] as usize;
                offset += 1;

                if offset + addr_len > buf.len() {
                    return Err("Truncated wallet address".to_string());
                }
                let address = String::from_utf8_lossy(&buf[offset..offset + addr_len]).to_string();
                offset += addr_len;

                let base_mnemonic = derive_base_mnemonic_12(&priv_key);
                let (base_priv_key, base_address) = derive_base_wallet_from_mnemonic(&base_mnemonic)
                    .unwrap_or_else(|_| {
                        let bk = derive_base_key(&priv_key);
                        let ba = evm_address_from_priv_key(&bk).unwrap_or_default();
                        (bk, ba)
                    });

                Some(VaultWalletData {
                    priv_key,
                    address,
                    base_priv_key,
                    base_address,
                    base_mnemonic,
                    created_at,
                })
            } else {
                None
            }
        } else {
            None
        };

        if offset + 4 > buf.len() {
            return Ok((wallet, Vec::new()));
        }

        let count = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let mut records = Vec::with_capacity(count);

        for _ in 0..count {
            if offset + 115 > buf.len() {
                return Err("Malformed binary record buffer".to_string());
            }

            let mut swap_id = [0u8; 32];
            swap_id.copy_from_slice(&buf[offset..offset + 32]);
            offset += 32;

            let mut secret = [0u8; 32];
            secret.copy_from_slice(&buf[offset..offset + 32]);
            offset += 32;

            let mut hash160 = [0u8; 20];
            hash160.copy_from_slice(&buf[offset..offset + 20]);
            offset += 20;

            let direction = buf[offset];
            offset += 1;
            let step = buf[offset];
            offset += 1;
            let status = buf[offset];
            offset += 1;

            let tru_atoms = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
            offset += 8;

            let usdc_atomic = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
            offset += 8;

            let timelock = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
            offset += 4;

            let created_at = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
            offset += 8;

            if offset >= buf.len() {
                return Err("Malformed beneficiary length".to_string());
            }
            let b_len = buf[offset] as usize;
            offset += 1;

            if offset + b_len > buf.len() {
                return Err("Malformed beneficiary string".to_string());
            }
            let beneficiary = String::from_utf8_lossy(&buf[offset..offset + b_len]).to_string();
            offset += b_len;

            records.push(SwapRecord {
                swap_id,
                secret,
                hash160,
                direction,
                step,
                status,
                tru_atoms,
                usdc_atomic,
                timelock,
                created_at,
                beneficiary,
            });
        }

        Ok((wallet, records))
    }

    fn deserialize_records(buf: &[u8]) -> Result<Vec<SwapRecord>, String> {
        let (_, records) = Self::deserialize_payload(buf)?;
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_compression_encryption_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!("truswap_test_{}", rand::random::<u32>()));
        let mut vault = EncryptedVault::open(&temp_dir).expect("Failed to open vault");

        // 1. Generate and test Dual-Vault in vault
        let wallet = vault.get_or_create_wallet().expect("Failed to create wallet");
        assert!(wallet.address.starts_with('T'));
        let base_addr = vault.get_base_address().expect("Failed to get base address");
        assert!(base_addr.starts_with("0x") && base_addr.len() == 42);

        let export = vault.export_wallet().expect("Failed to export wallet");
        assert_eq!(export.address, wallet.address);
        assert_eq!(export.base_address, base_addr);
        assert_eq!(export.mnemonic.split_whitespace().count(), 24);
        assert_eq!(export.base_mnemonic.split_whitespace().count(), 12);
        assert!(export.priv_key_wif.starts_with('K') || export.priv_key_wif.starts_with('L'));
        assert!(export.base_priv_key_hex.starts_with("0x"));

        // 2. Add swap records
        let records = vec![SwapRecord {
            swap_id: [0x11u8; 32],
            secret: [0x22u8; 32],
            hash160: [0x33u8; 20],
            direction: 1,
            step: 2,
            status: 0,
            tru_atoms: 10_000_000_000,
            usdc_atomic: 100_000_000,
            timelock: 1790900000,
            created_at: 1790800000,
            beneficiary: "0x1111111111111111111111111111111111111111".to_string(),
        }];

        vault.save_records(&records).expect("Failed to save records");

        // Verify vault.dat exists and does NOT contain plain text keys or addresses
        let raw = fs::read(&vault.file_path).unwrap();
        assert_eq!(&raw[..8], MAGIC_HEADER_V4);
        assert!(!String::from_utf8_lossy(&raw).contains(&wallet.address));
        assert!(!String::from_utf8_lossy(&raw).contains(&base_addr));
        assert!(!String::from_utf8_lossy(&raw).contains("0x1111111111111111111111111111111111111111"));

        // Reopen vault and verify all data persists
        let mut reopened = EncryptedVault::open(&temp_dir).expect("Failed to reopen vault");
        let reopened_wallet = reopened.get_or_create_wallet().expect("Failed to get wallet");
        assert_eq!(reopened_wallet.address, wallet.address);
        assert_eq!(reopened_wallet.priv_key, wallet.priv_key);
        assert_eq!(reopened.get_base_address().unwrap(), base_addr);

        let loaded_records = reopened.load_records().expect("Failed to load records");
        assert_eq!(loaded_records, records);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
