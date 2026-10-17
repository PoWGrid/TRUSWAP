/// Writes Bitcoin/TRU CompactSize VarInt
pub fn write_var_int(out: &mut Vec<u8>, val: u64) {
    if val < 0xfd {
        out.push(val as u8);
    } else if val <= 0xffff {
        out.push(0xfd);
        out.extend_from_slice(&(val as u16).to_le_bytes());
    } else if val <= 0xffffffff {
        out.push(0xfe);
        out.extend_from_slice(&(val as u32).to_le_bytes());
    } else {
        out.push(0xff);
        out.extend_from_slice(&val.to_le_bytes());
    }
}

/// Reads Bitcoin/TRU CompactSize VarInt from a slice. Returns (value, bytes_read).
pub fn read_var_int(bytes: &[u8]) -> Option<(u64, usize)> {
    if bytes.is_empty() {
        return None;
    }
    match bytes[0] {
        n @ 0..=0xfc => Some((n as u64, 1)),
        0xfd => {
            if bytes.len() < 3 {
                return None;
            }
            let val = u16::from_le_bytes([bytes[1], bytes[2]]);
            Some((val as u64, 3))
        }
        0xfe => {
            if bytes.len() < 5 {
                return None;
            }
            let val = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            Some((val as u64, 5))
        }
        0xff => {
            if bytes.len() < 9 {
                return None;
            }
            let val = u64::from_le_bytes([
                bytes[1], bytes[2], bytes[3], bytes[4],
                bytes[5], bytes[6], bytes[7], bytes[8],
            ]);
            Some((val, 9))
        }
    }
}

/// CompactSize / VarInt size in bytes for a given integer count
pub fn varint_len(val: usize) -> usize {
    if val < 0xfd {
        1
    } else if val <= 0xffff {
        3
    } else if val <= 0xffffffff {
        5
    } else {
        9
    }
}

/// Standard constants for TRU P2PKH transactions
pub const P2PKH_INPUT_BYTES: usize = 148; // 32 (txid) + 4 (vout) + 1 (varint len) + 107 (DER sig + pubkey) + 4 (sequence)
pub const P2PKH_OUTPUT_BYTES: usize = 34;  // 8 (atoms) + 1 (varint len) + 25 (scriptPubKey 76a914...88ac)
pub const HTLC_OUTPUT_BYTES: usize = 112;  // 8 (atoms) + 1 (varint len) + 103 (canonical HTLC script)
pub const TX_OVERHEAD_BYTES: usize = 8;    // 4 (version) + 4 (locktime)
pub const SAFETY_BUFFER_ATOMS: u64 = 10;   // Exactly +10 atoms insurance buffer
pub const MIN_RELAY_FEE_PER_BYTE: u64 = 1; // 1 atom per byte

/// Computes the exact serialized byte length of a standard TRU transaction.
pub fn estimate_tx_vsize(vin_count: usize, p2pkh_vout_count: usize, htlc_vout_count: usize) -> usize {
    let vin_cnt = if vin_count == 0 { 1 } else { vin_count };
    let total_vouts = p2pkh_vout_count + htlc_vout_count;
    let vout_cnt = if total_vouts == 0 { 1 } else { total_vouts };

    TX_OVERHEAD_BYTES
        + varint_len(vin_cnt)
        + (vin_cnt * P2PKH_INPUT_BYTES)
        + varint_len(vout_cnt)
        + (p2pkh_vout_count * P2PKH_OUTPUT_BYTES)
        + (htlc_vout_count * HTLC_OUTPUT_BYTES)
}

/// Computes exact transaction fee in atoms:
/// `fee = (tx_bytes * MIN_RELAY_FEE_PER_BYTE) + 10_atoms_buffer`
pub fn calculate_exact_fee_atoms(vin_count: usize, p2pkh_vout_count: usize, htlc_vout_count: usize) -> u64 {
    let tx_bytes = estimate_tx_vsize(vin_count, p2pkh_vout_count, htlc_vout_count);
    (tx_bytes as u64 * MIN_RELAY_FEE_PER_BYTE) + SAFETY_BUFFER_ATOMS
}

/// Converts atom fee to TRU with 8 decimal places
pub fn calculate_exact_fee_tru(vin_count: usize, p2pkh_vout_count: usize, htlc_vout_count: usize) -> f64 {
    calculate_exact_fee_atoms(vin_count, p2pkh_vout_count, htlc_vout_count) as f64 / 100_000_000.0
}
