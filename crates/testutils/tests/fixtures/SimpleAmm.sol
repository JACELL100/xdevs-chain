// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.38;

/// Constant-product AMM between native tXDS and an internal token: the victim market of
/// the Phase 5 sandwich-attack test (crates/testutils/tests/phase5_encrypted_mempool.rs).
/// No fee. Test fixture only.
///
/// Regenerate the creation bytecode embedded in the test with:
///   solc --evm-version osaka --optimize --optimize-runs 200 --bin SimpleAmm.sol
contract SimpleAmm {
    uint256 public reserveNative;
    uint256 public reserveToken;
    mapping(address => uint256) public tokenBalance;

    constructor(uint256 tokenReserve) payable {
        reserveNative = msg.value;
        reserveToken = tokenReserve;
    }

    /// Buy tokens with `msg.value`; reverts below `minOut`.
    function buy(uint256 minOut) external payable returns (uint256 out) {
        out = (reserveToken * msg.value) / (reserveNative + msg.value);
        require(out >= minOut, "slippage");
        reserveNative += msg.value;
        reserveToken -= out;
        tokenBalance[msg.sender] += out;
    }

    /// Sell `amount` tokens for native; reverts below `minOut`.
    function sell(uint256 amount, uint256 minOut) public returns (uint256 out) {
        tokenBalance[msg.sender] -= amount;
        out = (reserveNative * amount) / (reserveToken + amount);
        require(out >= minOut, "slippage");
        reserveToken += amount;
        reserveNative -= out;
        (bool ok,) = msg.sender.call{value: out}("");
        require(ok, "transfer");
    }

    /// Sell the caller's whole token balance.
    function sellAll(uint256 minOut) external returns (uint256) {
        return sell(tokenBalance[msg.sender], minOut);
    }
}
