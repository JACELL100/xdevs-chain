// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {XDRC20Bridged} from "./XDRC20Bridged.sol";

/// @title CanonicalTokenFactory - one verified bridged token per origin asset (plan §22.3)
/// @notice Governance deploys bridged XDRC-20 tokens here (CREATE2, salt = originChainId ‖ originToken),
///         so there is exactly one canonical "USDC from chain X" and wallets / explorers can verify any
///         token with `isCanonical(token)` or the `CanonicalTokenCreated` event instead of trusting a
///         name/symbol (which anyone can copy). Not a predeploy: deployed normally with constructor args.
contract CanonicalTokenFactory {
    struct Origin {
        uint256 chainId;
        address token;
    }

    address public governance;
    uint64 public immutable defaultApprovalDuration;
    uint64 public immutable maxApprovalDuration;

    mapping(uint256 chainId => mapping(address originToken => address)) public canonicalToken;
    mapping(address token => Origin) private _origins;
    address[] public allTokens;

    event CanonicalTokenCreated(
        uint256 indexed originChainId,
        address indexed originToken,
        address indexed token,
        address bridge,
        string name,
        string symbol,
        uint8 decimals
    );
    event GovernanceChanged(address governance);

    error NotGovernance(address caller);
    error AlreadyExists(address token);
    error ZeroAddress();

    constructor(address governance_, uint64 defaultDuration, uint64 maxDuration) {
        if (governance_ == address(0)) revert ZeroAddress();
        governance = governance_;
        defaultApprovalDuration = defaultDuration;
        maxApprovalDuration = maxDuration;
    }

    modifier onlyGovernance() {
        if (msg.sender != governance) revert NotGovernance(msg.sender);
        _;
    }

    function setGovernance(address governance_) external onlyGovernance {
        if (governance_ == address(0)) revert ZeroAddress();
        governance = governance_;
        emit GovernanceChanged(governance_);
    }

    /// @notice Deploy the canonical bridged token for (`originChainId`, `originToken`).
    function createCanonicalToken(
        uint256 originChainId,
        address originToken,
        string memory name,
        string memory symbol,
        uint8 decimals,
        address bridge
    ) external onlyGovernance returns (address token) {
        if (bridge == address(0) || originToken == address(0)) revert ZeroAddress();
        address existing = canonicalToken[originChainId][originToken];
        if (existing != address(0)) revert AlreadyExists(existing);
        token = address(
            new XDRC20Bridged{salt: keccak256(abi.encode(originChainId, originToken))}(
                name,
                symbol,
                decimals,
                bridge,
                originChainId,
                originToken,
                defaultApprovalDuration,
                maxApprovalDuration
            )
        );
        canonicalToken[originChainId][originToken] = token;
        _origins[token] = Origin(originChainId, originToken);
        allTokens.push(token);
        emit CanonicalTokenCreated(originChainId, originToken, token, bridge, name, symbol, decimals);
    }

    /// @notice True iff `token` was deployed by this factory.
    function isCanonical(address token) external view returns (bool) {
        return _origins[token].token != address(0);
    }

    function originOf(address token) external view returns (uint256 chainId, address originToken) {
        Origin memory o = _origins[token];
        return (o.chainId, o.token);
    }

    function allTokensLength() external view returns (uint256) {
        return allTokens.length;
    }
}
