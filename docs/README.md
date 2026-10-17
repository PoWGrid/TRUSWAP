# The TRUSWAP Book

A complete guide to **TRUSWAP** — an application for trading **Tokenized Real Utility ($TRU)** and **Base USD Coin ($USDC)** directly peer-to-peer without centralized exchanges or custodial bridges.

---

## What is TRUSWAP?

TRUSWAP is an open-source client for desktop and Android written in Rust. It lets you trade native Proof-of-Work cryptocurrency ($TRU) for stablecoins on Ethereum Layer-2 (Base USDC) directly with another user.

### Key Differences from Centralized Exchanges:
- **No Account Sign-up**: No email, passwords, KYC identity verification, or deposit approvals.
- **You Keep Your Keys**: Your funds never sit in an exchange wallet. Your private keys stay on your local device.
- **No Wrapped Tokens**: You trade real native TRU and real Base USDC. No "wTRU", bridges, or IOUs that can be hacked or frozen.
- **Automatic Refund Protection**: If the trade does not finish, both parties get their original coins back automatically when the timer runs out.
<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">ARCHITECTURE</span>
    <span class="diagram-heading">HOW TRUSWAP WORKS</span>
  </div>
  <div class="diagram-grid-2">
    <div class="diagram-node node-tru">
      <div class="node-badge">🟡 PROOF-OF-WORK</div>
      <div class="node-title">TRU Network</div>
      <ul class="node-list">
        <li>Native Proof-of-Work Coins</li>
        <li>103-byte HTLC Script Engine</li>
        <li>Direct P2P SPV Light Client (TCP 21833)</li>
      </ul>
    </div>
    <div class="diagram-node node-base">
      <div class="node-badge">🔵 ETHEREUM L2</div>
      <div class="node-title">Base Network</div>
      <ul class="node-list">
        <li>Official Circle USDC Stablecoins</li>
        <li>BaseHTLC Smart Contract</li>
        <li>Headless EVM Signer & RPC Watcher</li>
      </ul>
    </div>
  </div>
  <div class="diagram-connector">
    <div class="connector-line"></div>
    <div class="connector-arrow">▼</div>
  </div>
  <div class="diagram-node node-app">
    <div class="node-badge">⚡ UNIFIED RUNNER</div>
    <div class="node-title">TRUSWAP APP (Desktop & Android)</div>
    <div class="node-tags-grid">
      <div class="feature-tag">Dual-Vault (Keys & UTXO)</div>
      <div class="feature-tag">Exchange Cockpit & Orderbook</div>
      <div class="feature-tag">Autonomous Swap Engine</div>
      <div class="feature-tag">Direct TRU SPV (TCP 21833)</div>
      <div class="feature-tag">Base EVM Signer & Watcher</div>
      <div class="feature-tag">P2P Orderbook Relay Mesh</div>
      <div class="feature-tag">Native NDK Camera QR Scanner</div>
      <div class="feature-tag">Built-in Explorer & Book Reader</div>
    </div>
  </div>
</div>

---

## How an Atomic Swap Works (In 30 Seconds)

Imagine you want to trade 100 TRU for 50 USDC with someone across the internet:

1. **Locking Assets**:
   - You put 100 TRU into a cryptographic digital lockbox on the TRU network.
   - The buyer puts 50 USDC into a matching digital lockbox on the Base network.
2. **The Shared Key**:
   - Both lockboxes are keyed to the exact same cryptographic secret (`preimage`).
3. **Claiming**:
   - To take the 50 USDC on Base, you reveal this secret key to the smart contract.
   - The moment you claim your USDC, the secret key is published on the public blockchain.
   - The buyer reads that key and immediately uses it to unlock your 100 TRU on the TRU network.
4. **Safety Timer (Refund)**:
   - If the buyer never shows up or you change your mind before locking, the lockbox timers expire and everyone takes their original coins back. Nobody can steal your money.

---

## Book Chapters

This book is organized so that beginners can learn how to install and use the app, while developers and auditors can inspect the code and cryptographic rules:

1. [**Chapter 1: Introduction & Concepts**](01_introduction.md)  
   Why peer-to-peer swaps exist, how they differ from centralized exchanges and bridge hacks, and what TRU and Base are.

2. [**Chapter 2: Desktop Quick Start**](02_quickstart_desktop.md)  
   How to download or compile TRUSWAP on Linux, macOS, or Windows, start the background service, and open the web dashboard.

3. [**Chapter 3: Android Setup**](03_quickstart_android.md)  
   How to install `truswap.apk` on Android, grant camera permission for QR scanning, and use the mobile interface.

4. [**Chapter 4: Vault & Key Management**](04_vault_and_security.md)  
   How your TRU keys (24 words / WIF) and Base keys (12 words / BIP-44) are generated, stored in `vault.dat`, and encrypted on disk.

5. [**Chapter 5: Atomic Swap Protocol**](05_atomic_swap_protocol.md)  
   The step-by-step swap mechanics, the canonical 103-byte TRU HTLC script, BaseHTLC contract calls, and timer math ($T_1 > T_2$).

6. [**Chapter 6: TRU Network Engine**](06_tru_blockchain_engine.md)  
   How TRUSWAP connects to TRU nodes over TCP port 21833, verifies block Proof-of-Work, caches headers in `tru.dat`, and tracks unspent coins (UTXOs).

7. [**Chapter 7: Base Layer-2 Engine**](07_base_layer2_engine.md)  
   How the app connects to Base JSON-RPC, signs transactions with EIP-155 replay protection, approves USDC, and calls the `BaseHTLC` contract.

8. [**Chapter 8: P2P Orderbook Mesh**](08_p2p_orderbook_mesh.md)  
   How buy and sell orders are signed, broadcast across WebSocket relays in real time, and validated against fake spam orders.

9. [**Chapter 9: Android NDK Camera Scanner**](09_mobile_ndk_hardware.md)  
   How the camera QR scanner runs directly on device hardware using Android C headers without Java overhead or cloud APIs.

10. [**Chapter 10: Security & Threat Model**](10_security_model_and_threat_analysis.md)  
    How TRUSWAP protects against blockchain reorganizations, malicious dropouts, timing attacks, and rogue computer processes.

11. [**Chapter 11: Troubleshooting & Settings**](11_troubleshooting_and_faq.md)  
    Command-line flags, environment variables, gas requirements, and answers to common problems.

---

## The Three Core Rules

1. **You Hold Your Keys**: Keys are stored on your machine. No remote server ever sees them.
2. **Automatic Refunds on Timeout**: If a trade stalls or counterparty vanishes, your deposit returns to you.
3. **Open and Verifiable**: Every byte of logic is inspectable in the open-source repository and verifiable on public explorers.
