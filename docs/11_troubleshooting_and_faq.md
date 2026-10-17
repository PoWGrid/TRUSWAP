# Chapter 11: Troubleshooting, FAQ & Configuration

This chapter provides answers to common user questions, solutions to error messages, and a complete command-line reference for operators.

---

## 1. Frequently Asked Questions (FAQ)

### How much does it cost to use TRUSWAP?
**TRUSWAP charges 0.00% protocol fees.**  
There are no exchange commissions, registration costs, or deposit/withdrawal fees. You only pay standard on-chain blockchain transaction fees:
- **On TRU**: A standard mining transaction fee (typically around 0.001 TRU).
- **On Base**: A fraction of a cent in native ETH (typically $0.001 to $0.01).

### Do I need to create an account or verify my identity (KYC)?
**No.** TRUSWAP is open-source software running directly on your computer or phone. There are no accounts, email addresses, phone numbers, passwords, or central servers.

### Why does the app say "Insufficient Gas" when I have enough USDC?
On the Base network, token transfers and smart contract calls require native **ETH** to pay for network processing ("gas"). Even if you have 1,000 USDC, you cannot execute transactions without a tiny amount of ETH. Sending **$1.00 to $2.00 worth of ETH on Base** to your wallet is sufficient for dozens of swaps.

### Why is my swap waiting for block confirmations?
Proof-of-Work blockchains like TRU can occasionally experience orphaned blocks or short chain reorganizations. To ensure a deposit is irreversible before the buyer locks their USDC on Base, TRUSWAP requires **6 confirmed blocks** on the TRU network. This process takes a few minutes.

### What happens if the other trader disappears or closes their app?
If the counterparty abandons the swap, your funds are safe:
- When the timelock window expires (24 or 48 hours depending on your role), TRUSWAP will display a **Refund** button.
- Clicking **Refund** executes the refund path in the contract/script, returning 100% of your principal to your wallet.

### Can I use the same wallet on both my phone and computer?
Yes. Click **Export** on the TRU Vault Card and Base Vault Card on your primary device to view your backup phrases (24 words for TRU and 12 words for Base). On desktop devices, you can also copy your `data/vault.dat` and `data/.vault.key` files directly to the other computer's `data/` folder.

### How do I inspect my individual UTXOs and unspent coin outputs?
On the **TRU Vault Card** at the top left of the dashboard, click the **UTXO Ledger** link. A modal opens listing all unspent outputs with their TXID, output index (`vout`), value in TRU and raw atoms, confirmation count, and lock status.

### How do I view live blockchain blocks and transactions?
Click the **`🔍 TRU #<Height>`** badge in the top header bar, or navigate directly to `http://127.0.0.1:8899/explorer`. The built-in explorer lets you search by block height, hash, TXID, or address.

### Where can I read the full documentation inside the app?
Scroll to the footer at the bottom of the page and click **📖 The TRUSWAP Book**, or navigate directly to `http://127.0.0.1:8899/docs`.

---

## 2. Common Errors & How to Fix Them

### 1. "Camera permission denied" on Android
- **Cause**: Android blocked access to the camera hardware.
- **Solution**: Open your phone's **Settings** $\rightarrow$ **Apps** $\rightarrow$ **TRUSWAP** $\rightarrow$ **Permissions** $\rightarrow$ **Camera** $\rightarrow$ Select **"Allow only while using the app"**.

### 2. "Base RPC timeout or 429 Too Many Requests"
- **Cause**: The default public Base RPC endpoint (`https://mainnet.base.org`) is experiencing rate-limiting during high traffic.
- **Solution**: Launch TRUSWAP with an alternate public or private RPC endpoint:
  ```bash
  ./truswap --base-rpc https://base-rpc.publicnode.com
  ```

### 3. "No peers connected on TRU network"
- **Cause**: Outbound TCP port `21833` may be blocked by your local network or firewall.
- **Solution**: Ensure your firewall allows outbound TCP traffic on port `21833`, or supply alternative peer addresses using `--tru-seeds`.

---

## 3. Command-Line Reference & Configuration

TRUSWAP can be customized via command-line flags or environment variables:

| Flag | Default | Environment Variable | Description |
|---|---|---|---|
| `--bind <IP:PORT>` | `127.0.0.1:8899` | `BIND_ADDR` | Local web dashboard and API server address |
| `--dev` | `false` | `TRUSWAP_DEV=1` | Binds to `0.0.0.0:8899` to allow access from local network |
| `--mainnet` | `true` | `TRUSWAP_TESTNET=0` | Run on Base Mainnet (`8453`) |
| `--testnet` | `false` | `TRUSWAP_TESTNET=1` | Run on Base Sepolia testnet (`84532`) |
| `--data-dir <PATH>` | `./data` | `DATA_DIR` | Directory storing `vault.dat`, `tru.dat`, and `.vault.key` |
| `--tru-seeds <LIST>` | Hardcoded list | `TRU_SEEDS` | Comma-separated list of TRU Proof-of-Work seed nodes (`IP:21833`) |
| `--base-rpc <URL>` | `https://mainnet.base.org` | `BASE_RPC_URL` | Base Layer-2 JSON-RPC endpoint |
| `--base-wss <URL>` | `wss://mainnet.base.org` | `BASE_WSS_URL` | Base WebSocket endpoint for event monitoring |
| `--htlc <ADDRESS>` | `0x812b...4DF` | `BASE_HTLC_CONTRACT` | Address of deployed `BaseHTLC` smart contract |
| `--usdc <ADDRESS>` | `0x8335...913` | `BASE_USDC_CONTRACT` | Address of Circle USDC token contract |
| `--relay` | `false` | `TRUSWAP_RELAY=1` | Run node as an orderbook relay message router |
| `--relay-node <URL>` | `wss://tru.powgrid.xyz/orderbook` | `TRUSWAP_RELAY_NODE` | Orderbook relay node WebSocket endpoint to connect to |
| `--blazing` | `true` (if relay) | `RELAY_MODE=blazing` | Standard relay capacity profile (handles up to 5,000 concurrent client streams) |
| `--ultrablazing` | `false` | `RELAY_MODE=ultrablazing` | High-capacity relay profile (tunes Linux limits to 1,000,000 file descriptors) |
| `--resync` | `false` | `RESYNC=1` | Clear local block cache and re-download headers from genesis |
