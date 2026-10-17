# Chapter 6: TRU Blockchain Engine & SPV Synchronization

This chapter explains how TRUSWAP connects to the Tokenized Real Utility ($TRU) blockchain, synchronizes transactions, and verifies Proof-of-Work without relying on third-party block explorers.

---

## 1. Blockchain Fundamentals (For Beginners)

### What is a Light Client (SPV)?
Traditional blockchain nodes require downloading hundreds of gigabytes of historical data, which can take days and fill up your hard drive.

TRUSWAP embeds a **Simplified Payment Verification (SPV) client** directly into its core:
- **Direct Peer-to-Peer**: Connects directly to TRU network nodes over standard TCP.
- **Header-First Verification**: Downloads only block headers and recent blocks, verifying the mathematical Proof-of-Work locally on your device.
- **No Third-Party Trust**: It does not ask a centralized server or website what your balance is. It checks the cryptographic proofs directly.
- **Fast Startup**: Synchronizes within seconds and stores recent blocks in a compact cache file (`tru.dat`).

### What is a UTXO?
Unlike traditional bank accounts or Ethereum where your balance is a single number in a ledger, TRU uses the **UTXO (Unspent Transaction Output)** model (the same model used by Bitcoin):

- Think of UTXOs like physical paper bills in your wallet.
- If your total balance is 100 TRU, you might hold two "bills": one 60 TRU bill and one 40 TRU bill.
- When you spend 25 TRU, TRUSWAP takes the 40 TRU bill, sends 25 TRU to the recipient, and sends 15 TRU back to your wallet as "change" (minus a tiny mining fee).
- TRUSWAP automatically tracks your bills, selects the right inputs, and manages change outputs automatically.

---

## 2. P2P Network Architecture

<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">P2P ARCHITECTURE</span>
    <span class="diagram-heading">TRU SPV Engine & Sync Pipeline</span>
  </div>

  <div class="flow-steps">
    <div class="flow-step node-tru">
      <div class="flow-step-num">1</div>
      <div class="flow-step-content">
        <div class="flow-step-title">TRU P2P Network (Distributed Nodes)</div>
        <div class="flow-step-desc">Peer nodes listening on TCP Port 21833 (Magic: <code>0xEC7C8FB9</code>)</div>
      </div>
    </div>

    <div class="flow-connector">▼ TCP Port 21833 (tru-mainnet)</div>

    <div class="flow-step">
      <div class="flow-step-num">2</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Native Rust P2P Client</div>
        <div class="flow-step-desc">44-Byte Binary framing, Protobuf payloads, Version/VerAck handshake, Block/Tx/Inv synchronization</div>
      </div>
    </div>

    <div class="flow-connector">▼ Raw Headers & Transactions</div>

    <div class="flow-step">
      <div class="flow-step-num">3</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Consensus & UTXO Validation</div>
        <div class="flow-step-desc">SHA256d + 21E8 Target verification, in-memory UTXO tracking, HTLC lock output scanning</div>
      </div>
    </div>

    <div class="flow-connector">▼ Validated Chain State</div>

    <div class="flow-step">
      <div class="flow-step-num">4</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Gzip-Compressed Block Cache (tru.dat)</div>
        <div class="flow-step-desc">Fast local persistent storage for instant resume across restarts</div>
      </div>
    </div>
  </div>
</div>

---

## 3. Binary Wire Protocol Framing (`src/tru_p2p/wire.rs`)

Communications with TRU network nodes use a compact 44-byte binary frame header followed by a Protocol Buffers serialized payload:

### Header Layout (44 Bytes Fixed):

| Offset | Field Name | Data Type | Value / Description |
|---|---|---|---|
| `0..4` | `magic` | `[u8; 4]` | Network magic bytes: `0xec, 0x7c, 0x8f, 0xb9` (`MAINNET_P2P_MAGIC`) |
| `4..8` | `type` | `i32` (LE) | Little-endian signed integer identifying message opcode |
| `8..12` | `length` | `u32` (LE) | Little-endian payload size in bytes (up to 64 MB maximum) |
| `12..44` | `checksum` | `[u8; 32]` | Full `SHA-256` hash of the Protobuf payload |
| `44..N` | `payload` | Bytes | Serialized Protocol Buffers message |

### Message Types:

| Type ID | Message Name | Protobuf Structure | Purpose |
|---|---|---|---|
| `0` | `Version` | `Version` | Handshake greeting: protocol version, services, timestamp, start_height |
| `1` | `VerAck` | `VerAck` | Handshake acknowledgment confirming connection parameters |
| `2` | `Addr` | `Addr` | Peer exchange sharing IP addresses of active nodes |
| `3` | `Inv` | `Inv` | Inventory announcement (hashes of newly mined blocks or transactions) |
| `4` | `GetData` | `GetData` | Request to download specific block or transaction data by hash |
| `5` | `Block` | `BlockProto` | Serialized block: hash, header, transactions, chain work, height |
| `6` | `Tx` | `TxProto` | Raw transaction broadcast (transaction ID + serialized bytes) |
| `7` | `Ping` | `Ping` | Liveness check containing a 64-bit random nonce |
| `8` | `Pong` | `Pong` | Liveness reply echoing the ping nonce |
| `9` | `Chat` | `ChatMessage` | Optional peer communication message |
| `10` | `Height` | `HeightMessage` | Announcement of the node's current tip height |
| `11` | `GetBlock` | `GetBlockRequest` | Request for a specific block by height |
| `12` | `GetHeight` | `GetHeightRequest` | Query for the remote peer's highest block |
| `13` | `BlockNotFound` | `BlockNotFound` | Returned when a requested block is unavailable |

---

## 4. Consensus & Proof-of-Work Verification

TRUSWAP validates incoming blocks to protect users from forged transactions:
1. **Header Hash Validation**:
   Computes double-SHA256 (`SHA256d`) over the block header.
2. **Target Difficulty Verification**:
   Validates that the block hash satisfies the network target difficulty, checking for the canonical `21E8` pattern.
3. **Tail-100 Sliding Window**:
   TRUSWAP maintains continuous validation across the latest 100 blocks, detecting chain reorganizations and orphan blocks in real-time.
4. **Compressed Block Storage (`tru.dat`)**:
   Blocks relevant to the wallet are saved in a Gzip-compressed binary file (`tru.dat`), preserving network synchronization state across application restarts.

---

## 5. UTXO Memory Tracker & Transaction Builder

### UTXO Scanner:
- Matches incoming transaction outputs against the user's P2PKH script (`OP_DUP OP_HASH160 <pkh> OP_EQUALVERIFY OP_CHECKSIG`).
- Tracks confirmation depth from 0-confirmation mempool status up to confirmed blocks.
- Identifies and watches 103-byte HTLC script outputs for active swaps.
- **UtxoGuard**: Feeds verified unspent output balances to the orderbook relay, preventing users from creating sell orders without actual coins.

### Transaction Construction:
1. **Coin Selection**: Selects optimal unspent outputs to cover the required transfer amount plus mining fee.
2. **Change Output**: Calculates remainder and sends it back to the user's change address.
3. **ECDSA Signing**: Signs each input hash over SECP256k1 using the vault's private key.
4. **Broadcast**: Encapsulates the signed raw transaction in a `TxProto` message and broadcasts it across connected P2P peers.

---

## 6. How the TRU Engine Appears in the User Interface

The underlying P2P network, consensus verification, and UTXO engine are exposed directly in the interface:

1. **Header Badges & Shortcuts**:
   - **`● Online (N peers)`**: Updates in real time as peer connections are maintained over TCP port 21833.
   - **`🔍 TRU #<Height>`**: Shows the current verified block height. Clicking this badge opens the **TRU Block Explorer** (`/explorer`).
   - **`Mempool: N txs`**: Shows zero-confirmation transactions observed in the P2P broadcast stream. Clicking filters the explorer to show pending mempool items.

2. **UTXO Ledger Modal**:
   Clicking **UTXO Ledger** on the TRU Vault Card opens a dedicated ledger table:
   - **TXID & Vout**: Identifies each discrete unspent coin.
   - **Value**: Formatted in both standard TRU and 8-decimal raw atoms.
   - **Confirmations**: Real-time confirmation depth from 0-conf (in mempool) to confirmed blocks.
   - **Lock Status**: Highlights outputs locked in active HTLC scripts, preventing accidental double-spending.

3. **Built-in TRU Block Explorer (`/explorer`)**:
   TRUSWAP bundles a self-hosted block explorer that queries your local SPV daemon without calling third-party websites:
   - **Search Bar**: Search directly by block height (e.g., `#46936`), block hash, TXID, or wallet address (`T...`).
   - **Network Cards**: Block Height Tip, Last Block Time, Mempool Volume, and Connected Peers.
   - **Latest Blocks**: Table of mined blocks with hashes, timestamps, transaction counts, and block rewards.
   - **Latest Transactions**: Real-time transaction feed with status badges (`MEMPOOL` or confirmed block height).

4. **TRU Transaction Details Modal**:
   Clicking any transaction ID in the explorer or trade history opens a dialog displaying input outpoints, destination scripts, satoshis, and HTLC opcodes.
