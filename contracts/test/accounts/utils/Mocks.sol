// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {P256} from "@openzeppelin/contracts/utils/cryptography/P256.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";

/// @notice Test stand-in for the RIP-7212 / EIP-7951 `P256VERIFY` precompile at 0x100 (Foundry's `prague` EVM does
///         not ship it). Same ABI: 160-byte input `h || r || s || x || y`, returns `uint256(1)` when valid and empty
///         data otherwise. Backed by OpenZeppelin's pure-Solidity P-256 verifier.
contract P256VerifierMock {
    fallback(bytes calldata input) external returns (bytes memory) {
        if (input.length != 160) return "";
        (bytes32 h, bytes32 r, bytes32 s, bytes32 x, bytes32 y) =
            abi.decode(input, (bytes32, bytes32, bytes32, bytes32, bytes32));
        if (P256.verifySolidity(h, r, s, x, y)) return abi.encode(uint256(1));
        return "";
    }
}

contract MockERC20 is ERC20 {
    constructor() ERC20("Mock USD", "mUSD") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function decimals() public pure override returns (uint8) {
        return 6;
    }
}

contract MockERC721 is ERC721 {
    constructor() ERC721("Mock NFT", "mNFT") {}

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

/// @notice Simple dApp target for session-key / whitelist tests.
contract Counter {
    uint256 public count;
    mapping(address => uint256) public paid;

    function increment() external {
        ++count;
    }

    function pay() external payable {
        paid[msg.sender] += msg.value;
    }

    function other() external {}
}

/// @notice A second account implementation used to test upgrades.
contract DummyImplementationV2 {
    function version2() external pure returns (uint256) {
        return 2;
    }
}
