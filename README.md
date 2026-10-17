# TRUSWAP

[![Language](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Network](https://img.shields.io/badge/Network-TRU%20(21e8)-gold.svg)](https://tokenizedrealutility.com)
[![Base](https://img.shields.io/badge/Base-EVM-0052FF.svg)](https://basescan.org)
[![USDC](https://img.shields.io/badge/USDC-Circle-2775CA.svg)](https://www.circle.com/en/usdc)

**TRUSWAP** is a high-performance cross-chain P2P atomic swap client and runner written in Rust. It enables trustless, non-custodial trades between **TRU** (Tokenized Real Utility, Nakamoto PoW with the 21e8 puzzle) and **Circle USDC** on **Base** (Ethereum L2) without centralized intermediaries or wrapped tokens.

---

## Key Features

- **P2P Atomic Swaps**: Cryptographically guaranteed cross-chain swaps powered by mathematical Hash Time-Locked Contracts (HTLC).
- **Zero Counterparty Risk**: Funds are either claimed upon secret preimage reveal or automatically refunded upon timelock expiry.
- **Embedded Light Node**: Built-in autonomous SPV block scanner and header sync for direct TRU P2P wire verification.
- **Client-Side Encrypted Vault**: Local multi-chain keystore (`vault.dat`) protected with Argon2id and AES-256-GCM.
- **Distributed Relay Mesh**: Off-chain order matching and discovery via `wss://tru.powgrid.xyz/orderbook` with fail-closed cryptographic signature verification.
- **Cross-Platform**: Native standalone binaries for Linux (x86_64 & ARM64), Windows (x64), and Android (APK).
- **Embedded Web GUI & Explorer**: Dark-mode web cockpit and real-time block explorer served locally at `http://127.0.0.1:8899`.
- **Offline Documentation**: Full 11-chapter interactive book ("The TRUSWAP Book") built into the binary at `http://127.0.0.1:8899/docs`.

---

## Network Parameters & Contracts

### Base Mainnet (Default)
| Parameter | Value |
| :--- | :--- |
| **Chain ID** | `8453` |
| **RPC Endpoint** | `https://mainnet.base.org` |
| **BaseHTLC Contract** | [`0x812b611208297eb7A2b153BECA8b2AF47C7614DF`](https://basescan.org/address/0x812b611208297eb7A2b153BECA8b2AF47C7614DF) |
| **Circle USDC** | [`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`](https://basescan.org/token/0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913) |

### TRU PoW Network
| Parameter | Value |
| :--- | :--- |
| **Consensus** | `SHA256d + 21E8` Proof-of-Work |
| **P2P Wire Port** | `21833` |
| **Confirmation Safety** | $\ge 6$ block confirmations |

---

## Quick Start

### Pre-Built Binaries
Download the latest standalone release for your platform from [GitHub Releases](https://github.com/PoWGrid/TRUSWAP/releases).

### Building from Source

```bash
# Clone the repository
git clone https://github.com/PoWGrid/TRUSWAP.git
cd TRUSWAP

# Run via convenience script (auto-builds release)
./run.sh

# Or build and execute directly
cargo build --release
./target/release/truswap
```

### Accessing the Web Interface
Once started, open `http://127.0.0.1:8899` in your web browser. The application prints an ephemeral session token upon startup:
```text
UI URL: http://127.0.0.1:8899/?token=<EPHEMERAL_TOKEN>
```

---

## CLI Options

Configuration precedence: **CLI Arguments** $>$ **Environment Variables** $>$ **Defaults**.

| Flag | Description | Default |
| :--- | :--- | :--- |
| `--mainnet` | Run on Base Mainnet | `true` |
| `--testnet` | Switch to Base Sepolia testnet | `false` |
| `--bind <IP:PORT>` | Custom IP address and port for the Web UI | `127.0.0.1:8899` |
| `--dev` | Bind UI server to all network interfaces (`0.0.0.0:8899`) | `false` |
| `--relay-node <URL>` | Custom Orderbook Relay WebSocket URL | `wss://tru.powgrid.xyz/orderbook` |
| `--base-rpc <URL>` | Custom Base JSON-RPC HTTP endpoint | `https://mainnet.base.org` |
| `--data-dir <PATH>` | Custom directory for keystore and block data | `./data` |
| `--resync` | Purge local block cache (`tru.dat`) and resync from genesis | `false` |
| `--relay` | Run node as an orderbook relay server | `false` |

---

## Testing

Run the automated test suite:
```bash
cargo test
```

---

## License

Licensed under the [MIT License](LICENSE).
