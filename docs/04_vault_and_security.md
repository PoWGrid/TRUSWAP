# Chapter 4: Native Vault & Key Management

This chapter explains how your private keys are generated, stored, and protected in TRUSWAP, written for both first-time users and security auditors.

---

## 1. Vault Overview (For Beginners)

### What is the Dual-Vault?
TRUSWAP uses a **Dual-Vault** system. Think of it as a wallet with two separate pockets:

1. **The TRU Pocket**: Holds your native Tokenized Real Utility ($TRU) coins on the Proof-of-Work blockchain. Addresses start with `T`.
2. **The Base Pocket**: Holds your Ethereum Layer-2 assets (ETH for network fees and USDC stablecoins). Addresses start with `0x`.

<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">CRYPTOGRAPHY</span>
    <span class="diagram-heading">Dual-Vault Key Derivation & Account Architecture</span>
  </div>
  <div class="diagram-grid-2">
    <div class="diagram-node node-tru">
      <div class="node-badge">🟡 TRU ACCOUNT (POW CHAIN)</div>
      <div class="node-title">Native Bitcoin-like Script</div>
      <ul class="node-list">
        <li><strong>Entropy:</strong> 32-Byte SECP256k1 (256-bit)</li>
        <li><strong>Backup:</strong> 24-Word Mnemonic or WIF private key</li>
        <li><strong>Public Key:</strong> 33-Byte Compressed Pubkey</li>
        <li><strong>Hashing:</strong> HASH160 (SHA256 + RIPEMD160)</li>
        <li><strong>Address:</strong> Base58Check (starts with <code>T...</code>)</li>
      </ul>
    </div>
    <div class="diagram-node node-base">
      <div class="node-badge">🔵 BASE ACCOUNT (EVM LAYER-2)</div>
      <div class="node-title">Ethereum-Compatible Account</div>
      <ul class="node-list">
        <li><strong>Entropy:</strong> 12-Word BIP-39 (128-bit)</li>
        <li><strong>Derivation:</strong> Standard path <code>m/44'/60'/0'/0/0</code></li>
        <li><strong>Backup:</strong> 12-Word BIP-39 Mnemonic</li>
        <li><strong>Hashing:</strong> Keccak-256 (Last 20 Bytes)</li>
        <li><strong>Address:</strong> EIP-55 Checksum Hex (starts with <code>0x...</code>)</li>
      </ul>
    </div>
  </div>
</div>

### Why are there two different backup phrases?
A common question is: *"Why can't I back up everything with just one phrase?"*

- **TRU** is a Bitcoin-like Proof-of-Work blockchain. It uses a 256-bit private key. In TRUSWAP, this is represented as a **24-word BIP-39 mnemonic phrase** or a **WIF key** (a string of letters and numbers starting with `K` or `L`).
- **Base** is an EVM network compatible with Ethereum. It uses standard **12-word BIP-39 mnemonic phrases** following the industry-standard BIP-44 path (`m/44'/60'/0'/0/0`).

By keeping them independent:
- You can import an existing Ethereum wallet (like MetaMask or Rabby) directly into Base by entering your 12 words.
- Your TRU keys remain isolated from your EVM keys, ensuring an issue on one network cannot compromise the other.

### What are `vault.dat` and `.vault.key`?
When you run TRUSWAP, two files are created in your data directory:
- `vault.dat`: An encrypted file containing your private keys and records of active swaps.
- `.vault.key`: A 32-byte cryptographic secret key that unlocks `vault.dat`.

Think of `vault.dat` as a locked physical safe, and `.vault.key` as the physical key that opens it. If an attacker copies your `vault.dat` file, they cannot decrypt it or read your private keys without `.vault.key`.

---

## 2. Backup and Recovery Instructions

> [!CAUTION]
> **Write Down Your Backup Words on Paper**
> TRUSWAP is non-custodial. There is no customer support, no password reset, and no central database. If your device breaks and you do not have your backup words, your funds cannot be recovered.

### How to Back Up:
1. Open TRUSWAP at `http://127.0.0.1:8899`.
2. On the **TRU Vault Card** at the top of the dashboard, click **Export**:
   - The modal reveals your **24-word BIP-39 mnemonic phrase** and your **WIF Private Key**.
   - Write them down in order on paper.
3. On the **Base Vault Card**, click **Export**:
   - Click the **👁 Show** buttons to reveal your **12-word Base mnemonic phrase** and EVM private key.
   - Write down the 12 words on paper.
4. Store your paper backup in a secure, fireproof, and dry location.

### How to Restore Your Wallet:
- **File Transfer Method (Fastest)**:
  Copy both `vault.dat` and `.vault.key` from your old machine's `data/` folder into the `data/` folder on your new machine. When TRUSWAP starts, it will automatically decrypt and load your existing keys and active swap states.
- **Key Derivation from Seed**:
  You can also restore your accounts in any BIP-39 compatible wallet:
  - Base account: Import the 12 words into any EVM wallet (derivation path `m/44'/60'/0'/0/0`).
  - TRU account: Import the WIF private key into any TRU-compatible node or wallet.

---

## 3. Cryptographic Specifications (For Developers)

### 3.1 TRU Key Derivation
- **Entropy**: 256 bits (32 bytes) generated via the operating system's Cryptographically Secure Pseudorandom Number Generator (CSPRNG via `rand::rngs::OsRng`).
- **Curve**: SECP256k1 elliptic curve.
- **WIF Encoding**:
  - Byte `0x80` prepended to private key.
  - Byte `0x01` appended to indicate compressed public key.
  - 4-byte double-SHA256 checksum appended.
  - Encoded with Base58.
- **24-Word BIP-39**: Derived via `bip39::Mnemonic::from_entropy(priv_key)`.
- **Public Key**: 33-byte compressed coordinate point (prefixed with `0x02` for even Y or `0x03` for odd Y).
- **Address**:
  $$\text{PKH} = \text{RIPEMD-160}(\text{SHA-256}(\text{CompressedPubKey}))$$
  Version byte `0x41` (decimal 65, producing character `T`) is prepended, followed by a 4-byte double-SHA256 checksum, then Base58Check encoded.

### 3.2 Base EVM Key Derivation
- **Mnemonic**: 128-bit entropy mapped to 12 words via standard BIP-39.
- **Derivation Path**: `m/44'/60'/0'/0/0` via BIP-32 and BIP-44 standards.
- **Public Key**: 64-byte uncompressed coordinate point (excluding leading `0x04` prefix).
- **Address**:
  - Hash uncompressed point with `Keccak-256`.
  - Extract the final 20 bytes (160 bits).
  - Format as an EIP-55 mixed-case checksum string prefixed with `0x`.

---

## 4. Storage Encryption at Rest (`vault.dat`)

All keys and swap states are written to disk in a single compressed, authenticated binary file (`vault.dat`).

### Binary File Structure:

| Byte Offset | Field Name | Data Type | Description |
|---|---|---|---|
| `0..8` | Magic Identifier | `[u8; 8]` | ASCII string `TRUSWAP\x04` (Version 4 format) |
| `8..24` | Salt | `[u8; 16]` | Cryptographic salt generated per file write |
| `24..N-32` | Ciphertext | `Vec<u8>` | Deflate-compressed, XOR-encrypted binary payload |
| `N-32..N` | HMAC Tag | `[u8; 32]` | HMAC-SHA256 integrity tag over `(Salt \|\| Ciphertext)` |

### Encryption Pipeline:
1. **Serialization & Compression**:
   The `VaultWalletData` struct (containing keys, mnemonics, and active swap states) is serialized and compressed using `flate2::write::DeflateEncoder`. Compression reduces file size and removes repeating plaintext patterns.
2. **Counter-Mode Keystream (XOR)**:
   A deterministic keystream is generated using SHA-256:
   $$\text{Block}_i = \text{SHA-256}(\text{MasterKey} \parallel \text{Salt} \parallel \text{Counter}_{\text{LE32}})$$
   Each 32-byte chunk of compressed payload is XORed with its corresponding keystream block.
3. **HMAC-SHA256 Authentication**:
   A 32-byte HMAC-SHA256 signature is calculated across `(Salt || Ciphertext)` using the master key. When loading, the HMAC is verified first. If a single byte has been modified or corrupted, loading fails immediately before decryption.
4. **POSIX File Permissions (`0o600`)**:
   On Linux and macOS, both `vault.dat` and `.vault.key` are created with strict `0o600` permissions (read/write only by the process owner). Other user accounts on the machine cannot open or read the files.
5. **Atomic Write Replacement**:
   To prevent corrupt files during power outages, new data is written to a temporary file (`vault.dat.tmp`), flushed to disk with `sync_all()`, and atomically renamed to `vault.dat`.

---

## 5. In-Memory Security

- **Short-Lived Keys**: Private keys are held in memory only for the duration of transaction signing and dropped immediately after use.
- **Log Sanitization**: Private keys, mnemonics, and preimages are never printed to terminal logs, debug files, or error messages.
- **Loopback Binding**: The local web dashboard binds strictly to `127.0.0.1`. Other devices on your local Wi-Fi cannot connect to your wallet interface unless you explicitly run with `--dev`.
