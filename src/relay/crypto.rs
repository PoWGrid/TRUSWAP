use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
use secp256k1::{Message, Secp256k1};
use sha3::{Digest, Keccak256};

/// Computes Keccak-256 hash of byte slice
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Derives 20-byte Ethereum address string (with 0x prefix) from uncompressed public key
pub fn pubkey_to_eth_address(pubkey: &secp256k1::PublicKey) -> String {
    let uncompressed = pubkey.serialize_uncompressed(); // 65 bytes, starts with 0x04
    let hash = keccak256(&uncompressed[1..65]);
    format!("0x{}", hex::encode(&hash[12..32]))
}

/// Recovers the signer's Ethereum address from a standard personal_sign message and 65-byte signature
pub fn recover_personal_sign_address(
    message_text: &str,
    signature_hex: &str,
) -> Result<String, String> {
    let clean_sig = signature_hex.strip_prefix("0x").unwrap_or(signature_hex);
    let sig_bytes = hex::decode(clean_sig).map_err(|e| format!("Invalid hex signature: {}", e))?;
    if sig_bytes.len() != 65 {
        return Err(format!(
            "Signature must be exactly 65 bytes, received {}",
            sig_bytes.len()
        ));
    }

    // Prepare standard Ethereum personal_sign envelope: "\x19Ethereum Signed Message:\n<len><msg>"
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message_text.len());
    let mut eth_signed_data = Vec::with_capacity(prefix.len() + message_text.len());
    eth_signed_data.extend_from_slice(prefix.as_bytes());
    eth_signed_data.extend_from_slice(message_text.as_bytes());

    let prehash = keccak256(&eth_signed_data);
    let secp_msg = Message::from_digest(prehash);

    let mut v = sig_bytes[64];
    if v >= 27 {
        v -= 27;
    }
    if v > 3 {
        return Err(format!("Invalid signature recovery id v: {}", v));
    }

    let recovery_id = RecoveryId::from_i32(v as i32)
        .map_err(|e| format!("Invalid recovery ID: {}", e))?;

    let rec_sig = RecoverableSignature::from_compact(&sig_bytes[..64], recovery_id)
        .map_err(|e| format!("Failed to parse recoverable signature: {}", e))?;

    let secp = Secp256k1::new();
    let pubkey = secp
        .recover_ecdsa(&secp_msg, &rec_sig)
        .map_err(|e| format!("ECDSA recovery failed: {}", e))?;

    Ok(pubkey_to_eth_address(&pubkey))
}

/// Formats the canonical message for order intent signing
pub fn canonical_order_message(
    order_id: &str,
    side: &str,
    amount_tru: f64,
    amount_usdc: f64,
    hash160_hex: &str,
    timelock: u32,
    beneficiary_address: &str,
) -> String {
    format!(
        "TRUSWAP_INTENT:{}:{}:{:.8}:{:.6}:{}:{}:{}",
        order_id,
        side,
        amount_tru,
        amount_usdc,
        hash160_hex.to_lowercase(),
        timelock,
        beneficiary_address.to_lowercase()
    )
}

/// Formats the canonical message for order cancellation
pub fn canonical_cancel_message(order_id: &str, maker_identifier: &str) -> String {
    let clean_id = order_id.strip_prefix("0x").unwrap_or(order_id).to_lowercase();
    let ident = if maker_identifier.starts_with("0x") {
        maker_identifier.to_lowercase()
    } else {
        maker_identifier.trim().to_string()
    };
    format!(
        "TRUSWAP_CANCEL:{}:{}",
        clean_id,
        ident
    )
}

/// Normalizes an address or a 33-byte compressed pubkey hex to a canonical TRU address (Base58Check, starts with 'T')
pub fn normalize_to_tru_address(addr_or_pubkey: &str) -> String {
    let trimmed = addr_or_pubkey.trim();
    if trimmed.len() == 66 && (trimmed.starts_with("02") || trimmed.starts_with("03")) {
        if let Ok(bytes) = hex::decode(trimmed) {
            if bytes.len() == 33 {
                let mut arr = [0u8; 33];
                arr.copy_from_slice(&bytes);
                let pkh = crate::core::keys::hash160(&arr);
                return crate::core::keys::pkh_to_tru_address(&pkh);
            }
        }
    }
    trimmed.to_string()
}

/// Formats the canonical message for order take intent signing.
/// Always normalizes the TRU identifier to a canonical Base58Check TRU address (`T...`).
pub fn canonical_take_message(
    order_id: &str,
    taker_base_address: &str,
    taker_tru_address: &str,
) -> String {
    let canonical_tru = normalize_to_tru_address(taker_tru_address);
    format!(
        "TRUSWAP_TAKE:{}:{}:{}",
        order_id.strip_prefix("0x").unwrap_or(order_id).to_lowercase(),
        taker_base_address.to_lowercase(),
        canonical_tru
    )
}

/// Signs a text message according to Ethereum personal_sign (EIP-191) with a 32-byte private key
pub fn sign_personal_message(msg_text: &str, priv_key_bytes: &[u8; 32]) -> Result<String, String> {
    let secp = Secp256k1::new();
    let secret_key = secp256k1::SecretKey::from_slice(priv_key_bytes)
        .map_err(|e| format!("Invalid private key: {}", e))?;

    let prefix = format!("\x19Ethereum Signed Message:\n{}", msg_text.len());
    let mut eth_signed_data = Vec::with_capacity(prefix.len() + msg_text.len());
    eth_signed_data.extend_from_slice(prefix.as_bytes());
    eth_signed_data.extend_from_slice(msg_text.as_bytes());

    let prehash = keccak256(&eth_signed_data);
    let secp_msg = Message::from_digest(prehash);
    let rec_sig = secp.sign_ecdsa_recoverable(&secp_msg, &secret_key);

    let (recovery_id, compact) = rec_sig.serialize_compact();
    let mut sig_bytes = Vec::with_capacity(65);
    sig_bytes.extend_from_slice(&compact);
    sig_bytes.push(recovery_id.to_i32() as u8 + 27);

    Ok(hex::encode(sig_bytes))
}

/// Signs a text message according to the TRU message standard (BIP-137 / Bitcoin-style message envelope)
/// using a 32-byte SECP256k1 private key, returning a 65-byte compact recoverable signature in hex.
pub fn sign_tru_message(msg_text: &str, priv_key_bytes: &[u8; 32]) -> Result<String, String> {
    let secp = Secp256k1::new();
    let secret_key = secp256k1::SecretKey::from_slice(priv_key_bytes)
        .map_err(|e| format!("Invalid TRU private key: {}", e))?;

    let prefix = format!("\x18TRU Signed Message:\n{}", msg_text.len());
    let mut data = Vec::with_capacity(prefix.len() + msg_text.len());
    data.extend_from_slice(prefix.as_bytes());
    data.extend_from_slice(msg_text.as_bytes());

    let prehash = crate::core::keys::sha256(&crate::core::keys::sha256(&data));
    let secp_msg = Message::from_digest(prehash);
    let rec_sig = secp.sign_ecdsa_recoverable(&secp_msg, &secret_key);

    let (recovery_id, compact) = rec_sig.serialize_compact();
    let mut sig_bytes = Vec::with_capacity(65);
    sig_bytes.extend_from_slice(&compact);
    sig_bytes.push(recovery_id.to_i32() as u8 + 27);

    Ok(hex::encode(sig_bytes))
}

/// Recovers the canonical TRU address (starts with 'T') from a TRU signed message and 65-byte signature.
pub fn recover_tru_address(
    message_text: &str,
    signature_hex: &str,
) -> Result<String, String> {
    let clean_sig = signature_hex.strip_prefix("0x").unwrap_or(signature_hex).trim();
    let sig_bytes = hex::decode(clean_sig).map_err(|e| format!("Invalid hex signature: {}", e))?;
    if sig_bytes.len() != 65 {
        return Err(format!(
            "Signature must be exactly 65 bytes, received {}",
            sig_bytes.len()
        ));
    }

    let prefix = format!("\x18TRU Signed Message:\n{}", message_text.len());
    let mut data = Vec::with_capacity(prefix.len() + message_text.len());
    data.extend_from_slice(prefix.as_bytes());
    data.extend_from_slice(message_text.as_bytes());

    let prehash = crate::core::keys::sha256(&crate::core::keys::sha256(&data));
    let secp_msg = Message::from_digest(prehash);

    let mut v = sig_bytes[64];
    if v >= 27 {
        v -= 27;
    }
    if v > 3 {
        return Err(format!("Invalid signature recovery id v: {}", v));
    }

    let recovery_id = RecoveryId::from_i32(v as i32)
        .map_err(|e| format!("Invalid recovery ID: {}", e))?;

    let rec_sig = RecoverableSignature::from_compact(&sig_bytes[..64], recovery_id)
        .map_err(|e| format!("Failed to parse recoverable signature: {}", e))?;

    let secp = Secp256k1::new();
    let pubkey = secp
        .recover_ecdsa(&secp_msg, &rec_sig)
        .map_err(|e| format!("ECDSA recovery failed: {}", e))?;

    let compressed = pubkey.serialize();
    let pkh = crate::core::keys::hash160(&compressed);
    Ok(crate::core::keys::pkh_to_tru_address(&pkh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ecrecover_personal_sign() {
        let secp = Secp256k1::new();
        let mut secret_bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut secret_bytes);
        secret_bytes[0] &= 0x7f; // Ensure valid scalar
        if secret_bytes[0] == 0 { secret_bytes[0] = 1; }
        let secret_key = secp256k1::SecretKey::from_slice(&secret_bytes).unwrap();
        let public_key = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let expected_addr = pubkey_to_eth_address(&public_key);

        let msg_text = "TRUSWAP_INTENT:test_order_123";
        let prefix = format!("\x19Ethereum Signed Message:\n{}", msg_text.len());
        let mut eth_signed_data = Vec::new();
        eth_signed_data.extend_from_slice(prefix.as_bytes());
        eth_signed_data.extend_from_slice(msg_text.as_bytes());

        let prehash = keccak256(&eth_signed_data);
        let secp_msg = Message::from_digest(prehash);
        let rec_sig = secp.sign_ecdsa_recoverable(&secp_msg, &secret_key);

        let (recovery_id, compact) = rec_sig.serialize_compact();
        let mut sig_bytes = Vec::new();
        sig_bytes.extend_from_slice(&compact);
        sig_bytes.push(recovery_id.to_i32() as u8 + 27);

        let sig_hex = hex::encode(sig_bytes);
        let recovered = recover_personal_sign_address(msg_text, &sig_hex).unwrap();

        assert_eq!(recovered.to_lowercase(), expected_addr.to_lowercase());
    }

    #[test]
    fn test_evm_selectors() {
        assert_eq!(hex::encode(&keccak256(b"getSwap(bytes32)")[0..4]), "3da0e66e");
        assert_eq!(hex::encode(&keccak256(b"swaps(bytes32)")[0..4]), "eb84e7f2");
        assert_eq!(hex::encode(&keccak256(b"claim(bytes32,bytes)")[0..4]), "f0331024");
        assert_eq!(hex::encode(&keccak256(b"cooperativeRefund(bytes32)")[0..4]), "cbf90c7b");
    }

    #[test]
    fn test_sign_and_recover_take_order() {
        let secp = Secp256k1::new();
        let mut priv_bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut priv_bytes);
        priv_bytes[0] &= 0x7f;
        if priv_bytes[0] == 0 { priv_bytes[0] = 1; }
        let secret_key = secp256k1::SecretKey::from_slice(&priv_bytes).unwrap();
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let taker_base = pubkey_to_eth_address(&pubkey);

        let order_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let taker_tru = "TLXm111111111111111111111111111111";
        let msg = canonical_take_message(order_id, &taker_base, taker_tru);

        let sig = sign_personal_message(&msg, &priv_bytes).expect("signing failed");
        let recovered = recover_personal_sign_address(&msg, &sig).expect("recovery failed");

        assert_eq!(recovered.to_lowercase(), taker_base.to_lowercase());

        // Tampered message should not recover the same address
        let tampered_msg = canonical_take_message(order_id, &taker_base, "TLXmAttackerAddress");
        let tampered_recovered = recover_personal_sign_address(&tampered_msg, &sig);
        assert!(tampered_recovered.is_err() || tampered_recovered.unwrap().to_lowercase() != taker_base.to_lowercase());
    }

    #[test]
    fn test_canonical_take_message_normalization_both_ways() {
        let secp = Secp256k1::new();
        let mut priv_bytes = [0u8; 32];
        priv_bytes[0] = 42;
        let secret_key = secp256k1::SecretKey::from_slice(&priv_bytes).unwrap();
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let pubkey_bytes = pubkey.serialize();
        let pubkey_hex = hex::encode(pubkey_bytes);

        let pkh = crate::core::keys::hash160(&pubkey_bytes);
        let tru_addr = crate::core::keys::pkh_to_tru_address(&pkh);

        let order_id = "abc123order";
        let taker_base = "0x1111111111111111111111111111111111111111";

        let msg_from_addr = canonical_take_message(order_id, taker_base, &tru_addr);
        let msg_from_pubkey = canonical_take_message(order_id, taker_base, &pubkey_hex);

        assert_eq!(msg_from_addr, msg_from_pubkey);
        assert!(msg_from_addr.ends_with(&tru_addr));
    }

    #[test]
    fn test_sign_and_recover_tru_message() {
        let secp = Secp256k1::new();
        let mut priv_bytes = [0u8; 32];
        priv_bytes[0] = 77;
        let secret_key = secp256k1::SecretKey::from_slice(&priv_bytes).unwrap();
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let compressed = pubkey.serialize();
        let pkh = crate::core::keys::hash160(&compressed);
        let expected_tru_addr = crate::core::keys::pkh_to_tru_address(&pkh);

        let order_id = "595d624dc36418b36b86b99e6e3065095c6afb9a5e6483c489052a100639a87a";
        let cancel_msg = canonical_cancel_message(order_id, &expected_tru_addr);

        let sig = sign_tru_message(&cancel_msg, &priv_bytes).expect("signing failed");
        let recovered = recover_tru_address(&cancel_msg, &sig).expect("recovery failed");

        assert_eq!(recovered, expected_tru_addr);

        // Tampered message fails
        let tampered_msg = canonical_cancel_message("otherorderid", &expected_tru_addr);
        let tampered_res = recover_tru_address(&tampered_msg, &sig);
        assert!(tampered_res.is_err() || tampered_res.unwrap() != expected_tru_addr);
    }
}

