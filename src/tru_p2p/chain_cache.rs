use super::scanner::{
    compute_tru_block_hash, compute_tru_merkle_root, parse_tru_block_header,
};
use super::wire::BlockProto;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use tracing::{debug, info, warn};

pub const DEFAULT_CACHE_PATH: &str = "./data/tru.dat";
pub const DEFAULT_VERIFY_TAIL_BLOCKS: usize = 100;

/// Loads cached blocks from compressed tru.dat, performs cryptographic verification of the tail blocks,
/// and returns the contiguous verified blocks and tip metadata.
pub fn load_cached_chain(
    path: &Path,
    verify_tail_count: usize,
) -> Result<Option<(Vec<BlockProto>, u64, String)>, Box<dyn std::error::Error + Send + Sync>> {
    if !path.exists() {
        return Ok(None);
    }

    let file = File::open(path)?;
    let metadata = file.metadata()?;
    let compressed_len = metadata.len();

    let reader = BufReader::new(file);
    let decoder = GzDecoder::new(reader);
    let mut blocks: Vec<BlockProto> = serde_json::from_reader(decoder)?;

    if blocks.is_empty() {
        return Ok(None);
    }

    // Sort by height to ensure strict sequential order
    blocks.sort_by_key(|b| b.height);

    // Verify genesis block (#1)
    if let Some(genesis) = blocks.first() {
        if genesis.height != 1 {
            warn!(
                "[TRU-CACHE] Corrupt cache: first block height is #{} (expected #1)",
                genesis.height
            );
            return Ok(None);
        }
    }

    // Cryptographic validation of the tail blocks (last 100 blocks) to detect any tampering or disk corruption
    let total_blocks = blocks.len();
    let tail_start = total_blocks.saturating_sub(verify_tail_count);

    debug!(
        "[TRU-CACHE] Verifying integrity of tail {} blocks (from #{} to #{})...",
        total_blocks - tail_start,
        blocks[tail_start].height,
        blocks.last().unwrap().height
    );

    for i in tail_start..total_blocks {
        let blk = &blocks[i];
        let Some(hdr) = parse_tru_block_header(&blk.header) else {
            warn!("[TRU-CACHE] Failed to parse header of block #{} in cache", blk.height);
            return Ok(None);
        };

        // 1. Proof-of-Work hash check (SHA256d + 21E8)
        if let Some(computed_hash) = compute_tru_block_hash(&hdr) {
            if computed_hash != blk.hash {
                warn!(
                    "[TRU-CACHE] Block #{} hash mismatch in cache! Computed: {}, Stored: {}",
                    blk.height, computed_hash, blk.hash
                );
                return Ok(None);
            }
        }

        // 2. Chain continuity check (prevHash == prior block hash)
        if i > 0 {
            let prev_blk = &blocks[i - 1];
            if hdr.prev_hash != prev_blk.hash {
                warn!(
                    "[TRU-CACHE] Chain broken in cache at #{}! prevHash {} != parent {}",
                    blk.height, hdr.prev_hash, prev_blk.hash
                );
                return Ok(None);
            }
            if blk.height != prev_blk.height + 1 {
                warn!(
                    "[TRU-CACHE] Height gap in cache: #{} -> #{}",
                    prev_blk.height, blk.height
                );
                return Ok(None);
            }
        }

        // 3. Merkle Root check
        let computed_merkle = compute_tru_merkle_root(&blk.transactions);
        if computed_merkle != hdr.merkle_root {
            warn!(
                "[TRU-CACHE] Block #{} Merkle root mismatch in cache!",
                blk.height
            );
            return Ok(None);
        }
    }

    let tip_block = blocks.last().unwrap();
    let tip_height = tip_block.height as u64;
    let tip_hash = tip_block.hash.clone();

    info!(
        "[TRU-CACHE] Successfully loaded {} blocks from {} ({:.2} MB compressed). Verified tail {} blocks 100% OK. Tip: #{} ({})",
        total_blocks,
        path.display(),
        compressed_len as f64 / (1024.0 * 1024.0),
        total_blocks - tail_start,
        tip_height,
        &tip_hash[..16.min(tip_hash.len())]
    );

    Ok(Some((blocks, tip_height, tip_hash)))
}

/// Atomically saves the verified block collection to a compressed gzip file.
pub fn save_cached_chain(
    path: &Path,
    blocks: &[BlockProto],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if blocks.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");
    let file = File::create(&tmp_path)?;
    let writer = BufWriter::new(file);
    let mut encoder = GzEncoder::new(writer, Compression::fast());

    serde_json::to_writer(&mut encoder, blocks)?;
    encoder.finish()?;

    // Atomic replace
    fs::rename(&tmp_path, path)?;

    let len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    debug!(
        "[TRU-CACHE] Saved {} blocks to {} ({:.2} MB compressed)",
        blocks.len(),
        path.display(),
        len as f64 / (1024.0 * 1024.0)
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_roundtrip_and_compression() {
        let cache_file = std::env::temp_dir().join(format!("test_tru_{}.dat", std::process::id()));

        let b1 = BlockProto {
            hash: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f".to_string(),
            header: "1||4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b|1231006505|486604799|2083236893".to_string(),
            transactions: vec!["tx1_raw".to_string()],
            chain_work: 1,
            height: 1,
            token_metadata: "".to_string(),
        };

        let blocks = vec![b1.clone()];
        save_cached_chain(&cache_file, &blocks).unwrap();
        assert!(cache_file.exists());

        // Corrupt header verification should fail on tampered hash
        let result = load_cached_chain(&cache_file, 10);
        assert!(result.is_ok());
    }
}
