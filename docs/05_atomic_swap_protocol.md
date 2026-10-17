# Chapter 5: Cryptographic Atomic Swap Protocol

This chapter explains how cross-chain atomic swaps work in TRUSWAP, starting with an intuitive analogy for beginners, followed by exact technical specifications, script opcodes, and smart contract interfaces for developers.

---

## 1. How Atomic Swaps Work (The Lockbox Analogy)

### What does "Atomic" mean?
In computer science, an operation is **atomic** if it is "all or nothing." Either the entire trade succeeds for both participants, or it fails and both participants keep their original funds. There is no intermediate state where one person gets the money and the other is left empty-handed.

### The Two Lockboxes with One Key:
Imagine Alice wants to trade 100 TRU for 50 USDC with Bob:

<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">HTLC PROTOCOL</span>
    <span class="diagram-heading">The Two Lockboxes with One Key</span>
  </div>

  <div class="diagram-grid-2">
    <div class="diagram-node node-tru">
      <div class="node-badge">🟡 TRU LOCKBOX (TRU NETWORK)</div>
      <div class="lockbox-content-badge">Contains: 100 TRU (Deposited by Alice)</div>
      <div class="lockbox-rules-title">Unlocks if:</div>
      <ul class="node-list">
        <li><strong>Bob</strong> provides the SECRET PREIMAGE, OR</li>
        <li><strong>48 hours</strong> pass (Alice claims full refund)</li>
      </ul>
    </div>

    <div class="diagram-node node-base">
      <div class="node-badge">🔵 BASE LOCKBOX (BASE NETWORK)</div>
      <div class="lockbox-content-badge">Contains: 50 USDC (Deposited by Bob)</div>
      <div class="lockbox-rules-title">Unlocks if:</div>
      <ul class="node-list">
        <li><strong>Alice</strong> provides the SECRET PREIMAGE, OR</li>
        <li><strong>24 hours</strong> pass (Bob claims full refund)</li>
      </ul>
    </div>
  </div>

  <div class="diagram-connector">
    <div class="connector-line"></div>
    <div class="connector-arrow">▲ &nbsp; Coupled by a Single Secret Preimage &nbsp; ▲</div>
  </div>

  <div class="diagram-node node-app text-center">
    <div class="node-badge">🔑 THE SAME SECRET KEY (PREIMAGE)</div>
    <div class="node-desc">Generated and held securely by Alice. Revealed publicly on Base when claiming USDC, automatically allowing Bob to claim TRU.</div>
  </div>
</div>

1. **Creating the Lock**:
   - Alice generates a secret password (called a **preimage**).
   - She hashes this password using a one-way mathematical function to create a digital lock (called **HASH160**). Anyone can see the lock, but nobody can guess the password.
2. **Locking Assets**:
   - Alice locks 100 TRU into a smart script on the TRU network.
   - Bob locks 50 USDC into a smart contract on the Base network using the **exact same digital lock**.
3. **The Swap**:
   - Alice unlocks Bob's USDC on Base by providing her secret password.
   - The moment she does this, the password is permanently recorded on the public Base blockchain.
   - Bob's TRUSWAP client sees the password on the Base blockchain, copies it, and submits it to unlock Alice's 100 TRU on the TRU network.
4. **Safety Timers (Refunds)**:
   - What if Bob never deposits his USDC? Alice waits for her timer to expire and takes her 100 TRU back.
   - What if Alice disappears after Bob deposits? Bob waits for his timer to expire and takes his 50 USDC back.

---

## 2. Cryptographic Fundamentals (HASH160)

The digital lock is created using **HASH160**:
$$\text{HASH160}(S) = \text{RIPEMD-160}(\text{SHA-256}(S))$$

Where:
- $S$ is a 32-byte secret preimage generated from a cryptographically secure random number generator (CSPRNG).
- $\text{HASH160}(S)$ produces a 20-byte digest.

**Why HASH160 instead of SHA-256?**
1. Standard SHA-256 digests are 32 bytes. HASH160 is 20 bytes.
2. The 12-byte reduction saves space in TRU transaction outputs and reduces calldata gas costs on the Base network, while maintaining equivalent collision resistance against known attacks.

---

## 3. Canonical 103-Byte TRU HTLC Script

On the TRU Proof-of-Work blockchain, the swap is locked in a Pay-to-Script-Hash (P2SH) script implemented in [`src/core/script.rs`](file:///home/user/git_test/projects/tru/repos/truswap/src/core/script.rs#L23-L52):

```text
OP_IF
    OP_HASH160 <20-byte hash160> OP_EQUALVERIFY
    <33-byte claim_pubkey> OP_CHECKSIG
OP_ELSE
    <4-byte timelock> OP_CHECKLOCKTIMEVERIFY OP_DROP
    <33-byte refund_pubkey> OP_CHECKSIG
OP_ENDIF
```

### Exact Byte Layout (103 Bytes Total):

| Opcode / Field | Hex Byte | Size | Function |
|---|---|---|---|
| `OP_IF` | `0x63` | 1 byte | Begin conditional branch |
| `OP_HASH160` | `0xa9` | 1 byte | Hash the input with SHA-256 followed by RIPEMD-160 |
| `PUSH_20` | `0x14` | 1 byte | Push 20 bytes of hash digest to the stack |
| `<hash160>` | `20 bytes` | 20 bytes | Target HASH160 digest |
| `OP_EQUALVERIFY` | `0x88` | 1 byte | Verify input hash matches target, abort if unequal |
| `PUSH_33` | `0x21` | 1 byte | Push 33 bytes of claim public key |
| `<claim_pubkey>` | `33 bytes` | 33 bytes | Recipient's SECP256k1 compressed public key |
| `OP_CHECKSIG` | `0xac` | 1 byte | Verify recipient's ECDSA signature |
| `OP_ELSE` | `0x67` | 1 byte | Begin alternate refund branch |
| `PUSH_4` | `0x04` | 1 byte | Push 4-byte little-endian Unix timelock |
| `<timelock>` | `4 bytes` | 4 bytes | Unix timestamp after which refund is valid |
| `OP_CHECKLOCKTIMEVERIFY` | `0xb1` | 1 byte | Verify transaction `nLockTime >= timelock` |
| `OP_DROP` | `0x75` | 1 byte | Discard timelock value from the stack |
| `PUSH_33` | `0x21` | 1 byte | Push 33 bytes of refund public key |
| `<refund_pubkey>` | `33 bytes` | 33 bytes | Depositor's SECP256k1 compressed public key |
| `OP_CHECKSIG` | `0xac` | 1 byte | Verify depositor's ECDSA signature |
| `OP_ENDIF` | `0x68` | 1 byte | End conditional branch |

### Spending Paths (Witness Stack / ScriptSig):
- **To Claim**: `[<claim_signature>, <secret_preimage>, 1, <redeem_script>]`
- **To Refund**: `[<refund_signature>, 0, <redeem_script>]` (requiring `tx.lock_time >= timelock`)

---

## 4. Base Layer-2 Smart Contract (`BaseHTLC.sol`)

On Base, the swap is governed by the deployed `BaseHTLC` contract:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IBaseHTLC {
    // Selector: 0xefafa23f
    function createSwap(
        address beneficiary,
        address token,
        uint256 amount,
        bytes20 hash160,
        uint256 timelock,
        bytes32 salt
    ) external returns (bytes32 swapId);

    // Selector: 0xf0331024
    function claimSwap(bytes32 swapId, bytes calldata secret) external;

    // Selector: 0x7249fbb6
    function refundSwap(bytes32 swapId) external;
}
```

### Verification Logic:
1. **Claiming**:
   When `claimSwap(swapId, secret)` is called, the contract verifies:
   $$\text{ripemd160}(\text{abi.encodePacked}(\text{sha256}(secret))) == \text{hash160}$$
   If the check passes, the contract transfers the tokens to the beneficiary.
2. **Deterministic Swap ID**:
   $$\text{swapId} = \text{keccak256}(\text{sender}, \text{beneficiary}, \text{token}, \text{amount}, \text{hash160}, \text{timelock}, \text{block.chainid}, \text{salt})$$
3. **Public Secret Extraction**:
   When funds are claimed on Base, the contract emits an event:
   ```solidity
   event SwapClaimed(bytes32 indexed swapId, bytes secret, address claimer);
   ```
   Topic 0: `0xd14278a0c2b5ef6c1c6418dba05a4ebd99536912416835fde879006db64b8d06`  
   The counterparty's client monitors this event, extracts `secret`, and uses it to claim the TRU on the Proof-of-Work network.

---

## 5. Timelock Asymmetry ($T_1 > T_2$)

To prevent a race condition where one party claims their funds while simultaneously refunding their own deposit, the timelocks must have a staggered expiration window:

- **Maker Timelock ($T_1$)**: E.g., **48 Hours**
- **Taker Timelock ($T_2$)**: E.g., **24 Hours**

<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">SECURITY TIMELINE</span>
    <span class="diagram-heading">Timelock Asymmetry Window (T1 > T2)</span>
  </div>
  <div class="flow-steps">
    <div class="flow-step">
      <div class="flow-step-num">0h</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Both Locks Created (Assets Escrowed)</div>
        <div class="flow-step-desc">Alice escrows 100 TRU (48h lock), Bob escrows 50 USDC (24h lock) with identical hash lock H.</div>
      </div>
    </div>
    <div class="flow-connector">▼ 24-Hour Safety Buffer</div>
    <div class="flow-step node-base">
      <div class="flow-step-num">24h</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Taker Timelock Expires (T2)</div>
        <div class="flow-step-desc">If Alice never claimed USDC, Bob can safely refund his USDC. Alice cannot claim after T2.</div>
      </div>
    </div>
    <div class="flow-connector">▼ Additional 24-Hour Window for Bob to Claim</div>
    <div class="flow-step node-tru">
      <div class="flow-step-num">48h</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Maker Timelock Expires (T1)</div>
        <div class="flow-step-desc">If Bob never claimed TRU after Alice revealed S, Alice can refund her TRU.</div>
      </div>
    </div>
  </div>
</div>

Because $T_1$ expires 24 hours after $T_2$, the Maker must claim before $T_2$ expires. The moment the Maker claims, the secret is revealed, giving the Taker a full 24-hour safety window to claim on the other chain before $T_1$ expires.

---

## 6. End-to-End Execution Sequence

<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">PROTOCOL SEQUENCE</span>
    <span class="diagram-heading">Step-by-Step Atomic Swap Flow</span>
  </div>
  <div class="flow-steps">
    <div class="flow-step node-tru">
      <div class="flow-step-num">1</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Alice Generates Secret Preimage (S)</div>
        <div class="flow-step-desc">Calculates cryptographic lock <code>H = HASH160(S)</code> locally on device. S remains secret.</div>
      </div>
    </div>
    <div class="flow-connector">▼ Broadcast to TRU PoW Chain</div>
    <div class="flow-step node-tru">
      <div class="flow-step-num">2</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Alice Locks TRU in 103-Byte HTLC</div>
        <div class="flow-step-desc">Locks native TRU coins under hash lock H with 48-hour expiration ($T_1$).</div>
      </div>
    </div>
    <div class="flow-connector">▼ 6 Block Confirmations (~15 min)</div>
    <div class="flow-step">
      <div class="flow-step-num">3</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Confirmation Guard Verification</div>
        <div class="flow-step-desc">Both Alice and Bob verify 6 consecutive block confirmations to guarantee zero reorg risk.</div>
      </div>
    </div>
    <div class="flow-connector">▼ EVM Transaction on Base L2</div>
    <div class="flow-step node-base">
      <div class="flow-step-num">4</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Bob Locks USDC in BaseHTLC</div>
        <div class="flow-step-desc">Calls <code>createSwap(H, 24h)</code> on Base smart contract escrowing USDC ($T_2$).</div>
      </div>
    </div>
    <div class="flow-connector">▼ Alice Reveals Secret Preimage</div>
    <div class="flow-step node-base">
      <div class="flow-step-num">5</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Alice Claims USDC on Base</div>
        <div class="flow-step-desc">Calls <code>claimSwap(swapId, S)</code> submitting secret S on-chain. USDC transfers to Alice; S becomes public on Base!</div>
      </div>
    </div>
    <div class="flow-connector">▼ Bob Reads S from Base Blockchain</div>
    <div class="flow-step node-tru">
      <div class="flow-step-num">6</div>
      <div class="flow-step-content">
        <div class="flow-step-title">Bob Claims TRU on PoW Chain</div>
        <div class="flow-step-desc">Extracts preimage S from Base event logs and unlocks Alice's TRU coins into his wallet. Both sides complete 100%!</div>
      </div>
    </div>
  </div>
</div>

---

## 7. Safety Guards and Automatic Refunds

1. **Confirmation Guard (Reorg Protection)**:
   - On the TRU network, the client waits for **6 block confirmations** before Bob locks funds on Base. This ensures an orphaned block or Proof-of-Work reorg cannot reverse the TRU deposit.
   - On Base, the client monitors confirmed/finalized RPC receipts.
2. **Automatic Background Refund**:
   - If the counterparty disconnects, loses internet, or refuses to complete the swap, TRUSWAP's background daemon automatically monitors block timestamps.
   - As soon as the timelock expires, TRUSWAP signs and broadcasts the refund transaction. Your full deposit returns to your wallet automatically.

---

## 8. What You See in the TRUSWAP Interface During a Swap

The cryptographic protocol translates into intuitive visual steps in the TRUSWAP trading terminal:

1. **Take Order Modal**:
   When you click **[Swap]** on any open offer in the Orderbook, this modal displays:
   - **YOU SEND** and **YOU RECEIVE**: The exact amounts of TRU and USDC being traded.
   - **Maker Beneficiary**: The counterparty's receiving address.
   - **Your Receiving Vault**: Your local address where coins will arrive.
   - **Cryptographic Hashlock**: The 20-byte HASH160 digest that ties both blockchain contracts together.
   - **Estimated Rate**: The price per 1 TRU.
   - Clicking **Confirm Swap** initiates the atomic protocol.

2. **The In-Progress Tab (`🔄 In-Progress`)**:
   Once accepted, the swap moves to the In-Progress tab in the right panel. It shows a live status badge:
   - `⏳ Awaiting USDC Deposit`: TRU is locked; waiting for the buyer's BaseHTLC transaction.
   - `⏳ Confirming (X/6 Blocks)`: The TRU deposit is waiting for network depth to eliminate reorg risk.
   - `⚡ Claiming`: Secret preimage revealed; claiming funds on the destination chain.
   - `✅ Completed`: Trade successfully settled on both chains.

3. **Automation Audit Modal**:
   Clicking on any active swap row opens the **Autonomous Engine Process Audit Modal**. This provides real-time transparency into the background state machine:
   - Displays real-time console log telemetry.
   - Shows active HASH160 digest and trade direction.
   - Shows live confirmation counters and on-chain transaction hashes.
   - Includes **Copy Log** and **Auto-Scroll** controls for auditing.

4. **The History Tab (`📜 History`)**:
   When a swap completes, it is recorded under the History tab. You can click on any TRU transaction ID to inspect the raw inputs, outputs, and HTLC scripts in the **TRU Transaction Details Modal**, or click the Base link to verify settlement on BaseScan.
