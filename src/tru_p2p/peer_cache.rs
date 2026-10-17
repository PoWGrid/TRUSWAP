use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use tracing::{debug, info, warn};

pub const DEFAULT_PEERS_CACHE_FILE: &str = "peers.dat";

/// Loads previously gossiped and verified peer socket addresses from compressed peers.dat.
pub fn load_cached_peers(data_dir: &Path) -> Vec<String> {
    let path = data_dir.join(DEFAULT_PEERS_CACHE_FILE);
    if !path.exists() {
        return Vec::new();
    }

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            warn!("[TRU-PEER-CACHE] Failed to open {}: {}", path.display(), e);
            return Vec::new();
        }
    };

    let reader = BufReader::new(file);
    let decoder = GzDecoder::new(reader);
    match serde_json::from_reader::<_, Vec<String>>(decoder) {
        Ok(peers) => {
            let valid_peers: Vec<String> = peers
                .into_iter()
                .filter(|p| !p.trim().is_empty() && p.contains(':'))
                .collect();
            info!(
                "[TRU-PEER-CACHE] Successfully loaded {} persistent peer(s) from {}",
                valid_peers.len(),
                path.display()
            );
            valid_peers
        }
        Err(e) => {
            warn!(
                "[TRU-PEER-CACHE] Corrupted peers.dat at {}: {}",
                path.display(),
                e
            );
            Vec::new()
        }
    }
}

/// Atomically saves the known set of peer endpoints into compressed peers.dat using Gzip.
pub fn save_cached_peers(data_dir: &Path, peers: &HashSet<String>) -> Result<(), String> {
    if peers.is_empty() {
        return Ok(());
    }

    let _ = fs::create_dir_all(data_dir);
    let final_path = data_dir.join(DEFAULT_PEERS_CACHE_FILE);
    let temp_path = data_dir.join(format!("{}.tmp.{}", DEFAULT_PEERS_CACHE_FILE, std::process::id()));

    let list: Vec<&String> = peers.iter().filter(|p| !p.trim().is_empty()).collect();

    let file = File::create(&temp_path).map_err(|e| format!("Failed to create temp peers file: {}", e))?;
    let writer = BufWriter::new(file);
    let encoder = GzEncoder::new(writer, Compression::default());

    serde_json::to_writer(encoder, &list).map_err(|e| format!("Failed to serialize peers: {}", e))?;

    fs::rename(&temp_path, &final_path).map_err(|e| {
        let _ = fs::remove_file(&temp_path);
        format!("Failed to atomically rename peers file: {}", e)
    })?;

    debug!(
        "[TRU-PEER-CACHE] Atomically saved {} peers to {}",
        list.len(),
        final_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peers_cache_roundtrip() {
        let temp_dir = std::env::temp_dir().join("truswap_test_peers_cache");
        let _ = fs::create_dir_all(&temp_dir);

        let mut sample_peers = HashSet::new();
        sample_peers.insert("1.2.3.4:21833".to_string());
        sample_peers.insert("5.6.7.8:21833".to_string());
        sample_peers.insert("9.10.11.12:21833".to_string());

        assert!(save_cached_peers(&temp_dir, &sample_peers).is_ok());

        let loaded = load_cached_peers(&temp_dir);
        assert_eq!(loaded.len(), 3);
        for p in &sample_peers {
            assert!(loaded.contains(p));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
