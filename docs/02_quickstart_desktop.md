# Chapter 2: Desktop Quick Start Guide

This guide walks you through downloading, compiling, launching, and using TRUSWAP on Linux, macOS, or Windows.

---

## 1. System Requirements

- **Operating System**: Linux (Ubuntu 20.04+, Debian 11+, Arch, Fedora), macOS (Apple Silicon or Intel), or Windows 10/11 (Native `truswap.exe` or WSL2).
- **CPU**: 64-bit `x86_64` or `aarch64`.
- **Memory**: 512 MB RAM minimum (1 GB recommended).
- **Disk Space**: ~100 MB for the binary, local header cache (`tru.dat`), and encrypted vault (`vault.dat`).
- **Network Access**: Outbound TCP port `21833` for TRU P2P network and HTTPS port `443` for Base RPC endpoints.

---

## 2. Installation & Compilation

### Option A: Running a Precompiled Binary

#### On Linux & macOS:
```bash
# Make binary executable
chmod +x truswap

# Run on Base Mainnet (default)
./truswap

# Or run on Base Sepolia testnet
./truswap --testnet
```

#### On Windows (PowerShell or Command Prompt):
```powershell
# Run on Base Mainnet (default)
.\truswap.exe

# Or run on Base Sepolia testnet
.\truswap.exe --testnet
```

---

### Option B: Building from Source

Ensure Rust toolchain is installed (`rustc >= 1.75`):

#### 1. Clone the repository
```bash
git clone https://github.com/powgrid/truswap.git
cd truswap
```

#### 2. Build release binary

- **On Linux / macOS / Windows WSL2**:
  ```bash
  cargo build --release
  # Executable generated at: ./target/release/truswap
  ```

- **On Windows Native (PowerShell / Command Prompt)**:
  Requires Microsoft C++ Build Tools (Visual Studio 2022 C++ build tools):
  ```powershell
  cargo build --release
  # Executable generated at: .\target\release\truswap.exe
  ```

- **Cross-compiling for Windows from Linux**:
  You can cross-compile a native standalone Windows `.exe` directly from Linux without a Windows machine:
  ```bash
  cargo zigbuild --target x86_64-pc-windows-gnu --release
  # Executable generated at: ./target/x86_64-pc-windows-gnu/release/truswap.exe
  ```

---

## 3. Configuration & Environment Variables

TRUSWAP runs with working defaults out of the box. To customize ports, network seeds, or RPC endpoints, you can place a `.env` file in the working directory or pass command-line arguments:

```ini
# Network Mode (0 = Mainnet [default], 1 = Testnet)
TRUSWAP_TESTNET=0

# Local Web Interface Bind Address (default: 127.0.0.1:8899)
# Use TRUSWAP_DEV=1 to bind to 0.0.0.0:8899 for local network access
BIND_ADDR=127.0.0.1:8899

# Base RPC Endpoints
BASE_RPC_URL=https://mainnet.base.org
BASE_WSS_URL=wss://mainnet.base.org

# Smart Contracts (Base Mainnet Defaults)
BASE_HTLC_CONTRACT=0x812b611208297eb7A2b153BECA8b2AF47C7614DF
BASE_USDC_CONTRACT=0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913

# TRU P2P Seed Nodes (IP:Port)
TRU_SEEDS=137.184.68.43:21833,193.181.216.10:21833

# Storage Directory
DATA_DIR=./data
```

---

## 4. Starting the Daemon

Run the executable from your terminal:

```bash
./target/release/truswap
```

The daemon initializes the encrypted vault, connects to peer nodes, and launches the local web server:

```text
[INFO] TRUSWAP starting up...
[INFO] Vault initialized at ./data/vault.dat
[INFO] Connecting to TRU P2P network (21833)...
[INFO] Connecting to Base RPC (https://mainnet.base.org)...
[INFO] Web UI listening on http://127.0.0.1:8899
```

Open your browser and navigate to:
```
http://127.0.0.1:8899
```

---

## 5. User Interface Tour (What You See on Screen)

When you open `http://127.0.0.1:8899`, TRUSWAP presents a single unified terminal interface organized into distinct operational areas:

<div class="cyber-diagram-box ui-tour-diagram">
  <div class="diagram-title-bar">
    <span class="diagram-tag">LAYOUT BLUEPRINT</span>
    <span class="diagram-heading">TRUSWAP Terminal Cockpit Organization</span>
  </div>

  <div class="ui-mockup-frame">
    <div class="ui-mockup-top">
      <span>TRUSWAP v1.0.0 • Atomic P2P Market</span>
      <span>● Online (3 peers) &nbsp;|&nbsp; 🔍 TRU #46,936 &nbsp;|&nbsp; Mempool: 1</span>
    </div>

    <div class="ui-mockup-vaults">
      <div class="mockup-card tru-card">
        <div class="card-title">🟡 TRU VAULT</div>
        <div class="card-detail">Balance: <strong>37.59838566 TRU</strong> • Address: TRJQ...94Gm1</div>
        <div class="card-pills">
          <span class="pill">[Export]</span>
          <span class="pill">[Copy]</span>
          <span class="pill">[Deposit]</span>
          <span class="pill">[Withdraw]</span>
          <span class="pill">🔒 Local Encrypted</span>
        </div>
      </div>

      <div class="mockup-card base-card">
        <div class="card-title">🔵 BASE VAULT</div>
        <div class="card-detail">Balance: <strong>0.00 USDC</strong> • ⛽ Gas: 0.0098 ETH • 0xfb...cA83</div>
        <div class="card-pills">
          <span class="pill">[Export]</span>
          <span class="pill">[Copy]</span>
          <span class="pill">[Deposit]</span>
          <span class="pill">[Withdraw]</span>
          <span class="pill">Base Explorer ↗</span>
        </div>
      </div>
    </div>

    <div class="ui-mockup-section">
      <div class="section-title">⚡ SPOT LIMIT ORDER</div>
      <div class="mockup-form-row">
        <span>[Buy TRU] [Sell TRU]</span>
        <span>• Available: 0.00 USDC</span>
        <span>• Price: [1.0000] USDC</span>
        <span>• Amount: [0.0000] TRU (25% | 50% | 75% | 100%)</span>
        <span>• Button: [Enter Amount & Price]</span>
      </div>
    </div>

    <div class="ui-mockup-section">
      <div class="section-title">📊 TRU / USDC SPOT (Orderbook)</div>
      <div class="mockup-table-preview">
        <div class="mockup-row">
          <span>PRICE: $1.0000</span>
          <span>AMOUNT: 29.0000 TRU</span>
          <span>TOTAL: $29.00 USDC</span>
          <span class="action-btn">[Swap]</span>
        </div>
      </div>
    </div>
  </div>
</div>

### Top Navigation & Status Bar
- **Network Status Badge**: Shows `● Online` with current count of connected TRU P2P daemon peers.
- **Blockchain Tip Badge**: Displays current TRU block height (e.g. `🔍 TRU #46,936`). Clicking this badge opens the built-in **TRU Block Explorer** (`/explorer`).
- **Mempool Counter**: Shows unconfirmed transactions currently in the local mempool. Clicking opens the mempool view in the explorer.

### Dual Vault Cards
Positioned at the top of the interface for immediate balance awareness:
1. **TRU Vault Card (Gold)**:
   - Displays your confirmed native TRU balance and unconfirmed balance.
   - Shows your local TRU receiving address (starts with `T`).
   - Action buttons:
     - **Export**: Opens the key reveal modal showing your 24-word BIP-39 mnemonic and WIF private key.
     - **Copy**: Copies your TRU address to clipboard.
     - **Deposit**: Opens the deposit modal displaying a QR code and address for receiving TRU.
     - **Withdraw**: Opens the TRU transfer modal to send coins to an external address.
     - **UTXO Ledger**: Opens the unspent transaction output inspector.
2. **Base Vault Card (Blue)**:
   - Displays your available Base USDC balance and native ETH gas balance.
   - Shows your local Base EVM address (starts with `0x`).
   - Action buttons:
     - **Export**: Opens the export modal with reveal buttons for your EVM private key and 12-word seed.
     - **Copy**: Copies your Base `0x` address to clipboard.
     - **Deposit**: Opens the Base deposit modal with QR code and address.
     - **Withdraw**: Opens the Base withdrawal modal for USDC or ETH.
     - **Base Explorer ↗**: Opens your address directly on BaseScan.

### Exchange Trading Cockpit
Positioned in a clean stacked layout directly below the vault cards:
- **Spot Limit Order (Top Card)**:
  - **Direction Switcher**: Toggle between **Buy TRU** (spending USDC) and **Sell TRU** (spending TRU).
  - **Available Balance Readout**: Dynamically displays available funds in the corresponding vault.
  - **3 Responsive Inputs**:
    - **Price per 1 TRU**: Set your limit price in USDC, with a one-click **Best Price** helper.
    - **Amount**: Specify quantity of TRU, with an interactive percentage slider (25%, 50%, 75%, 100%) visible when selling.
    - **Total**: Calculated total in USDC, with an interactive percentage slider visible when buying.
  - **Action Button**: Displays `Enter Amount & Price` when inputs are empty, updating dynamically to `Buy TRU` or `Sell TRU`.
- **TRU / USDC Spot (Bottom Orderbook Terminal)**:
  - Sub-navigation filter tabs:
    - **📊 Book**: Combined overview of open bids and asks.
    - **🔴 Asks**: Open sell offers from peers (where you can buy TRU immediately).
    - **🟢 Bids**: Open buy offers from peers (where you can sell TRU immediately).
    - **🔄 In-Progress**: Active atomic swaps currently executing, with live status tags and access to the **Automation Audit Modal**.
    - **📜 History**: Historical completed trades and on-chain HTLC transactions.

### Footer
Contains copyright information and the official documentation link: **📖 The TRUSWAP Book** (`/docs`).

---

## 6. Setting Up and Backing Up Your Vault

When TRUSWAP starts for the first time, it automatically creates `data/vault.dat` encrypted with a 256-bit key (`data/.vault.key`) restricted to `0o600` permissions.

### Backing Up Your Recovery Phrases:
1. On the **TRU Vault Card**, click the **Export** button.
   - The modal displays your **24-word BIP-39 seed phrase** and your **WIF Private Key**.
   - Write these down on paper in exact order.
2. On the **Base Vault Card**, click the **Export** button.
   - Click **👁 Show** next to the private key and 12-word seed phrase.
   - Write down your **12-word Base seed phrase** on paper.
3. Keep both paper backups in a safe, offline place.

> [!CAUTION]
> TRUSWAP is strictly non-custodial. If your computer crashes and you do not have these backup phrases, your funds cannot be recovered by anyone.

---

## 7. Funding Your Vault (Deposits & Gas)

Before trading, you must deposit funds into your local vault:

### 1. Depositing TRU (to Sell TRU):
- Click **Deposit** on the TRU Vault Card.
- Copy your `T...` address or scan the displayed QR code.
- Send TRU from your mining pool or external wallet. Once the transaction confirms in a block, your balance updates automatically.

### 2. Depositing Base USDC & ETH (to Buy TRU):
- Click **Deposit** on the Base Vault Card.
- Copy your `0x...` address or scan the QR code.
- Send **Circle USDC on the Base network** to this address.
- **Essential Gas Requirement**: Send a small amount of **native ETH on Base** ($1.00 to $2.00 worth). Every interaction with the Base blockchain (approving USDC, locking deposits, or claiming funds) requires a fraction of a cent in ETH gas. Without ETH in your Base vault, transactions cannot be signed.

---

## 8. Step-by-Step Trading Walkthrough

### Scenario A: Buying TRU with USDC (Taker)
1. In the **TRU / USDC Spot** orderbook below, click the **🔴 Asks** tab to view open sell offers.
2. Find an offer that suits you and click the **[Swap]** button in that row.
3. The **Take Order Modal** appears, showing:
   - **YOU SEND**: Exact amount of USDC required on Base.
   - **YOU RECEIVE**: Exact amount of TRU you will receive.
   - **Maker Beneficiary**: The seller's Base address.
   - **Your Receiving Vault**: Your local TRU address.
   - **Cryptographic Hashlock**: The 20-byte HASH160 digest securing the swap.
   - **Estimated Rate**: Exchange rate (e.g., `1 TRU = 1.0000 USDC`).
4. Click **Confirm Swap**.
5. TRUSWAP executes the swap automatically:
   - Approves USDC spending on the `BaseHTLC` contract if needed.
   - Calls `createSwap` on Base to lock your USDC in escrow.
   - Switches the order to the **🔄 In-Progress** tab.
   - Monitors the TRU blockchain until the seller locks the TRU and 6 confirmations pass.
   - When the seller claims the USDC on Base, TRUSWAP extracts the secret preimage from the on-chain event and automatically claims your TRU on the PoW network.

### Scenario B: Selling TRU for USDC (Maker)
1. In the **Spot Limit Order** card above, ensure the **Sell TRU** tab is selected.
2. Enter your desired **Price (USDC)** and the **Amount (TRU)** you want to sell (or drag the percentage slider).
3. The button changes from `Enter Amount & Price` to **Sell TRU**. Click **Sell TRU**.
4. TRUSWAP cryptographically signs the order with your local vault key and broadcasts it to the P2P relay mesh. The order appears in the orderbook for other traders.
5. When a buyer accepts your order:
   - The swap appears under **🔄 In-Progress**.
   - TRUSWAP broadcasts a 103-byte HTLC script locking your TRU on-chain.
   - Once the buyer deposits their USDC in `BaseHTLC`, TRUSWAP calls `claimSwap` on Base using your secret preimage, receiving your USDC.
   - Publishing the secret allows the buyer to claim their TRU simultaneously.

### The Automation Audit Modal
At any time during an active swap, you can click on the swap in the **🔄 In-Progress** tab to open the **Automation Audit Modal**. This modal displays real-time telemetry from the autonomous swap engine, including step progress indicators, cryptographic hashlocks, block confirmation counters, and raw on-chain transaction hashes.

---

## 9. Withdrawing Funds to External Wallets

When you want to transfer funds out of TRUSWAP to cold storage or an external wallet:

### Withdrawing Native TRU:
1. Click **Withdraw** on the **TRU Vault Card**.
2. Enter the destination TRU address (`T...`), or click the **Camera Icon** to scan a QR code if using a webcam.
3. Click **Max** to withdraw your entire available balance, or enter a specific amount.
4. The modal shows the exact network mining fee (e.g. `0.00000498 TRU`) and the net amount you will receive.
5. Click **Broadcast Withdrawal**. The transaction is signed locally and sent to the TRU P2P network.

### Withdrawing Base Assets (USDC or ETH):
1. Click **Withdraw** on the **Base Vault Card**.
2. Select the asset to withdraw: **USDC** or **ETH**.
3. Enter the destination Ethereum address (`0x...`).
4. Enter the amount or click **Available: Max**.
5. The form calculates the required gas fee. Click **Submit Withdrawal**. The transaction is signed and broadcast via JSON-RPC.
