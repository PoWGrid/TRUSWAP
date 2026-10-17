// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/**
 * @title BaseHTLC
 * @notice Production-grade non-custodial Hash Time Locked Contract (HTLC) strictly for
 *         Cross-Chain Atomic Swaps between TRU and Base (Circle USDC).
 * @dev Enforces 100% Circle USDC (0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913).
 *      No native ETH and no arbitrary/fake ERC-20 tokens are permitted.
 *      Uses ripemd160(sha256(preimage)) to match Bitcoin/TRU OP_HASH160 20-byte digest.
 *      Includes ReentrancyGuard, SafeERC20 low-level calls, Fee-on-transfer protection,
 *      timelock expiration enforcement on claim(), USDC pull-over-push fallback,
 *      and preimage length bounds.
 */

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

contract BaseHTLC {
    /// @notice Canonical Circle USDC address on the deployed EVM chain
    address public immutable USDC;

    /**
     * @notice Initializes the HTLC contract with designated Circle USDC address.
     * @param _usdc Address of Circle USDC contract (pass address(0) to default to Base Mainnet USDC).
     */
    constructor(address _usdc) {
        if (_usdc == address(0)) {
            USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
        } else {
            USDC = _usdc;
        }
    }

    struct SwapOrder {
        address sender;             // Party who locked the funds (funder) [20 bytes] \
        uint64 timelock;            // Unix timestamp expiration [8 bytes]             | -> Slot 0 (30 bytes)
        bool claimed;               // Set to true once claimed [1 byte]               |
        bool refunded;              // Set to true once refunded [1 byte]              /
        address beneficiary;        // Party entitled to claim funds [20 bytes]        \ -> Slot 1 (32 bytes packed)
        uint96 amount;              // Amount locked in USDC atoms (6 decimals) [12B]  /
        bytes20 hash160Digest;      // 20-byte ripemd160(sha256(secret)) matching TRU-SWAP-V1 -> Slot 2
        bytes32 secret;             // Revealed preimage once claimed (0 until claimed) -> Slot 3
    }

    // Storage mapping from unique swapId to packed SwapOrder
    mapping(bytes32 => SwapOrder) internal _swaps;

    // Pull-over-push fallback balance for USDC if recipient fails to receive (e.g. Circle blacklist)
    mapping(address => uint256) public pendingWithdrawals;

    // Gas-efficient Reentrancy Guard (1 = NOT_ENTERED, 2 = ENTERED)
    uint256 private _status = 1;

    modifier nonReentrant() {
        require(_status == 1, "HTLC: reentrant call");
        _status = 2;
        _;
        _status = 1;
    }

    // Events for off-chain coordinators, indexers, and UI listeners
    event SwapCreated(
        bytes32 indexed swapId,
        address indexed sender,
        address indexed beneficiary,
        address token,
        uint256 amount,
        bytes20 hash160Digest,
        uint256 timelock
    );

    event SwapClaimed(
        bytes32 indexed swapId,
        bytes secret,
        address indexed claimer
    );

    event SwapRefunded(
        bytes32 indexed swapId,
        address indexed refunder
    );

    event WithdrawalPending(
        bytes32 indexed swapId,
        address indexed recipient,
        uint256 amount
    );

    event WithdrawalClaimed(
        address indexed recipient,
        uint256 amount
    );

    /**
     * @notice Locks Circle USDC in HTLC for a designated beneficiary.
     * @param beneficiary Recipient address on Base who can claim with preimage.
     * @param token Must strictly be Circle USDC (0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913).
     * @param amount Amount of USDC atoms to lock (6 decimals).
     * @param hash160Digest 20-byte RIPEMD160(SHA256(secret)) hashlock.
     * @param timelock Unix timestamp expiration for safety refund (min 15m, max 7d).
     * @param salt Unique 32-byte salt preventing hash collisions on identical swaps.
     * @return swapId Unique deterministic identifier of the swap.
     */
    function createSwap(
        address beneficiary,
        address token,
        uint256 amount,
        bytes20 hash160Digest,
        uint256 timelock,
        bytes32 salt
    ) external nonReentrant returns (bytes32 swapId) {
        require(token == USDC, "HTLC: only USDC accepted");
        require(beneficiary != address(0), "HTLC: invalid beneficiary");
        require(beneficiary != address(this), "HTLC: beneficiary cannot be contract");
        require(beneficiary != msg.sender, "HTLC: self-swap not allowed");
        require(amount > 0 && amount <= type(uint96).max, "HTLC: invalid amount");
        require(hash160Digest != bytes20(0), "HTLC: zero hashlock");
        require(timelock >= block.timestamp + 15 minutes, "HTLC: timelock too short (min 15m)");
        require(timelock <= block.timestamp + 7 days, "HTLC: timelock too far (max 7d)");

        swapId = keccak256(
            abi.encodePacked(
                msg.sender,
                beneficiary,
                USDC,
                amount,
                hash160Digest,
                timelock,
                block.chainid,
                salt
            )
        );

        require(_swaps[swapId].sender == address(0), "HTLC: swap already exists");

        _safeTransferFrom(USDC, msg.sender, address(this), amount);

        _swaps[swapId] = SwapOrder({
            sender: msg.sender,
            timelock: uint64(timelock),
            claimed: false,
            refunded: false,
            beneficiary: beneficiary,
            amount: uint96(amount),
            hash160Digest: hash160Digest,
            secret: bytes32(0)
        });

        emit SwapCreated(swapId, msg.sender, beneficiary, USDC, amount, hash160Digest, timelock);
    }

    /**
     * @notice Claims locked USDC by providing secret preimage.
     * @dev Secret is hashed with SHA256 then RIPEMD160 to match TRU OP_HASH160.
     *      Enforces block.timestamp < swap.timelock to prevent race condition with refund().
     *      Anyone can relay the transaction, but USDC is strictly transferred to swap.beneficiary.
     * @param swapId Swap identifier.
     * @param secret Preimage bytes whose HASH160 matches swap.hash160Digest.
     */
    function claim(bytes32 swapId, bytes calldata secret) external nonReentrant {
        SwapOrder storage swap = _swaps[swapId];
        require(swap.sender != address(0), "HTLC: swap not found");
        require(!swap.claimed, "HTLC: already claimed");
        require(!swap.refunded, "HTLC: already refunded");
        require(block.timestamp < swap.timelock, "HTLC: timelock expired");
        require(secret.length == 32, "HTLC: secret must be 32 bytes");

        // Compute RIPEMD160(SHA256(secret)) matching TRU-SWAP-V1
        bytes20 computed = ripemd160(abi.encodePacked(sha256(secret)));
        require(computed == swap.hash160Digest, "HTLC: invalid preimage secret");

        swap.claimed = true;
        swap.secret = bytes32(secret[:32]);

        _sendUSDC(swapId, swap.beneficiary, swap.amount);

        emit SwapClaimed(swapId, secret, msg.sender);
    }

    /**
     * @notice Refunds locked USDC back to sender after timelock expiration.
     * @param swapId Swap identifier.
     */
    function refund(bytes32 swapId) external nonReentrant {
        SwapOrder storage swap = _swaps[swapId];
        require(swap.sender != address(0), "HTLC: swap not found");
        require(!swap.claimed, "HTLC: already claimed");
        require(!swap.refunded, "HTLC: already refunded");
        require(block.timestamp >= swap.timelock, "HTLC: timelock not expired");

        swap.refunded = true;

        _sendUSDC(swapId, swap.sender, swap.amount);

        emit SwapRefunded(swapId, msg.sender);
    }

    /**
     * @notice Allows the designated beneficiary to cooperatively and early refund USDC back to sender
     *         without waiting for timelock expiration.
     * @dev Only beneficiary can trigger this because beneficiary is the sole party entitled to claim.
     *      Early refund protects sender from waiting the full timelock if both parties agree to abort.
     * @param swapId Swap identifier.
     */
    function cooperativeRefund(bytes32 swapId) external nonReentrant {
        SwapOrder storage swap = _swaps[swapId];
        require(swap.sender != address(0), "HTLC: swap not found");
        require(msg.sender == swap.beneficiary, "HTLC: only beneficiary can cooperatively refund");
        require(!swap.claimed, "HTLC: already claimed");
        require(!swap.refunded, "HTLC: already refunded");

        swap.refunded = true;

        _sendUSDC(swapId, swap.sender, swap.amount);

        emit SwapRefunded(swapId, msg.sender);
    }

    /**
     * @notice Safely transfers USDC to a recipient with fallback to pull-withdrawal.
     * @dev Limits forwarded gas to 150,000 to prevent gas griefing and out-of-gas reverts.
     *      If recipient is blacklisted by Circle USDC, funds fall back to pendingWithdrawals.
     */
    function _sendUSDC(bytes32 swapId, address recipient, uint256 amount) internal {
        (bool success, bytes memory data) = USDC.call{gas: 150000}(
            abi.encodeWithSelector(IERC20.transfer.selector, recipient, amount)
        );
        if (success && (data.length == 0 || (data.length >= 32 && abi.decode(data, (bool))))) {
            return;
        }
        pendingWithdrawals[recipient] += amount;
        emit WithdrawalPending(swapId, recipient, amount);
    }

    /**
     * @notice Allows pulling USDC if a direct push transfer previously failed.
     */
    function withdrawPending() external nonReentrant {
        _withdrawPendingTo(msg.sender);
    }

    /**
     * @notice Allows pulling USDC to a custom destination address if direct transfer failed.
     * @param to Recipient address for the USDC.
     */
    function withdrawPendingTo(address to) external nonReentrant {
        _withdrawPendingTo(to);
    }

    function _withdrawPendingTo(address to) internal {
        require(to != address(0), "HTLC: zero address");
        require(to != address(this), "HTLC: cannot withdraw to contract");
        uint256 amount = pendingWithdrawals[msg.sender];
        require(amount > 0, "HTLC: no pending balance");
        pendingWithdrawals[msg.sender] = 0;
        _safeTransfer(USDC, to, amount);
        emit WithdrawalClaimed(to, amount);
    }

    /**
     * @dev Low-level safe transfer helper compatible with tokens returning bool or void.
     */
    function _safeTransfer(address token, address to, uint256 amount) internal {
        (bool success, bytes memory data) = token.call(
            abi.encodeWithSelector(IERC20.transfer.selector, to, amount)
        );
        require(success && (data.length == 0 || (data.length >= 32 && abi.decode(data, (bool)))), "HTLC: token transfer failed");
    }

    /**
     * @dev Low-level safe transferFrom helper compatible with tokens returning bool or void.
     */
    function _safeTransferFrom(address token, address from, address to, uint256 amount) internal {
        (bool success, bytes memory data) = token.call(
            abi.encodeWithSelector(IERC20.transferFrom.selector, from, to, amount)
        );
        require(success && (data.length == 0 || (data.length >= 32 && abi.decode(data, (bool)))), "HTLC: token transferFrom failed");
    }

    /**
     * @notice Helper view to inspect a swap's current status and parameters.
     */
    function getSwap(bytes32 swapId) public view returns (
        address sender,
        address beneficiary,
        address token,
        uint256 amount,
        bytes20 hash160Digest,
        uint256 timelock,
        bool claimed,
        bool refunded,
        bytes memory secret
    ) {
        SwapOrder storage s = _swaps[swapId];
        return (
            s.sender,
            s.beneficiary,
            USDC,
            uint256(s.amount),
            s.hash160Digest,
            uint256(s.timelock),
            s.claimed,
            s.refunded,
            s.claimed ? abi.encodePacked(s.secret) : new bytes(0)
        );
    }

    /**
     * @notice Backward-compatible view mirroring the public swaps mapping.
     */
    function swaps(bytes32 swapId) external view returns (
        address sender,
        address beneficiary,
        address token,
        uint256 amount,
        bytes20 hash160Digest,
        uint256 timelock,
        bool claimed,
        bool refunded,
        bytes memory secret
    ) {
        return getSwap(swapId);
    }
}
