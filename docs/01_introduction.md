# Chapter 1: Introduction & Concepts

TRUSWAP is an application that allows two people to trade cryptocurrency directly with each other across different blockchains. It connects **Tokenized Real Utility ($TRU)**, a Proof-of-Work coin, with **USD Coin ($USDC)** on Base, an Ethereum Layer-2 network.

---

## The Problem: Why Traditional Exchanges and Bridges Are Risky

When you want to trade between two different blockchains, you usually have three options — and all three carry significant risks:

1. **Centralized Exchanges (CEXs)**:
   - You deposit your coins into the exchange's wallet.
   - While your coins are there, you do not own them — the exchange does ("not your keys, not your coins").
   - If the exchange goes bankrupt, gets hacked, or freezes withdrawals, your funds can disappear.
2. **Wrapped Tokens (like wBTC or wTRU)**:
   - A custodian locks real coins on one chain and issues "wrapped" IOU tokens on another chain.
   - If the custodian is dishonest or their reserve wallet is compromised, the wrapped token loses all its backing and becomes worthless.
3. **Cross-Chain Bridges**:
   - Bridges use multi-signature wallets or smart contracts to move value.
   - Over the years, bridge exploits have been responsible for billions of dollars in losses due to smart contract bugs and stolen signing keys.

---

## The Solution: How Atomic Swaps Work

In computer science, an **atomic** operation is an all-or-nothing event. It either completes 100%, or it gets cancelled completely. There is never an in-between state.

An **Atomic Swap** allows you and a counterparty to trade across two separate blockchains without an escrow agent or middleman:

- **If everything goes right**: You get the counterparty's coins, and they get yours at the exact same moment.
- **If anything goes wrong**: Either person disconnects, changes their mind, or tries to cheat — the built-in timer expires, and both people get their original funds back automatically.
<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">ATOMIC SETTLEMENT</span>
    <span class="diagram-heading">All-or-Nothing Swap Outcomes</span>
  </div>

  <div class="outcome-grid">
    <div class="outcome-card success">
      <div class="outcome-header">
        <span class="outcome-icon">✅</span>
        <span class="outcome-title">Trade Succeeds</span>
      </div>
      <div class="outcome-condition">Secret preimage revealed on Base within time window</div>
      <ul class="outcome-points">
        <li><strong>Alice</strong> gets USDC on Base</li>
        <li><strong>Bob</strong> gets TRU on TRU blockchain</li>
        <li>Atomic settlement completed without intermediaries</li>
      </ul>
    </div>

    <div class="outcome-divider-badge">OR</div>

    <div class="outcome-card refund">
      <div class="outcome-header">
        <span class="outcome-icon">⏱️</span>
        <span class="outcome-title">Trade Cancels</span>
      </div>
      <div class="outcome-condition">Counterparty disappears or timer expires</div>
      <ul class="outcome-points">
        <li><strong>Alice</strong> claims full refund of her TRU</li>
        <li><strong>Bob</strong> claims full refund of his USDC</li>
        <li>Zero funds lost — guaranteed by cryptography</li>
      </ul>
    </div>
  </div>
</div>

---

## The Two Blockchains TRUSWAP Connects

TRUSWAP bridges two very different blockchain technologies:

### 1. Tokenized Real Utility ($TRU)
- **Type**: Proof-of-Work (PoW) blockchain, similar to Bitcoin.
- **Accounting Model**: UTXO (Unspent Transaction Output) — coins exist as discrete outputs from previous transactions.
- **Scripting Language**: Bitcoin-compatible Forth-like stack scripts. Supports hash checks (`OP_HASH160`) and time checks (`OP_CHECKLOCKTIMEVERIFY`).
- **P2P Network**: Communicates directly with TRU network nodes over TCP port `21833`.

### 2. Base (Ethereum Layer-2)
- **Type**: Optimistic Rollup that settles transactions on Ethereum Layer-1.
- **Accounting Model**: Account balance model with smart contracts (EVM).
- **Speed & Cost**: 2-second block times and sub-cent gas fees.
- **Stablecoin**: Official Circle USDC (`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`), pegged 1:1 to the US Dollar.

---

## Who Uses TRUSWAP?

1. **Proof-of-Work Miners**:
   - You mine TRU using mining pools or independent nodes and receive native block rewards.
   - You want to convert mined TRU into stable USD Coin (USDC) on Base without giving your coins to a centralized exchange, paying high withdrawal fees, or undergoing KYC identity checks.
2. **Crypto Traders & Buyers**:
   - You hold USDC on Base (Coinbase, Uniswap, or MetaMask) and want to purchase native Proof-of-Work TRU.
   - You receive real Layer-1 coins directly into your own private keys, rather than an IOU or wrapped synthetic token.

---

## How TRUSWAP Is Built

- **Single Self-Contained Binary**: TRUSWAP contains its own TRU SPV light client, Base transaction signer, orderbook communicator, and local web dashboard.
- **Dual-Vault Key Management**: Generates and manages both your TRU keys (24-word phrase or WIF) and your Base keys (12-word phrase) locally in an encrypted file (`vault.dat`).
- **No Java on Android**: On Android, TRUSWAP runs as a native Linux binary (`aarch64`), accessing the camera and network directly through system C libraries for faster startup and lower battery usage.
- **Unified Terminal Web Interface**: When launched, it opens a local web dashboard (`http://127.0.0.1:8899`) with live balance cards, limit order form, real-time spot orderbook, and an integrated blockchain explorer (`/explorer`).
- **Fully Transparent**: You can inspect active swaps, raw transaction scripts, and block headers in the built-in blockchain explorer at any time.
