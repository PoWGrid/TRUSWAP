import time
import sys
import os
from web3 import Web3
from eth_account import Account

rpc = os.environ.get("BASE_RPC_URL", "https://mainnet.base.org")
chain_id = int(os.environ.get("CHAIN_ID", "8453"))
w3 = Web3(Web3.HTTPProvider(rpc))

operator_key = os.environ.get("REFUND_PRIVATE_KEY") or os.environ.get("OPERATOR_KEY")
if not operator_key:
    print("Error: REFUND_PRIVATE_KEY or OPERATOR_KEY environment variable is required", file=sys.stderr)
    sys.exit(1)

operator_acc = Account.from_key(operator_key)

htlc_addr_str = os.environ.get("BASE_HTLC_CONTRACT")
if not htlc_addr_str:
    print("Error: BASE_HTLC_CONTRACT environment variable is required", file=sys.stderr)
    sys.exit(1)
htlc_addr = Web3.to_checksum_address(htlc_addr_str)

usdc_addr_str = os.environ.get("BASE_USDC_CONTRACT", "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")
usdc_addr = Web3.to_checksum_address(usdc_addr_str)

htlc_abi = [
    {
        "constant": True,
        "inputs": [{"name": "", "type": "bytes32"}],
        "name": "swaps",
        "outputs": [
            {"name": "sender", "type": "address"},
            {"name": "beneficiary", "type": "address"},
            {"name": "token", "type": "address"},
            {"name": "amount", "type": "uint256"},
            {"name": "hash160", "type": "bytes20"},
            {"name": "timelock", "type": "uint256"},
            {"name": "claimed", "type": "bool"},
            {"name": "refunded", "type": "bool"}
        ],
        "payable": False,
        "stateMutability": "view",
        "type": "function"
    },
    {
        "constant": False,
        "inputs": [{"name": "swapId", "type": "bytes32"}],
        "name": "refund",
        "outputs": [],
        "payable": False,
        "stateMutability": "nonpayable",
        "type": "function"
    }
]
erc20_abi = [{"constant":True,"inputs":[{"name":"_owner","type":"address"}],"name":"balanceOf","outputs":[{"name":"balance","type":"uint256"}],"type":"function"}]

htlc_contract = w3.eth.contract(address=htlc_addr, abi=htlc_abi)
usdc_contract = w3.eth.contract(address=usdc_addr, abi=erc20_abi)

target_swap_id = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("TARGET_SWAP_ID")
if not target_swap_id:
    print("Usage: python3 auto_refund.py <0x_SWAP_ID> or set TARGET_SWAP_ID env variable", file=sys.stderr)
    sys.exit(1)

target_clean = target_swap_id[2:] if target_swap_id.startswith("0x") else target_swap_id
target_bytes = bytes.fromhex(target_clean)

print(f"Auto-refund daemon started for Swap 0x{target_clean}")

while True:
    try:
        s = htlc_contract.functions.swaps(target_bytes).call()
        sender, beneficiary, token, amount, h160, timelock, claimed, refunded = s
        if claimed or refunded:
            print(f"Swap already finalized: claimed={claimed}, refunded={refunded}. Exiting daemon.")
            sys.exit(0)
            
        latest_block = w3.eth.get_block('latest')
        curr_ts = latest_block['timestamp']
        diff = timelock - curr_ts
        
        if diff <= 0:
            print(f"Timelock expired (curr_ts={curr_ts}, timelock={timelock})! Executing refund...")
            nonce = w3.eth.get_transaction_count(operator_acc.address, 'pending')
            tx = htlc_contract.functions.refund(target_bytes).build_transaction({
                'from': operator_acc.address,
                'nonce': nonce,
                'gas': 120000,
                'maxFeePerGas': int(w3.eth.gas_price * 1.5),
                'maxPriorityFeePerGas': w3.to_wei(0.01, 'gwei'),
                'chainId': chain_id
            })
            signed_tx = w3.eth.account.sign_transaction(tx, operator_key)
            tx_hash = w3.eth.send_raw_transaction(signed_tx.raw_transaction)
            print(f"Broadcasted refund tx: {tx_hash.hex()}")
            receipt = w3.eth.wait_for_transaction_receipt(tx_hash, timeout=120)
            print(f"Transaction mined in block {receipt.blockNumber}, status={receipt.status}")
            print(f"Contract USDC balance: {usdc_contract.functions.balanceOf(htlc_addr).call() / 1e6}")
            print(f"Operator USDC balance: {usdc_contract.functions.balanceOf(operator_acc.address).call() / 1e6}")
            print("Auto-refund completed successfully.")
            sys.exit(0)
        else:
            # Sleep adaptive: if diff > 60s, sleep 30s; else sleep 5s
            sleep_time = 30 if diff > 60 else 5
            time.sleep(sleep_time)
    except Exception as e:
        print(f"Error in auto-refund loop: {e}", file=sys.stderr)
        time.sleep(10)
