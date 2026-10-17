use crate::core::keys::tru_address_to_pkh;

/// Converts a TRU mainnet address to its canonical P2PKH scriptPubKey: `76a914<pkh>88ac`
pub fn tru_address_to_spk(addr: &str) -> Option<String> {
    let pkh = tru_address_to_pkh(addr)?;
    Some(format!("76a914{}88ac", hex::encode(pkh)))
}

/// Converts a TRU P2PKH address into scriptPubKey bytes
pub fn address_to_script_pubkey(addr: &str) -> Result<Vec<u8>, String> {
    let pkh = tru_address_to_pkh(addr)
        .ok_or_else(|| format!("Invalid TRU address: {}", addr))?;
    let mut spk = Vec::with_capacity(25);
    spk.push(0x76); // OP_DUP
    spk.push(0xa9); // OP_HASH160
    spk.push(0x14); // PUSH 20
    spk.extend_from_slice(&pkh);
    spk.push(0x88); // OP_EQUALVERIFY
    spk.push(0xac); // OP_CHECKSIG
    Ok(spk)
}

/// Constructs canonical 103-byte TRU HTLC script:
/// Format:
/// OP_IF OP_HASH160 <20-byte hash160> OP_EQUALVERIFY <33-byte claim_pubkey> OP_CHECKSIG
/// OP_ELSE <4-byte timelock> OP_CHECKLOCKTIMEVERIFY OP_DROP <33-byte refund_pubkey> OP_CHECKSIG OP_ENDIF
pub fn build_canonical_htlc_script(
    hash160: &[u8; 20],
    claim_pubkey: &[u8; 33],
    refund_pubkey: &[u8; 33],
    timelock: u32,
) -> Vec<u8> {
    let mut script = Vec::with_capacity(103);
    script.push(0x63); // OP_IF
    script.push(0xa9); // OP_HASH160
    script.push(0x14); // Push 20 bytes
    script.extend_from_slice(hash160);
    script.push(0x88); // OP_EQUALVERIFY
    script.push(0x21); // Push 33 bytes
    script.extend_from_slice(claim_pubkey);
    script.push(0xac); // OP_CHECKSIG
    script.push(0x67); // OP_ELSE
    script.push(0x04); // Push 4 bytes
    script.extend_from_slice(&timelock.to_le_bytes());
    script.push(0xb1); // OP_CHECKLOCKTIMEVERIFY
    script.push(0x75); // OP_DROP
    script.push(0x21); // Push 33 bytes
    script.extend_from_slice(refund_pubkey);
    script.push(0xac); // OP_CHECKSIG
    script.push(0x68); // OP_ENDIF
    script
}

/// Parsed HTLC Script Components
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedHtlcScript {
    pub hash160: [u8; 20],
    pub claim_pubkey: [u8; 33],
    pub refund_pubkey: [u8; 33],
    pub timelock: u32,
}

/// Inspects if a script matches the canonical 103-byte TRU HTLC format
pub fn inspect_canonical_htlc_script(script: &[u8]) -> Option<ParsedHtlcScript> {
    if script.len() != 103 {
        return None;
    }
    if script[0] != 0x63 || script[1] != 0xa9 || script[2] != 0x14 {
        return None;
    }
    let mut hash160 = [0u8; 20];
    hash160.copy_from_slice(&script[3..23]);

    if script[23] != 0x88 || script[24] != 0x21 {
        return None;
    }
    let mut claim_pubkey = [0u8; 33];
    claim_pubkey.copy_from_slice(&script[25..58]);

    if script[58] != 0xac || script[59] != 0x67 || script[60] != 0x04 {
        return None;
    }
    let timelock = u32::from_le_bytes(script[61..65].try_into().ok()?);

    if script[65] != 0xb1 || script[66] != 0x75 || script[67] != 0x21 {
        return None;
    }
    let mut refund_pubkey = [0u8; 33];
    refund_pubkey.copy_from_slice(&script[68..101]);

    if script[101] != 0xac || script[102] != 0x68 {
        return None;
    }

    Some(ParsedHtlcScript {
        hash160,
        claim_pubkey,
        refund_pubkey,
        timelock,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum HtlcValidationError {
    NonCanonicalScript,
    HashMismatch,
    ClaimPubkeyMismatch,
    Underfunded { actual_atoms: u64, expected_atoms: u64 },
    TimelockExpiredOrTooShort { script_timelock: u32, min_timelock: u32 },
}

impl std::fmt::Display for HtlcValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonCanonicalScript => write!(f, "Script is not a canonical 103-byte TRU HTLC"),
            Self::HashMismatch => write!(f, "Hash160 does not match order"),
            Self::ClaimPubkeyMismatch => write!(f, "Claim pubkey does not match our vault public key (potential hijacking attack)"),
            Self::Underfunded { actual_atoms, expected_atoms } => write!(f, "Underfunded: output has {} atoms, expected at least {} atoms", actual_atoms, expected_atoms),
            Self::TimelockExpiredOrTooShort { script_timelock, min_timelock } => write!(f, "Timelock is too short or expired: {} < {}", script_timelock, min_timelock),
        }
    }
}

impl std::error::Error for HtlcValidationError {}

/// Validates an on-chain TRU HTLC output against expected swap terms for a Buyer.
/// Defends against Under-funding, Pubkey Hijacking, Malformed Scripts, and Timelock Spoofing.
pub fn validate_buyer_tru_htlc(
    raw_script: &[u8],
    actual_atoms: u64,
    expected_hash160: &[u8; 20],
    expected_claim_pubkey: &[u8; 33],
    min_expected_atoms: u64,
    min_timelock: u32,
) -> Result<ParsedHtlcScript, HtlcValidationError> {
    let parsed = inspect_canonical_htlc_script(raw_script)
        .ok_or(HtlcValidationError::NonCanonicalScript)?;

    if &parsed.hash160 != expected_hash160 {
        return Err(HtlcValidationError::HashMismatch);
    }

    if &parsed.claim_pubkey != expected_claim_pubkey {
        return Err(HtlcValidationError::ClaimPubkeyMismatch);
    }

    // Anti Under-funding defense:
    // Allow at most 0.01% floating-point rounding margin:
    let required_atoms = min_expected_atoms.saturating_mul(9999) / 10000;
    if actual_atoms < required_atoms {
        return Err(HtlcValidationError::Underfunded {
            actual_atoms,
            expected_atoms: min_expected_atoms,
        });
    }

    if min_timelock > 0 && parsed.timelock < min_timelock {
        return Err(HtlcValidationError::TimelockExpiredOrTooShort {
            script_timelock: parsed.timelock,
            min_timelock,
        });
    }

    Ok(parsed)
}

/// Minimum safety margin delta required between the first and second on-chain locks.
/// Guarantees the responder has sufficient time to claim funds on counter-chain once the secret is revealed.
pub const MIN_TIMELOCK_DELTA_SECS: u32 = 1800; // 30 minutes minimum

/// Minimum remaining validity required on the first lock to safely accept the trade and create the second lock.
pub const MIN_FIRST_LOCK_REMAINING_SECS: u32 = 2400; // 40 minutes minimum

/// Calculates the safe asymmetric timelock for the second (responder) lock in an atomic swap.
/// The second lock MUST expire at least delta_t BEFORE the first lock to prevent race-condition refunds:
/// T_responder <= T_initiator - delta_t.
pub fn calculate_asymmetric_timelock(
    first_lock_timelock: u32,
    now_sec: u32,
) -> Result<u32, String> {
    if first_lock_timelock <= now_sec {
        return Err("First lock timelock has already expired".to_string());
    }
    let remaining = first_lock_timelock - now_sec;
    if remaining < MIN_FIRST_LOCK_REMAINING_SECS {
        return Err(format!(
            "Insufficient timelock remaining on first lock: {}s remaining, minimum required is {}s",
            remaining, MIN_FIRST_LOCK_REMAINING_SECS
        ));
    }

    // Delta is 1 hour (3600s) if remaining >= 7200s (2h), otherwise 30 minutes (1800s):
    let delta = if remaining >= 7200 {
        3600
    } else {
        MIN_TIMELOCK_DELTA_SECS
    };

    if remaining <= delta + 300 {
        return Err(format!(
            "Remaining timelock ({}s) is too close to safety delta ({}s)",
            remaining, delta
        ));
    }

    Ok(first_lock_timelock.saturating_sub(delta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_htlc_and_validation() {
        let hash160 = [0xabu8; 20];
        let claim_pk = [0x02u8; 33];
        let refund_pk = [0x03u8; 33];
        let timelock = 1790000000;

        let script = build_canonical_htlc_script(&hash160, &claim_pk, &refund_pk, timelock);
        assert_eq!(script.len(), 103);

        let parsed = inspect_canonical_htlc_script(&script).expect("Should inspect canonical script");
        assert_eq!(parsed.hash160, hash160);
        assert_eq!(parsed.claim_pubkey, claim_pk);
        assert_eq!(parsed.refund_pubkey, refund_pk);
        assert_eq!(parsed.timelock, timelock);

        // Valid validation
        let res = validate_buyer_tru_htlc(&script, 100_000_000, &hash160, &claim_pk, 100_000_000, 1780000000);
        assert!(res.is_ok());

        // Pubkey hijacking attempt (attacker pubkey in script)
        let attacker_pk = [0x04u8; 33];
        let hijacked_res = validate_buyer_tru_htlc(&script, 100_000_000, &hash160, &attacker_pk, 100_000_000, 1780000000);
        assert_eq!(hijacked_res, Err(HtlcValidationError::ClaimPubkeyMismatch));

        // Under-funding attempt
        let underfunded_res = validate_buyer_tru_htlc(&script, 10_000, &hash160, &claim_pk, 100_000_000, 1780000000);
        assert!(matches!(underfunded_res, Err(HtlcValidationError::Underfunded { .. })));

        // Timelock expired / too short
        let timelock_res = validate_buyer_tru_htlc(&script, 100_000_000, &hash160, &claim_pk, 100_000_000, 1800000000);
        assert!(matches!(timelock_res, Err(HtlcValidationError::TimelockExpiredOrTooShort { .. })));
    }

    #[test]
    fn test_calculate_asymmetric_timelock() {
        let now = 1_000_000;

        // 1. Normal 2-hour first lock -> 1-hour delta
        let first_tl = now + 7200;
        let second_tl = calculate_asymmetric_timelock(first_tl, now).expect("Should compute asymmetric timelock");
        assert_eq!(second_tl, first_tl - 3600);
        assert_eq!(second_tl - now, 3600);

        // 2. 1-hour first lock -> 30-minute delta
        let first_tl = now + 3600;
        let second_tl = calculate_asymmetric_timelock(first_tl, now).expect("Should compute asymmetric timelock");
        assert_eq!(second_tl, first_tl - 1800);
        assert_eq!(second_tl - now, 1800);

        // 3. First lock too short (less than 40m remaining) -> must error
        let first_tl = now + 2000;
        assert!(calculate_asymmetric_timelock(first_tl, now).is_err());

        // 4. Already expired first lock -> must error
        let first_tl = now - 100;
        assert!(calculate_asymmetric_timelock(first_tl, now).is_err());
    }
}
