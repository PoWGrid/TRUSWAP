use std::path::Path;

fn main() {
    let data_dir = std::env::args().nth(1).unwrap_or_else(|| "./data".to_string());
    println!("Opening vault at: {}", data_dir);
    match truswap::wallet::keystore::EncryptedVault::open(Path::new(&data_dir)) {
        Ok(mut vault) => {
            println!("Vault opened successfully!");
            match vault.export_wallet() {
                Ok(w) => {
                    println!("Wallet info:");
                    println!("  TRU Address  : {}", w.address);
                    println!("  TRU PrivHex  : {}", w.priv_key_hex);
                    println!("  TRU WIF      : {}", w.priv_key_wif);
                    println!("  Base Address : {}", w.base_address);
                    println!("  Base PrivHex : {}", w.base_priv_key_hex);
                    println!("  Base Mnemonic: {}", w.base_mnemonic);
                }
                Err(e) => println!("Error exporting wallet: {}", e),
            }
            match vault.load_records() {
                Ok(mut records) => {
                    println!("Found {} records:", records.len());
                    for (i, r) in records.iter().enumerate() {
                        println!(
                            "[{}] swap_id=0x{} hash160=0x{} direction={} step={} status={} tru_atoms={} usdc_atomic={} beneficiary={}",
                            i,
                            hex::encode(r.swap_id),
                            hex::encode(r.hash160),
                            r.direction,
                            r.step,
                            r.status,
                            r.tru_atoms,
                            r.usdc_atomic,
                            r.beneficiary
                        );
                    }
                    if let Some(idx_str) = std::env::args().nth(2) {
                        if let Ok(idx) = idx_str.parse::<usize>() {
                            let new_status = std::env::args().nth(3).and_then(|s| s.parse::<u8>().ok()).unwrap_or(2);
                            if idx < records.len() {
                                records[idx].status = new_status;
                                if let Err(e) = vault.save_records(&records) {
                                    println!("Failed to save records: {}", e);
                                } else {
                                    println!("Marked record [{}] with status={}", idx, new_status);
                                }
                            }
                        }
                    }
                }
                Err(e) => println!("Error loading records: {}", e),
            }
        }
        Err(e) => println!("Error opening vault: {}", e),
    }
}
