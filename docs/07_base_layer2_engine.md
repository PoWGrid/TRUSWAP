# Chapter 7: Base Layer-2 Integration & Smart Contracts

This chapter explains how TRUSWAP interacts with Ethereum's Base Layer-2 network to settle stablecoin swaps, covering both everyday user requirements and technical smart contract interfaces.

---

## 1. Base Network Fundamentals (For Beginners)

### What is Base?
Base is an Ethereum Layer-2 (L2) network developed by Coinbase:
- **Fast Settlements**: Blocks confirm in approximately **2 seconds**.
- **Low Fees**: Network transaction fees (called **gas**) typically cost less than **$0.01** per transaction.
- **Ethereum Security**: Base settles its transaction proofs directly to the main Ethereum blockchain.

### What is USDC?
USD Coin (USDC) is a digital dollar issued by Circle. 1 USDC is pegged to 1 US Dollar. When trading on TRUSWAP, USDC provides price stability against market volatility.

### What Assets Do You Need in Your Base Wallet?
To execute a trade on Base, your Base address (starts with `0x`) needs:
1. **A Small Amount of ETH (for Gas)**:
   - Every transaction on Base (approving tokens, locking deposits, or claiming funds) requires a tiny fraction of ETH to pay the network.
   - Having **$1 to $2 worth of ETH** in your wallet is typically enough for dozens of swaps.
2. **USDC (if Buying TRU)**:
   - If you want to buy TRU, you need Circle USDC on Base in your wallet.
   - If you are selling TRU, you do not need USDC upfront; you will receive USDC when the swap completes.

### Why Are Browser Extensions (MetaMask) Not Needed?
Most decentralized exchange websites force you to install a browser extension (like MetaMask or Coinbase Wallet) and click "Confirm" on multiple popups for every step of a trade.

TRUSWAP eliminates this complexity:
- It includes a **built-in EVM signer** written in native Rust.
- Transactions are constructed and signed locally on your machine.
- Token approvals and claim executions are processed automatically in the background.

---

## 2. Built-in Headless EVM Signer (`src/chains/evm/signer.rs`)

TRUSWAP interfaces directly with public Base JSON-RPC endpoints:

### Key Features:
- **No Third-Party Extension Dependency**: Directly generates, signs, and broadcasts raw transactions via JSON-RPC.
- **Automatic RPC Failover**: Rotates between public Base RPC endpoints (`https://mainnet.base.org`, `base-rpc.publicnode.com`, `1rpc.io/base`, `base.llamarpc.com`).
- **Sequential Nonce Tracker**: Protects against nonce collisions when multiple transactions are staged in succession.

### Core JSON-RPC Methods:
- `eth_blockNumber`: Checks the latest block height.
- `eth_getBalance`: Retrieves the account's ETH balance (for gas).
- `eth_call`: Executes read-only contract calls (checking USDC balance, allowances, swap states).
- `eth_getTransactionCount`: Queries account nonce (`pending` and `latest`).
- `eth_gasPrice`: Queries the current network base fee and priority fee.
- `eth_sendRawTransaction`: Broadcasts locally signed RLP-encoded transaction envelopes.
- `eth_getTransactionReceipt`: Polls for confirmation receipts and event logs.

---

## 3. EIP-155 Transaction Signing Pipeline

Every transaction submitted to Base is signed locally using EIP-155 replay protection:

1. **Construct Unsigned Payload**:
   $$\text{SigningPayload} = \text{RLP}([nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0])$$
2. **Keccak-256 Hashing**:
   $$\text{TxHash} = \text{Keccak-256}(\text{SigningPayload})$$
3. **ECDSA Signature (`secp256k1`)**:
   Produces signature parameters $(r, s)$ and recovery identifier $rec\_id \in \{0, 1\}$.
4. **EIP-155 Recovery ID ($v$)**:
   $$v = \text{chainId} \times 2 + 35 + rec\_id$$
   - **Base Mainnet** (`chainId = 8453`): $v \in \{16941, 16942\}$
   - **Base Sepolia** (`chainId = 84532`): $v \in \{169099, 169100\}$
5. **RLP-Encode Final Envelope**:
   $$\text{RawTx} = \text{RLP}([nonce, gasPrice, gasLimit, to, value, data, v, r, s])$$

---

## 4. ERC-20 Token Engine (USDC)

For stablecoin swaps, TRUSWAP interfaces with the official Circle USDC contract:

1. **Decimals Normalization**:
   - USDC uses **6 decimal places** ($1.00 = 1,000,000$ units).
   - TRU uses **8 decimal places** ($1.00000000 = 100,000,000$ atoms).
   - Integer fixed-point math is used exclusively to prevent floating-point rounding errors.
2. **Two-Step ERC-20 Allowance & Approval**:
   - Before locking tokens in an HTLC, the client calls `allowance(owner, htlcAddress)` (selector `0xdd62ed3e`).
   - If current allowance is less than the required swap amount, TRUSWAP automatically constructs and broadcasts an `approve(htlcAddress, amount)` transaction (selector `0x095ea7b3`) before creating the swap.

---

## 5. BaseHTLC Smart Contract

### Deployed Contract Addresses:

| Network | Contract | Address | Explorer Link |
|---|---|---|---|
| **Base Mainnet** (`8453`) | `BaseHTLC` | `0x812b611208297eb7A2b153BECA8b2AF47C7614DF` | [View on BaseScan](https://basescan.org/address/0x812b611208297eb7A2b153BECA8b2AF47C7614DF) |
| **Base Mainnet** (`8453`) | Official Circle `USDC` | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` | [View on BaseScan](https://basescan.org/token/0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913) |
| **Base Sepolia** (`84532`) | `BaseHTLC` (Testnet) | `0xF2e70fbd7Da79d188f805e61e8587AACD49772a5` | [View on Sepolia BaseScan](https://sepolia.basescan.org/address/0xF2e70fbd7Da79d188f805e61e8587AACD49772a5) |
| **Base Sepolia** (`84532`) | Test `USDC` | `0x036CbD53842c5426634e7929541eC2318f3dCF7e` | [View on Sepolia BaseScan](https://sepolia.basescan.org/token/0x036CbD53842c5426634e7929541eC2318f3dCF7e) |

### Solidity Interface (`IBaseHTLC`):

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IBaseHTLC {
    // 1. Lock funds for a swap
    // Selector: 0xefafa23f
    function createSwap(
        address beneficiary,
        address token,
        uint256 amount,
        bytes20 hash160Digest,
        uint256 timelock,
        bytes32 salt
    ) external returns (bytes32 swapId);

    // 2. Claim funds with the 32-byte secret preimage
    // Selector: 0xf0331024
    function claimSwap(bytes32 swapId, bytes calldata secret) external;

    // 3. Refund deposit if the timelock has elapsed
    // Selector: 0x7249fbb6
    function refundSwap(bytes32 swapId) external;

    // Event emitted when a swap is created
    event SwapCreated(
        bytes32 indexed swapId,
        address indexed sender,
        address indexed beneficiary,
        address token,
        uint256 amount,
        bytes20 hash160,
        uint256 timelock
    );

    // Event emitted when funds are claimed (revealing the secret)
    event SwapClaimed(
        bytes32 indexed swapId,
        bytes secret,
        address claimer
    );
}
```

### Event Log Monitoring:
- `SwapCreated` Topic 0: `0x6095347ebb4084720e12a89ad2c70fc169534308159a6884c677a6ec9a30742c`
- `SwapClaimed` Topic 0: `0xd14278a0c2b5ef6c1c6418dba05a4ebd99536912416835fde879006db64b8d06`

When a participant claims their USDC on Base, the `SwapClaimed` event publishes the secret on-chain. TRUSWAP's background watcher (`src/chains/evm/mod.rs`) reads this transaction receipt, extracts the secret bytes, and automatically claims the TRU deposit on the Proof-of-Work blockchain.

---

## 6. How Base Layer-2 Is Managed in the TRUSWAP Interface

The Base EVM engine is integrated into the user interface through dedicated controls:

1. **The Base Vault Card**:
   - **USDC Balance**: Displays available stablecoin funds ready to buy TRU.
   - **Gas Balance (`⛽ Gas: X.XXXX ETH`)**: Highlights your native ETH balance directly on the card. This ensures you always know if you have sufficient gas to execute contracts before starting a trade.
   - **Address (`0x...`)**: Displays your local EVM address with one-click clipboard copying.
   - **Base Explorer ↗ Link**: Opens your account directly on BaseScan or Sepolia BaseScan to check on-chain token transfers.

2. **Base Modal Dialogs**:
   - **Deposit**: Displays your Base address and QR code.
   - **Withdraw**: Allows withdrawing either **USDC** or native **ETH** to any external Ethereum address. Includes a **Max** button and calculates estimated network gas fees automatically.
   - **Export Keys**: Reveals your **12-word recovery mnemonic** and raw private key behind visibility toggles (`👁 Show`).

3. **Autonomous Gas and Token Handling**:
   - During swaps, you do not need to manually calculate gas prices or approve contract allowances.
   - TRUSWAP checks the current ERC-20 `allowance`. If an allowance is required, it signs the `approve()` transaction automatically and proceeds to `createSwap()`.
   - If your ETH gas balance is insufficient, the UI warns you before attempting the transaction, preventing lost gas or stuck orders.
