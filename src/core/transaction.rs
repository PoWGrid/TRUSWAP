use crate::core::fee::{read_var_int, write_var_int};
use crate::core::keys::sha256;

#[derive(Clone, Debug, Default)]
pub struct TxInput {
    pub txid: String, // 64 hex chars
    pub vout: u32,
    pub script_sig: Vec<u8>,
    pub sequence: u32,
    pub pub_key: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct TxOutput {
    pub amount_atoms: u64,
    pub script_pubkey: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct RawTransaction {
    pub version: u32,
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
    pub lock_time: u32,
}

impl RawTransaction {
    pub fn new() -> Self {
        Self {
            version: 1,
            inputs: Vec::new(),
            outputs: Vec::new(),
            lock_time: 0,
        }
    }

    /// Serializes binary according to TRU wire format
    pub fn serialize_binary(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.version.to_le_bytes());
        write_var_int(&mut out, self.inputs.len() as u64);
        for input in &self.inputs {
            let txid_bytes = hex::decode(&input.txid).unwrap_or_else(|_| vec![0u8; 32]);
            out.extend_from_slice(&txid_bytes);
            out.extend_from_slice(&input.vout.to_le_bytes());
            write_var_int(&mut out, input.script_sig.len() as u64);
            out.extend_from_slice(&input.script_sig);
            out.extend_from_slice(&input.sequence.to_le_bytes());
        }
        write_var_int(&mut out, self.outputs.len() as u64);
        for output in &self.outputs {
            out.extend_from_slice(&output.amount_atoms.to_le_bytes());
            write_var_int(&mut out, output.script_pubkey.len() as u64);
            out.extend_from_slice(&output.script_pubkey);
        }
        out.extend_from_slice(&self.lock_time.to_le_bytes());
        // metadata flag: 0 = no metadata
        out.push(0);
        out
    }

    /// Computes canonical transaction ID (double SHA256 of binary with empty scriptSigs)
    pub fn compute_txid(&self) -> String {
        let mut stripped = self.clone();
        for inp in &mut stripped.inputs {
            inp.script_sig.clear();
        }
        let bin = stripped.serialize_binary();
        let h1 = sha256(&bin);
        let h2 = sha256(&h1);
        hex::encode(h2)
    }

    /// Converts transaction to canonical TRU pipe-delimited text format required by C++ nodes on P2P wire:
    /// `txid|isCoinbase|vin_count|vout_count|in0_txid|in0_vout|in0_scriptSigHex|in0_pubKeyHex|in0_seq|...|out0_amt|out0_spk|...[|TXCORE:version:locktime]`
    pub fn to_pipe_string(&self, txid: &str) -> String {
        let mut parts = vec![
            txid.to_string(),
            "0".to_string(), // isCoinbase: false
            self.inputs.len().to_string(),
            self.outputs.len().to_string(),
        ];
        for inp in &self.inputs {
            parts.push(inp.txid.clone());
            parts.push(inp.vout.to_string());
            parts.push(hex::encode(&inp.script_sig));
            parts.push(hex::encode(&inp.pub_key));
            parts.push(inp.sequence.to_string());
        }
        for out in &self.outputs {
            parts.push(out.amount_atoms.to_string());
            parts.push(hex::encode(&out.script_pubkey));
        }
        if self.version != 1 || self.lock_time != 0 {
            parts.push(format!("TXCORE:{}:{}", self.version, self.lock_time));
        }
        parts.join("|")
    }

    /// Deserializes a raw binary wire transaction into a RawTransaction
    pub fn deserialize_binary(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = 0;
        if bytes.len() < 4 {
            return Err("Payload too short for version".to_string());
        }
        let version = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;

        let (vin_count, read) = read_var_int(&bytes[cursor..]).ok_or("Failed to read vin_count")?;
        cursor += read;
        let mut inputs = Vec::with_capacity(vin_count as usize);
        for _ in 0..vin_count {
            if cursor + 36 > bytes.len() {
                return Err("Truncated input header".to_string());
            }
            let txid = hex::encode(&bytes[cursor..cursor + 32]);
            cursor += 32;
            let vout = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let (sig_len, read) = read_var_int(&bytes[cursor..]).ok_or("Failed to read script_sig length")?;
            cursor += read;
            if cursor + (sig_len as usize) > bytes.len() {
                return Err("Truncated script_sig".to_string());
            }
            let script_sig = bytes[cursor..cursor + (sig_len as usize)].to_vec();
            cursor += sig_len as usize;

            if cursor + 4 > bytes.len() {
                return Err("Truncated sequence".to_string());
            }
            let sequence = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;

            inputs.push(TxInput {
                txid,
                vout,
                script_sig,
                sequence,
                pub_key: Vec::new(),
            });
        }

        let (vout_count, read) = read_var_int(&bytes[cursor..]).ok_or("Failed to read vout_count")?;
        cursor += read;
        let mut outputs = Vec::with_capacity(vout_count as usize);
        for _ in 0..vout_count {
            if cursor + 8 > bytes.len() {
                return Err("Truncated output amount".to_string());
            }
            let amount_atoms = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
            cursor += 8;
            let (spk_len, read) = read_var_int(&bytes[cursor..]).ok_or("Failed to read script_pubkey length")?;
            cursor += read;
            if cursor + (spk_len as usize) > bytes.len() {
                return Err("Truncated script_pubkey".to_string());
            }
            let script_pubkey = bytes[cursor..cursor + (spk_len as usize)].to_vec();
            cursor += spk_len as usize;

            outputs.push(TxOutput {
                amount_atoms,
                script_pubkey,
            });
        }

        let lock_time = if cursor + 4 <= bytes.len() {
            u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap())
        } else {
            0
        };

        Ok(RawTransaction {
            version,
            inputs,
            outputs,
            lock_time,
        })
    }
}
