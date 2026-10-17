use rand::RngCore;
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

/// Generates a cryptographically secure 32-byte secret preimage.
pub fn generate_secret() -> [u8; 32] {
    let mut secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret);
    secret
}

/// Computes SHA256 of raw bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Computes RIPEMD160 of raw bytes.
pub fn ripemd160(data: &[u8]) -> [u8; 20] {
    let mut hasher = Ripemd160::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Computes canonical HASH160: RIPEMD160(SHA256(data)).
/// Matches TRU OP_HASH160 and BaseHTLC.sol ripemd160(abi.encodePacked(sha256(secret))).
pub fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = sha256(data);
    ripemd160(&sha)
}

const BASE58_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Encodes raw bytes to Bitcoin-style Base58 string
pub fn base58_encode(data: &[u8]) -> String {
    let mut zeros = 0;
    while zeros < data.len() && data[zeros] == 0 {
        zeros += 1;
    }

    let size = (data.len() - zeros) * 138 / 100 + 1;
    let mut digits = vec![0u8; size];

    for &byte in &data[zeros..] {
        let mut carry = byte as u32;
        for digit in digits.iter_mut().rev() {
            carry += (*digit as u32) << 8;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
    }

    let mut i = 0;
    while i < digits.len() && digits[i] == 0 {
        i += 1;
    }

    let mut res = String::with_capacity(zeros + (digits.len() - i));
    for _ in 0..zeros {
        res.push('1');
    }
    for &digit in &digits[i..] {
        res.push(BASE58_ALPHABET[digit as usize] as char);
    }
    res
}

/// Decodes a Bitcoin-style Base58 string to raw bytes
pub fn base58_decode(s: &str) -> Option<Vec<u8>> {
    let mut zeros = 0;
    while zeros < s.len() && s.as_bytes()[zeros] == b'1' {
        zeros += 1;
    }

    let mut b58 = Vec::with_capacity(s.len());
    for c in s.bytes() {
        let val = match c {
            b'1'..=b'9' => c - b'1',
            b'A'..=b'H' => c - b'A' + 9,
            b'J'..=b'N' => c - b'J' + 17,
            b'P'..=b'Z' => c - b'P' + 22,
            b'a'..=b'k' => c - b'a' + 33,
            b'm'..=b'z' => c - b'm' + 44,
            _ => return None,
        };
        b58.push(val);
    }

    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    for &digit in &b58[zeros..] {
        let mut carry = digit as u32;
        let mut j = 0;
        while j < out.len() || carry != 0 {
            if j >= out.len() {
                out.push(0);
            }
            carry += (out[j] as u32) * 58;
            out[j] = (carry & 0xff) as u8;
            carry >>= 8;
            j += 1;
        }
    }

    let mut result = vec![0u8; zeros];
    for &byte in out.iter().rev() {
        result.push(byte);
    }
    Some(result)
}

/// Converts a 20-byte PubKeyHash to a canonical TRU mainnet address (Version 0x41, starts with 'T')
pub fn pkh_to_tru_address(pkh: &[u8; 20]) -> String {
    let mut payload = Vec::with_capacity(25);
    payload.push(0x41); // MAINNET_P2PKH_VERSION
    payload.extend_from_slice(pkh);
    let checksum = sha256(&sha256(&payload));
    payload.extend_from_slice(&checksum[..4]);
    base58_encode(&payload)
}

/// Decodes a TRU mainnet address (Version 0x41) into its 20-byte PubKeyHash
pub fn tru_address_to_pkh(addr: &str) -> Option<[u8; 20]> {
    let raw = base58_decode(addr)?;
    if raw.len() != 25 || raw[0] != 0x41 {
        return None;
    }
    let checksum = sha256(&sha256(&raw[..21]));
    if checksum[..4] != raw[21..25] {
        return None;
    }
    let mut pkh = [0u8; 20];
    pkh.copy_from_slice(&raw[1..21]);
    Some(pkh)
}

/// Canonical SECP256k1 KeyPair for TRU Vaults
#[derive(Clone, Debug)]
pub struct KeyPair {
    pub priv_key: [u8; 32],
    pub pub_key: [u8; 33],
    pub address: String,
}

pub fn generate_keypair() -> KeyPair {
    loop {
        let secret = generate_secret();
        if let Ok(kp) = keypair_from_bytes(&secret) {
            return kp;
        }
    }
}

pub fn keypair_from_bytes(priv_bytes: &[u8; 32]) -> Result<KeyPair, String> {
    let signing_key = k256::ecdsa::SigningKey::from_bytes(priv_bytes.into())
        .map_err(|e| format!("Invalid private key: {}", e))?;
    Ok(keypair_from_signing_key(&signing_key))
}

pub fn keypair_from_signing_key(signing_key: &k256::ecdsa::SigningKey) -> KeyPair {
    let verifying_key = signing_key.verifying_key();
    let pub_point = verifying_key.to_sec1_point(true);
    let mut pub_key = [0u8; 33];
    pub_key.copy_from_slice(pub_point.as_bytes());
    let pkh = hash160(&pub_key);
    let address = pkh_to_tru_address(&pkh);
    KeyPair {
        priv_key: signing_key.to_bytes().into(),
        pub_key,
        address,
    }
}

/// Converts a 32-byte SECP256k1 private key to canonical WIF (Wallet Import Format).
/// Uses prefix 0x80 (Mainnet private key) and compressed pubkey flag 0x01.
pub fn priv_key_to_wif(priv_key: &[u8; 32]) -> String {
    let mut payload = Vec::with_capacity(38);
    payload.push(0x80); // MAINNET_PRIVATE_KEY_VERSION
    payload.extend_from_slice(priv_key);
    payload.push(0x01); // Compressed public key flag
    let checksum = sha256(&sha256(&payload));
    payload.extend_from_slice(&checksum[..4]);
    base58_encode(&payload)
}

/// Decodes a WIF private key string into raw 32-byte private key.
pub fn wif_to_priv_key(wif: &str) -> Option<[u8; 32]> {
    let raw = base58_decode(wif)?;
    if raw.len() == 38 && raw[0] == 0x80 && raw[33] == 0x01 {
        // Compressed (34 bytes payload + 4 checksum)
        let checksum = sha256(&sha256(&raw[..34]));
        if checksum[..4] != raw[34..38] {
            return None;
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&raw[1..33]);
        Some(key)
    } else if raw.len() == 37 && raw[0] == 0x80 {
        // Uncompressed (33 bytes payload + 4 checksum)
        let checksum = sha256(&sha256(&raw[..33]));
        if checksum[..4] != raw[33..37] {
            return None;
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&raw[1..33]);
        Some(key)
    } else {
        None
    }
}

/// Converts a 32-byte private key (256-bit entropy) into a standard 24-word BIP-39 mnemonic phrase.
pub fn priv_key_to_mnemonic(priv_key: &[u8; 32]) -> Result<String, String> {
    let mnemonic = bip39::Mnemonic::from_entropy(priv_key)
        .map_err(|e| format!("Failed to generate BIP-39 mnemonic: {}", e))?;
    Ok(mnemonic.to_string())
}

/// Restores a 32-byte private key from a 24-word BIP-39 mnemonic phrase.
pub fn mnemonic_to_priv_key(phrase: &str) -> Result<[u8; 32], String> {
    let mnemonic = bip39::Mnemonic::parse_normalized(phrase)
        .map_err(|e| format!("Invalid BIP-39 mnemonic: {}", e))?;
    let entropy = mnemonic.to_entropy();
    if entropy.len() != 32 {
        return Err("Mnemonic entropy must be exactly 32 bytes (24 words)".to_string());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&entropy);
    Ok(key)
}

/// Converts a 20-byte address to an EIP-55 mixed-case checksummed Ethereum/Base address string.
pub fn to_checksum_address(addr_bytes: &[u8]) -> String {
    use sha3::{Digest, Keccak256};
    let hex_addr = hex::encode(addr_bytes).to_lowercase();
    let hash = Keccak256::digest(hex_addr.as_bytes());
    let hash_hex = hex::encode(hash);

    let mut checksummed = String::with_capacity(42);
    checksummed.push_str("0x");
    for (i, c) in hex_addr.chars().enumerate() {
        let hash_nibble = u8::from_str_radix(&hash_hex[i..i + 1], 16).unwrap_or(0);
        if hash_nibble >= 8 {
            checksummed.push(c.to_ascii_uppercase());
        } else {
            checksummed.push(c.to_ascii_lowercase());
        }
    }
    checksummed
}

/// Derives a standard EIP-55 checksummed EVM address (0x...) from a 32-byte SECP256k1 private key.
pub fn evm_address_from_priv_key(priv_bytes: &[u8; 32]) -> Result<String, String> {
    use sha3::{Digest, Keccak256};
    let signing_key = k256::ecdsa::SigningKey::from_bytes(priv_bytes.into())
        .map_err(|e| format!("Invalid private key for EVM: {}", e))?;
    let verifying_key = signing_key.verifying_key();
    let pub_point = verifying_key.to_sec1_point(false);
    let pub_bytes = pub_point.as_bytes();
    if pub_bytes.len() != 65 || pub_bytes[0] != 0x04 {
        return Err("Invalid uncompressed SECP256k1 point".to_string());
    }
    // Keccak256 of the 64-byte uncompressed public key (omitting 0x04 prefix)
    let hash = Keccak256::digest(&pub_bytes[1..]);
    let addr_bytes = &hash[12..32];
    Ok(to_checksum_address(addr_bytes))
}

/// Generates a fresh random 12-word BIP-39 mnemonic phrase (128-bit entropy).
pub fn generate_base_mnemonic_12() -> String {
    let mut entropy = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut entropy);
    let mnemonic = bip39::Mnemonic::from_entropy(&entropy).expect("16-byte entropy is valid for 12 words");
    mnemonic.to_string()
}

/// Deterministically derives a 12-word BIP-39 mnemonic phrase from a primary TRU private key.
pub fn derive_base_mnemonic_12(tru_priv_key: &[u8; 32]) -> String {
    let mut payload = Vec::with_capacity(32 + 28);
    payload.extend_from_slice(tru_priv_key);
    payload.extend_from_slice(b"TRUSWAP_BASE_MNEMONIC_12_V1");
    let hash = sha256(&payload);
    let entropy: [u8; 16] = hash[..16].try_into().unwrap();
    let mnemonic = bip39::Mnemonic::from_entropy(&entropy).expect("16-byte entropy is valid for 12 words");
    mnemonic.to_string()
}

/// Derives standard EVM / Base private key and EIP-55 address from a 12-word BIP-39 mnemonic using BIP-44 path m/44'/60'/0'/0/0.
/// 100% compatible with Rabby, MetaMask, TrustWallet, and Ledger.
pub fn derive_base_wallet_from_mnemonic(phrase: &str) -> Result<([u8; 32], String), String> {
    use std::str::FromStr;
    let mnemonic = bip39::Mnemonic::parse_normalized(phrase)
        .map_err(|e| format!("Invalid BIP-39 mnemonic: {}", e))?;
    let seed = mnemonic.to_seed("");
    let path = bip32::DerivationPath::from_str("m/44'/60'/0'/0/0")
        .map_err(|e| format!("Invalid derivation path: {}", e))?;
    let child = bip32::XPrv::derive_from_path(&seed, &path)
        .map_err(|e| format!("BIP-32 derivation failed: {}", e))?;
    let priv_key: [u8; 32] = child.private_key().to_bytes().into();
    let addr = evm_address_from_priv_key(&priv_key)?;
    Ok((priv_key, addr))
}

/// Deterministically derives a secondary Base EVM key from the primary TRU vault private key.
/// Ensures single 24-word seed phrase securely backs up BOTH TRU and Base wallets!
pub fn derive_base_key(tru_priv_key: &[u8; 32]) -> [u8; 32] {
    let mut payload = Vec::with_capacity(32 + 24);
    payload.extend_from_slice(tru_priv_key);
    payload.extend_from_slice(b"TRUSWAP_BASE_BURNER_V1");
    sha256(&payload)
}

pub use derive_base_key as derive_base_burner_key;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wif_roundtrip() {
        let secret = [0x55u8; 32];
        let wif = priv_key_to_wif(&secret);
        assert!(wif.starts_with('K') || wif.starts_with('L'));
        let recovered = wif_to_priv_key(&wif).expect("Failed to decode WIF");
        assert_eq!(secret, recovered);
    }

    #[test]
    fn test_bip39_mnemonic_roundtrip() {
        let secret = [0x42u8; 32];
        let phrase = priv_key_to_mnemonic(&secret).expect("Failed to derive mnemonic");
        let words: Vec<&str> = phrase.split_whitespace().collect();
        assert_eq!(words.len(), 24);
        let recovered = mnemonic_to_priv_key(&phrase).expect("Failed to recover key from mnemonic");
        assert_eq!(secret, recovered);
    }

    #[test]
    fn test_evm_address_derivation() {
        // Standard test vector: privkey = 1 -> 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf (checksummed)
        let mut priv_bytes = [0u8; 32];
        priv_bytes[31] = 1;
        let addr = evm_address_from_priv_key(&priv_bytes).expect("Failed to derive EVM address");
        assert_eq!(addr, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");

        let base_key = derive_base_key(&priv_bytes);
        assert_ne!(base_key, priv_bytes);
        let base_addr = evm_address_from_priv_key(&base_key).expect("Base address valid");
        assert!(base_addr.starts_with("0x") && base_addr.len() == 42);
    }

    #[test]
    fn test_bip32_derivation() {
        use bip32::{XPrv, DerivationPath};
        use std::str::FromStr;
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = bip39::Mnemonic::parse_normalized(phrase).unwrap();
        let seed = mnemonic.to_seed("");
        let path = DerivationPath::from_str("m/44'/60'/0'/0/0").unwrap();
        let child = XPrv::derive_from_path(&seed, &path).unwrap();
        let priv_key: [u8; 32] = child.private_key().to_bytes().into();
        let addr = evm_address_from_priv_key(&priv_key).unwrap();
        // Known Ethereum BIP-44 address for 'abandon ... about' at m/44'/60'/0'/0/0:
        // 0x9858EfFD232B4033E47d90003D41EC34EcaEda94
        assert_eq!(addr, "0x9858EfFD232B4033E47d90003D41EC34EcaEda94");
    }
}

