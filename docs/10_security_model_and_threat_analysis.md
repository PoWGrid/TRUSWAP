# Chapter 10: Security Model & Threat Analysis

This chapter outlines the security assumptions, potential threat vectors, and cryptographic protections implemented in TRUSWAP, explained for everyday users and security researchers.

---

## 1. Security Overview (For Beginners)

### Can I lose my money if the other person cheats or disconnects?
**No.** The atomic swap protocol relies on mathematics rather than trust:
- Either the complete swap finishes and both people receive their new coins, OR
- The trade cancels and both people receive their original deposits back.
- There is no scenario where the other person can take your money without revealing the secret that allows you to take theirs.

### What happens if my device loses power or internet during a trade?
All active trade states, cryptographic keys, and secret preimages are saved to your local disk in `vault.dat`:
- When you reconnect your internet and reopen TRUSWAP, the app automatically inspects the blockchains.
- If the other person completed their side of the trade, TRUSWAP claims your coins.
- If the trade was abandoned and the timer has expired, TRUSWAP presents the refund button to retrieve your deposit.

### Can someone change the price while the trade is happening?
**No.** The trade amount and exchange price are permanently locked into the smart contract and transaction scripts at the start of the trade. Neither party can alter the terms once the contracts are created.

---

## 2. Core Security Invariants (For Developers)

1. **Solvency Invariant**: An honest participant never loses their deposited principal without receiving the agreed counterparty asset, provided they broadcast their claim or refund transaction before the respective timelock window expires.
2. **Key Exclusivity**: Private keys are generated locally via CSPRNG, never leave process memory, and are never transmitted over network sockets or written to logs.
3. **Decentralized Operation**: The system does not depend on any central operator or matching engine; users can interact directly or deploy private relays.

---

## 3. Threat Vectors and Protocol Mitigations

### Vector A: The "Free Option" Problem & Preimage Extortion
- **The Threat**: A taker waits until the final minutes of a lock period to see if market prices fluctuate. If the price moves favorably, they claim; if the price drops, they let the trade expire, treating the lock as a free financial option.
- **Protocol Mitigation**:
  - **Timelock Asymmetry**: The Taker's timelock ($T_2$) expires substantially earlier than the Maker's timelock ($T_1$):
    $$T_1 - T_2 \ge \Delta_{\text{safety}} \quad (\text{typically } 24 \text{ hours})$$
  - The Maker is motivated to claim their funds before $T_2$ expires. The moment the Maker claims, the secret is published on-chain, giving the Taker a guaranteed 24-hour window to claim their TRU deposit on the Proof-of-Work chain before $T_1$ expires.

### Vector B: Blockchain Reorganizations (51% Attacks)
- **The Threat**: An attacker deposits funds into an HTLC, waits for the counterparty to lock funds on the other chain, and then mines a private chain reorganization to erase their initial deposit transaction.
- **Protocol Mitigation**:
  - **TRU PoW Depth**: The client enforces a **6-block confirmation threshold** via `ConfirmationGuard` before the counterparty contract is deployed on Base.
  - **Base Finality**: The client verifies RPC block receipts and L1 settlement before treating EVM transactions as finalized.

### Vector C: Timelock Griefing & Fake Liquidity
- **The Threat**: A malicious user floods the orderbook with fictitious buy or sell offers to lock up honest traders' liquidity in expired contracts.
- **Protocol Mitigation**:
  - **UtxoGuard**: The relay validates that makers creating `SELL_TRU` orders provably own confirmed unspent outputs on the TRU blockchain.
  - **BaseWatcher**: The relay verifies that makers creating `BUY_TRU` orders hold adequate USDC balance and contract allowance on Base.
  - **ConnectionGuard**: Enforces connection quotas and order submission rate limits per IP address.

---

## 4. Cryptographic & System Security

- **Constant-Time Verification**: Preimage comparisons ($\text{ripemd160}(\text{sha256}(S)) == \text{hash160}$) are executed using constant-time equality checks to mitigate timing side-channel attacks.
- **Short Memory Lifetimes**: Private keys are instantiated only during signature production and dropped immediately afterward.
- **Strict POSIX Permissions (`0o600`)**: On Unix, Linux, and Android systems, the master key (`.vault.key`) and vault database (`vault.dat`) are created with `0o600` permissions (read/write access restricted to the process owner).
- **Local Loopback API Binding**: The embedded daemon binds exclusively to `127.0.0.1` to prevent unauthorized local network access, with incoming WebSocket handshake validation against Cross-Site WebSocket Hijacking (CSWSH).

---

## 5. Security Transparency in the User Interface

TRUSWAP gives users direct visual tools to verify security invariants:

1. **Automation Audit Modal**:
   Clicking any active swap in the **🔄 In-Progress** tab opens the audit dialog. Users can inspect the exact HASH160 digest, watch the confirmation counter increment toward 6 blocks, and see raw blockchain transaction hashes.
2. **Explicit Timelock Countdown & Refund Button**:
   Every in-progress swap tracks the elapsed blockchain time. If a counterparty disconnects or fails to claim before their deadline, the UI status switches to `Expired` and displays an active **[Refund]** button to immediately reclaim locked funds.
3. **UTXO Ledger Isolation**:
   In the **UTXO Ledger** modal, outputs locked in active HTLC scripts are visually tagged and deducted from spendable balances, ensuring funds in flight cannot be accidentally spent elsewhere.
4. **Independent On-Chain Verification**:
   All contract interactions provide direct links to the built-in **TRU Block Explorer** (`/explorer`) and **BaseScan**, allowing traders to verify state on public ledgers independently.
