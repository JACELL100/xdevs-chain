// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";
import {Create2} from "@openzeppelin/contracts/utils/Create2.sol";

import {Predeploys} from "../common/Predeploys.sol";
import {AccountExtension} from "./AccountExtension.sol";
import {IAccountExtension} from "./interfaces/IXdevsSmartAccount.sol";
import {InitParams, Signer, VaultPolicy, TokenLimit} from "./libraries/AccountTypes.sol";

/// @title SmartAccountFactory
/// @notice Deterministic (CREATE2) creation of xDEVS smart accounts as ERC-1967 proxies pointing at the predeployed
///         implementation (plan §16.1, §18). Passkey-only accounts are supported (no seed phrase).
/// @dev Predeployed at `Predeploys.SMART_ACCOUNT_FACTORY` with no constructor, no storage and NO init params; the
///      implementation address is the constant `Predeploys.SMART_ACCOUNT_IMPL`. The account address commits to the
///      full init data (signers, guardians, policy) and the salt, so nobody can front-run a deployment with
///      different owners. Usable as an ERC-4337 factory: `initCode = factory || createAccount…(…)`.
///
///      This predeploy also hosts `AccountExtension`, the cold-path logic every xDEVS account reaches through its
///      fallback with DELEGATECALL (see `AccountConstants.ACCOUNT_EXTENSION`). The factory entry points refuse to run
///      in an account's context (`onlyFactory`), and the extension functions are inert when called on the factory.
contract SmartAccountFactory is AccountExtension {
    event AccountCreated(address indexed account, bytes32 indexed salt);

    error NotFactoryContext();

    modifier onlyFactory() {
        if (address(this) != Predeploys.SMART_ACCOUNT_FACTORY) revert NotFactoryContext();
        _;
    }

    /// @notice The implementation every account proxy points to.
    function accountImplementation() public pure returns (address) {
        return Predeploys.SMART_ACCOUNT_IMPL;
    }

    /// @notice Creates (or returns the existing) account with `signers` and default settings: no guardians, vault
    ///         disabled (opt-in; 24 h delay once enabled), 48 h recovery delay.
    function createAccount(Signer[] calldata signers, bytes32 salt) external onlyFactory returns (address) {
        return _create(_initData(defaultParams(signers)), salt);
    }

    /// @notice Creates (or returns the existing) account with a full configuration.
    function createAccountWithConfig(InitParams calldata params, bytes32 salt)
        external
        onlyFactory
        returns (address)
    {
        return _create(_initData(params), salt);
    }

    /// @notice Counterfactual address of `createAccount(signers, salt)`.
    function getAddress(Signer[] calldata signers, bytes32 salt) external view onlyFactory returns (address) {
        return _predict(_initData(defaultParams(signers)), salt);
    }

    /// @notice Counterfactual address of `createAccountWithConfig(params, salt)`.
    function getAddressWithConfig(InitParams calldata params, bytes32 salt)
        external
        view
        onlyFactory
        returns (address)
    {
        return _predict(_initData(params), salt);
    }

    function defaultParams(Signer[] calldata signers) public pure returns (InitParams memory p) {
        p.signers = signers;
        p.guardians = new address[](0);
        p.whitelist = new address[](0);
        p.tokenLimits = new TokenLimit[](0);
        p.policy = VaultPolicy({
            enabled: false,
            blockUnlimitedApprovals: false,
            allowInstantSignatures: false,
            delay: 0,
            nativeThreshold: 0,
            dailyLimit: 0,
            dailyGasBudget: 0
        });
    }

    function _initData(InitParams memory params) private pure returns (bytes memory) {
        return abi.encodeCall(IAccountExtension.initialize, (params));
    }

    function _bytecode(bytes memory initData) private pure returns (bytes memory) {
        return abi.encodePacked(
            type(ERC1967Proxy).creationCode, abi.encode(Predeploys.SMART_ACCOUNT_IMPL, initData)
        );
    }

    function _predict(bytes memory initData, bytes32 salt) private view returns (address) {
        return Create2.computeAddress(salt, keccak256(_bytecode(initData)));
    }

    function _create(bytes memory initData, bytes32 salt) private returns (address account) {
        bytes memory code = _bytecode(initData);
        account = Create2.computeAddress(salt, keccak256(code));
        if (account.code.length != 0) return account;
        account = Create2.deploy(0, salt, code);
        emit AccountCreated(account, salt);
    }
}
