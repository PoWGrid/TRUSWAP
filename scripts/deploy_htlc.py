#!/usr/bin/env python3
import json
import sys
import os
from web3 import Web3

RPC_URL = os.environ.get("BASE_RPC_URL", "https://mainnet.base.org")
CHAIN_ID = int(os.environ.get("CHAIN_ID", "8453"))
DEPLOYER_KEY = os.environ.get("DEPLOYER_KEY")
USDC_ADDR = os.environ.get("BASE_USDC_CONTRACT", "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")

def main():
    if not DEPLOYER_KEY:
        print("Error: DEPLOYER_KEY environment variable is required", file=sys.stderr)
        sys.exit(1)

    w3 = Web3(Web3.HTTPProvider(RPC_URL))
    if not w3.is_connected():
        print(f"Error: Failed to connect to Base RPC at {RPC_URL}", file=sys.stderr)
        sys.exit(1)

    account = w3.eth.account.from_key(DEPLOYER_KEY)
    deployer_addr = account.address
    balance = w3.eth.get_balance(deployer_addr)
    print(f"Deployer: {deployer_addr}")
    print(f"Balance:  {w3.from_wei(balance, 'ether')} ETH")

    if balance == 0:
        print("Error: Deployer balance is 0 ETH", file=sys.stderr)
        sys.exit(1)

    abi_path = os.path.join(os.path.dirname(__file__), "../contracts/BaseHTLC.abi")
    bin_path = os.path.join(os.path.dirname(__file__), "../contracts/BaseHTLC.bin")

    with open(abi_path) as f:
        abi = json.load(f)
    with open(bin_path) as f:
        bytecode = f.read().strip()
        if not bytecode.startswith("0x"):
            bytecode = "0x" + bytecode

    print("Preparing deployment transaction...")
    htlc_factory = w3.eth.contract(abi=abi, bytecode=bytecode)

    nonce = w3.eth.get_transaction_count(deployer_addr, "pending")
    gas_price = w3.eth.gas_price

    # Constructor takes address _usdc
    deploy_tx = htlc_factory.constructor(Web3.to_checksum_address(USDC_ADDR)).build_transaction({
        "chainId": CHAIN_ID,
        "from": deployer_addr,
        "nonce": nonce,
        "gasPrice": int(gas_price * 1.2),  # 20% buffer for fast inclusion
    })

    # Estimate gas
    try:
        estimated_gas = w3.eth.estimate_gas(deploy_tx)
        deploy_tx["gas"] = int(estimated_gas * 1.25)
    except Exception as e:
        print(f"Gas estimation warning: {e}, using fallback gas limit 1,500,000")
        deploy_tx["gas"] = 1500000

    print(f"Estimated Gas: {deploy_tx['gas']}, Gas Price: {w3.from_wei(deploy_tx['gasPrice'], 'gwei')} Gwei")
    signed_tx = w3.eth.account.sign_transaction(deploy_tx, private_key=DEPLOYER_KEY)
    
    print("Broadcasting transaction...")
    tx_hash = w3.eth.send_raw_transaction(signed_tx.raw_transaction)
    tx_hash_hex = tx_hash.hex()
    if not tx_hash_hex.startswith("0x"):
        tx_hash_hex = "0x" + tx_hash_hex
    print(f"Deploy Tx Hash: {tx_hash_hex}")
    print("Waiting for transaction receipt...")

    receipt = w3.eth.wait_for_transaction_receipt(tx_hash, timeout=120)
    if receipt.status == 1:
        contract_addr = receipt.contractAddress
        print("=" * 60)
        print(" BaseHTLC DEPLOYED SUCCESSFULLY!")
        print(f" Contract Address: {contract_addr}")
        print(f" Block Number:     {receipt.blockNumber}")
        print(f" Gas Used:         {receipt.gasUsed}")
        explorer_domain = "sepolia.basescan.org" if CHAIN_ID == 84532 else "basescan.org"
        print(f" BaseScan Link:    https://{explorer_domain}/address/{contract_addr}")
        print("=" * 60)
        
        # Verify USDC constant in deployed contract
        deployed_contract = w3.eth.contract(address=contract_addr, abi=abi)
        usdc_val = deployed_contract.functions.USDC().call()
        print(f"Verified on-chain USDC: {usdc_val}")
        return contract_addr
    else:
        print(f"Error: Transaction failed with status {receipt.status}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
