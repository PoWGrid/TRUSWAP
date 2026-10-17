# Chapter 8: P2P Orderbook Mesh & WebSocket Relay

This chapter explains how trade orders are shared across the network without central matchmaking servers, written for both general users and network engineers.

---

## 1. How the Orderbook Works (For Beginners)

### What is an Orderbook?
An **orderbook** is an open board displaying active trade offers:
- *"Alice wants to sell 500 TRU at $0.05 each ($25.00 USDC total)."*
- *"Bob wants to buy 1,000 TRU at $0.05 each ($50.00 USDC total)."*

When you use TRUSWAP, you can either pick an existing offer from the list to trade immediately (**Taker**), or publish your own offer for others to accept (**Maker**).

### What is an Orderbook Relay?
Because users trade from home Wi-Fi or mobile phones where incoming connections are often blocked by routers or firewalls:
- **Relays act like public message boards**: You connect to a relay over a standard outbound web connection (WebSocket), the same way you load any website.
- When you create an order, you sign it on your device and send it to the relay.
- The relay broadcasts your order to every connected user in real time.

### Can a Relay Steal Your Money?
**No. A relay never has custody of any funds or private keys.**
- The relay only transmits text messages containing signed order parameters in JSON format.
- When another user clicks your order to trade, the actual swap happens directly between your two devices and the respective blockchain smart contracts/scripts.
- Even if a relay is hacked or goes offline, your funds remain secure in your local wallet.

### How Does the Relay Prevent Spam and Fake Orders?
Anyone could try to spam a message board with fake orders like *"I want to sell 10,000,000 TRU."* TRUSWAP relays prevent this through two automated checks:
1. **UtxoGuard**: If you post an order to sell TRU, the relay checks the TRU blockchain. If your wallet address does not actually own those unspent coins, the order is rejected.
2. **BaseWatcher**: If you post an order to buy TRU with USDC, the relay checks Base to confirm you hold the required USDC balance and have approved the HTLC contract.

### What Protocol is Used?
- **Between the UI and the Local App**: Communication is **100% standard JSON** over local WebSocket (`ws://127.0.0.1:8899/ws`). The frontend sends readable JSON action commands and receives JSON event updates.
- **Between the App and the Public Relay**: Communication is also **100% standard JSON over WebSockets** (`wss://.../orderbook`). Orders, cancellations, and orderbook snapshots are transmitted as UTF-8 JSON text messages.

---

## 2. Relay Architecture

<div class="cyber-diagram-box">
  <div class="diagram-title-bar">
    <span class="diagram-tag">ORDERBOOK RELAY</span>
    <span class="diagram-heading">Maker-Taker WebSocket Architecture</span>
  </div>

  <div class="flow-steps">
    <div class="flow-step node-tru">
      <div class="flow-step-num">1</div>
      <div class="flow-step-content">
        <div class="flow-step-title">MAKER (Alice: Desktop)</div>
        <div class="flow-step-desc">Submits Signed <code>SELL_TRU</code> Order with UTXO ownership proof via WebSocket</div>
      </div>
    </div>

    <div class="flow-connector">▼ WebSocket (JSON Text Frames)</div>

    <div class="diagram-node node-app">
      <div class="node-badge">⚡ TRUSWAP ORDERBOOK RELAY SERVICE</div>
      <div class="node-title">Verification & In-Memory Matching</div>
      <div class="node-tags-grid">
        <div class="feature-tag">ConnectionGuard (Per-IP limits)</div>
        <div class="feature-tag">UtxoGuard (Live PoW UTXO check)</div>
        <div class="feature-tag">BaseWatcher (Base USDC check)</div>
        <div class="feature-tag">In-Memory Book (Instant match)</div>
      </div>
    </div>

    <div class="flow-connector">▼ Real-time JSON Broadcast Event Stream</div>

    <div class="flow-step node-base">
      <div class="flow-step-num">2</div>
      <div class="flow-step-content">
        <div class="flow-step-title">TAKER (Bob: Android)</div>
        <div class="flow-step-desc">Receives instant orderbook update, clicks Swap, and locks Base USDC in BaseHTLC</div>
      </div>
    </div>
  </div>
</div>

### Relay Capacity Profiles:
Relay operators can select an operating profile based on intended server scale:
- **Standard Profile** (`--blazing`): Default setting for personal nodes and smaller networks (supports up to 5,000 concurrent client streams).
- **High-Capacity Server Profile** (`--ultrablazing`): Setting for public relay infrastructure. Automatically configures Linux operating system limits (`tune_kernel_limits()`) to 1,048,576 file descriptors to handle high connection volume.

---

## 3. Cryptographic Order Structure (`src/relay/protocol.rs`)

Orders submitted to a relay are cryptographically signed by the creator using standard JSON payloads:

```json
{
  "order_id": "8a3f91c2...64hex",
  "side": "SELL_TRU",
  "amount_tru": 500.0,
  "amount_usdc": 25.0,
  "price": 0.05,
  "hash160_hex": "3f91c28a...40hex",
  "timelock": 1790900000,
  "maker_tru_address": "T...",
  "maker_tru_pubkey": "03a1b2...",
  "maker_base_address": "0x1111111111111111111111111111111111111111",
  "signature_base": "0x4b2c...",
  "signature_tru": null,
  "created_at": 1790800000,
  "is_matched": false
}
```

### Order Verification Rules:
1. **Base Signature Verification**: The relay runs ECDSA `ecrecover` on `signature_base` using the EIP-191 personal sign standard. The recovered address must match `maker_base_address`.
2. **TRU Signature Verification**: If `signature_tru` is provided, the relay verifies the SECP256k1 signature against `maker_tru_address`.
3. **Payload Immutability**: `order_id` is computed from the order fields. Modifying amounts, prices, addresses, or timelocks breaks the signature and causes the order to be rejected.

---

## 4. Relay Messaging Protocol (JSON over WebSockets)

All communication between clients and the relay mesh uses human-readable, UTF-8 JSON text messages:

### Client Commands (`ClientMessage`):
- **Submit Order**:
  ```json
  {
    "action": "SubmitOrder",
    "payload": {
      "order_id": "8a3f...",
      "side": "SELL_TRU",
      "amount_tru": 500.0,
      "amount_usdc": 25.0,
      "hash160_hex": "3f91...",
      "timelock": 1790900000,
      "maker_tru_address": "T...",
      "maker_base_address": "0x...",
      "signature_base": "0x..."
    }
  }
  ```
- **Cancel Order**:
  ```json
  {
    "action": "CancelOrder",
    "payload": {
      "order_id": "8a3f...",
      "maker_base_address": "0x...",
      "signature_base": "0x..."
    }
  }
  ```
- **Take Order**:
  ```json
  {
    "action": "TakeOrder",
    "payload": {
      "order_id": "8a3f...",
      "taker_base_address": "0x...",
      "taker_tru_address": "T...",
      "signature_base": "0x..."
    }
  }
  ```
- **Ping / Liveness**:
  ```json
  {
    "action": "Ping"
  }
  ```

### Relay Broadcast Events (`RelayBroadcastEvent`):
- **Orderbook Snapshot**: Pushed immediately when a client connects, containing all currently active orders, server timestamp, and operational mode.
- **Order Added**: Broadcast when a verified order is submitted to the book.
- **Order Removed**: Broadcast when an order is cancelled by its maker or taken by a counterparty.
- **Order Matched**: Broadcast to notify participants that a taker has initiated the atomic swap escrow.
- **Pong**: Heartbeat reply confirming connection health.
- **Error**: Returned when an invalid payload or unauthorized action occurs:
  ```json
  {
    "event": "Error",
    "data": {
      "code": "INVALID_SIGNATURE",
      "message": "Recovered address does not match maker_base_address"
    }
  }
  ```

---

## 5. Network Connectivity & Firewall Traversal

- **Standard Outbound Ports**: Clients connect over outbound TCP ports `80` (`ws://`) or `443` (`wss://`), allowing the app to function on standard home broadband, office networks, and mobile data (4G/5G).
- **No Port Forwarding**: Users do not need to configure router settings, UPnP, or dynamic DNS to trade.
- **Auto-Reconnect**: The client automatically reconnects with exponential backoff if a connection drops during network changes.

---

## 6. How the Orderbook Appears in the User Interface

The P2P orderbook relay mesh feeds directly into the interactive Exchange Trading Cockpit on `http://127.0.0.1:8899`:

1. **Spot Limit Order (Top Card)**:
   - **`[Buy TRU]` / `[Sell TRU]` Direction Switcher**: Toggle your trade intent with a single click.
   - **Available Balance**: Reflects spendable funds in your local vault (Base USDC for buying, TRU for selling).
   - **Price per 1 TRU**: Set your limit price in USDC. Click the **Best Price** hint to automatically fill the lowest current ask or highest bid.
   - **Interactive Percentage Sliders (`25%`, `50%`, `75%`, `100%`)**: Smooth sliders with snap marks allow precise allocation of your available vault balance without typing numbers manually.
   - **Dynamic Action Button**: Reads `Enter Amount & Price` when inputs are empty, updating automatically to `Buy TRU` or `Sell TRU` once amounts are specified. Clicking signs the order with your local private key and pushes it to the relay via WebSocket.

2. **TRU / USDC Spot (Bottom Orderbook Terminal)**:
   - **`📊 Book` Tab**: Unified orderbook view showing market spread and depth.
   - **`🔴 Asks` Tab**: Lists open sell offers. To buy TRU immediately, click **[Swap]** on any row to open the Take Order Modal.
   - **`🟢 Bids` Tab**: Lists open buy offers. To sell TRU immediately, click **[Swap]** to accept the offer.
   - **`🔄 In-Progress` Tab**: Real-time tracker for executing atomic swaps. Active trades display step badges (e.g. `Locking TRU`, `Confirming`, `Claiming`). Clicking any trade opens the **Automation Audit Modal** with live terminal telemetry.
   - **`📜 History` Tab**: Log of settled swaps and verified on-chain HTLC contracts.
   - **Order Cancellation**: If you posted an active order, it shows a red **[Cancel]** button in the table so you can withdraw it from the relay at any time. All updates sync instantly across clients via JSON WebSockets without page reloads.
