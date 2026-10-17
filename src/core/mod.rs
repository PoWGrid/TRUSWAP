pub mod fee;
pub mod keys;
pub mod script;
pub mod sighash;
pub mod transaction;

pub use fee::*;
pub use keys::*;
pub use script::*;
pub use transaction::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash160_matches_canonical() {
        let preimage = b"TRU_ATOMIC_SWAP_SECRET_2026";
        let h160 = hash160(preimage);
        assert_eq!(h160.len(), 20);
        assert_ne!(h160, [0u8; 20]);
    }

    #[test]
    fn test_tru_genesis_address_derivation_and_roundtrip() {
        let addr = "TRU88888rDZc2EAWLBnuc5cY3CUNRd7zWk";
        let pkh = tru_address_to_pkh(addr).expect("Valid TRU address");
        assert_eq!(pkh.len(), 20);
        let derived = pkh_to_tru_address(&pkh);
        assert_eq!(derived, addr);
    }

    #[test]
    fn test_secp256k1_ephemeral_keypair_derivation() {
        let mut priv_arr = [0u8; 32];
        priv_arr[31] = 1;

        let kp = keypair_from_bytes(&priv_arr).expect("Valid secp256k1 key");
        assert_eq!(kp.address, "TLeUZDGLWnyiJVFcp3m3M1782uBsGWa8uf");
        assert_eq!(
            hex::encode(kp.pub_key),
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        );
    }

    #[test]
    fn test_exact_byte_and_fee_calculation_with_10_atoms_safety() {
        let fee_1in_1out = calculate_exact_fee_atoms(1, 1, 0);
        assert_eq!(fee_1in_1out, 192 + 10);

        let fee_1in_2out = calculate_exact_fee_atoms(1, 2, 0);
        assert_eq!(fee_1in_2out, 226 + 10);

        let fee_1in_1htlc = calculate_exact_fee_atoms(1, 1, 1);
        assert_eq!(fee_1in_1htlc, 304 + 10);
    }

    #[test]
    fn test_raw_transaction_sighash_and_signing() {
        let mut priv_arr = [0u8; 32];
        priv_arr[31] = 1;
        let kp = keypair_from_bytes(&priv_arr).unwrap();

        let mut tx = RawTransaction::new();
        tx.inputs.push(TxInput {
            txid: "258232591a9c284111966566dee1029a6f78f2ba8daf2b623f7edefda0b3035d".to_string(),
            vout: 0,
            script_sig: Vec::new(),
            sequence: 0xffffffff,
            pub_key: Vec::new(),
        });

        let dest_spk = address_to_script_pubkey(&kp.address).unwrap();
        tx.outputs.push(TxOutput {
            amount_atoms: 100_000_000,
            script_pubkey: dest_spk.clone(),
        });

        tx.sign_p2pkh_input(0, &dest_spk, &kp.priv_key, &kp.pub_key).expect("Sign p2pkh input");
        let serialized = tx.serialize_binary();
        assert!(!serialized.is_empty());
        let txid = tx.compute_txid();
        assert_eq!(txid.len(), 64);

        let roundtrip_tx = RawTransaction::deserialize_binary(&serialized).expect("Deserialize binary");
        assert_eq!(roundtrip_tx.inputs.len(), 1);
        assert_eq!(roundtrip_tx.outputs.len(), 1);
        assert_eq!(roundtrip_tx.outputs[0].amount_atoms, 100_000_000);
        assert_eq!(roundtrip_tx.compute_txid(), txid);

        let pipe_str = tx.to_pipe_string(&txid);
        assert!(pipe_str.starts_with(&format!("{}|0|1|1|", txid)));
    }
}
