use crate::core::keys::sha256;
use crate::core::transaction::RawTransaction;

impl RawTransaction {
    /// Computes SIGHASH_ALL digest for a specific input
    pub fn get_sighash(&self, input_index: usize, script_pubkey: &[u8]) -> [u8; 32] {
        let mut temp = self.clone();
        for inp in &mut temp.inputs {
            inp.script_sig.clear();
        }
        temp.inputs[input_index].script_sig = script_pubkey.to_vec();
        let mut bin = temp.serialize_binary();
        // Append SIGHASH_ALL as 4-byte LE: 0x01000000
        bin.extend_from_slice(&1u32.to_le_bytes());
        let h1 = sha256(&bin);
        sha256(&h1)
    }

    /// Signs an input with a SECP256k1 signing key (Low-S canonical DER + SIGHASH_ALL)
    pub fn sign_p2pkh_input(
        &mut self,
        input_index: usize,
        script_pubkey: &[u8],
        priv_key_bytes: &[u8; 32],
        pub_key: &[u8; 33],
    ) -> Result<(), String> {
        let sighash = self.get_sighash(input_index, script_pubkey);
        let signing_key = k256::ecdsa::SigningKey::from_bytes(priv_key_bytes.into())
            .map_err(|e| format!("Invalid signing key: {}", e))?;

        use k256::ecdsa::signature::hazmat::PrehashSigner;
        let (sig, _): (k256::ecdsa::Signature, _) = signing_key
            .sign_prehash(&sighash)
            .map_err(|e| format!("Signing failed: {}", e))?;
        let sig_low_s = sig.normalize_s();
        let der = sig_low_s.to_der();

        let mut script_sig = Vec::with_capacity(1 + der.as_bytes().len() + 1 + 1 + 33);
        // Push signature (including 0x01 sighash byte)
        script_sig.push((der.as_bytes().len() + 1) as u8);
        script_sig.extend_from_slice(der.as_bytes());
        script_sig.push(0x01); // SIGHASH_ALL
        // Push 33-byte compressed public key
        script_sig.push(33);
        script_sig.extend_from_slice(pub_key);

        self.inputs[input_index].script_sig = script_sig;
        self.inputs[input_index].pub_key = pub_key.to_vec();
        Ok(())
    }
}
