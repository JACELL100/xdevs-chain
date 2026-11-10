// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.28;

/// Self-contained copies of the ERC-4337 v0.9 types this helper needs (no imports, so it compiles on its own).
struct PackedUserOperation {
    address sender;
    uint256 nonce;
    bytes initCode;
    bytes callData;
    bytes32 accountGasLimits;
    uint256 preVerificationGas;
    bytes32 gasFees;
    bytes paymasterAndData;
    bytes signature;
}

interface IEntryPointLite {
    function senderCreator() external view returns (address);
    function getUserOpHash(PackedUserOperation calldata userOp) external view returns (bytes32);
}

interface ISenderCreatorLite {
    function createSender(bytes calldata initCode) external returns (address);
}

interface IAccountLite {
    function validateUserOp(PackedUserOperation calldata userOp, bytes32 userOpHash, uint256 missingAccountFunds)
        external
        returns (uint256);
}

interface IAccountExecuteLite {
    function executeUserOp(PackedUserOperation calldata userOp, bytes32 userOpHash) external;
}

interface IPaymasterLite {
    function validatePaymasterUserOp(PackedUserOperation calldata userOp, bytes32 userOpHash, uint256 maxCost)
        external
        returns (bytes memory context, uint256 validationData);
    function postOp(uint8 mode, bytes calldata context, uint256 actualGasCost, uint256 actualUserOpFeePerGas)
        external;
}

/// @title XdevsBundlerGasMeter (DEV BUNDLER ONLY)
/// @notice Gas measurement for the xDEVS dev bundler on nodes WITHOUT `eth_call` state overrides (the xDEVS node
///         rejects them). The bundler never calls `measure` directly: it calls
///         `EntryPoint.delegateAndRevert(meter, abi.encodeCall(measure, (op, maxCost)))` inside an `eth_call`, so this
///         code runs with `address(this) == EntryPoint`. Every external call it makes therefore comes FROM the
///         EntryPoint, exactly like during `handleOps`: SenderCreator deploys the account, the account validates and
///         executes, the paymaster validates and runs `postOp`. The result is always returned as a revert
///         (`Measurement`), and nothing persists because the whole thing is an `eth_call`.
contract XdevsBundlerGasMeter {
    error Measurement(
        uint256 deployGas,
        uint256 validationGas,
        uint256 validationData,
        uint256 paymasterValidationGas,
        uint256 paymasterValidationData,
        uint256 postOpGas,
        bool callSuccess,
        uint256 callGas,
        bytes callResult
    );
    error StepFailed(uint8 step, bytes reason);

    uint8 internal constant STEP_DEPLOY = 1;
    uint8 internal constant STEP_VALIDATE = 2;
    uint8 internal constant STEP_PAYMASTER = 3;

    bytes4 internal constant EXECUTE_USER_OP = 0x8dd7712f;

    struct Result {
        uint256 deployGas;
        uint256 validationGas;
        uint256 validationData;
        uint256 paymasterValidationGas;
        uint256 paymasterValidationData;
        uint256 postOpGas;
        bool callSuccess;
        uint256 callGas;
        bytes callResult;
    }

    function measure(PackedUserOperation calldata op, uint256 maxCost) external {
        Result memory r;
        r.deployGas = _deploy(op);
        bytes32 h = IEntryPointLite(address(this)).getUserOpHash(op);
        bool hasPaymaster = op.paymasterAndData.length >= 20;
        _validate(op, h, hasPaymaster ? 0 : maxCost, r);
        bytes memory context;
        if (hasPaymaster) context = _validatePaymaster(op, h, maxCost, r);
        _execute(op, h, r);
        if (hasPaymaster && context.length != 0) _postOp(op, context, maxCost, r);
        revert Measurement(
            r.deployGas,
            r.validationGas,
            r.validationData,
            r.paymasterValidationGas,
            r.paymasterValidationData,
            r.postOpGas,
            r.callSuccess,
            r.callGas,
            r.callResult
        );
    }

    function _validate(PackedUserOperation calldata op, bytes32 h, uint256 missing, Result memory r) private {
        uint256 g = gasleft();
        (bool ok, bytes memory ret) =
            op.sender.call(abi.encodeCall(IAccountLite.validateUserOp, (op, h, missing)));
        r.validationGas = g - gasleft();
        if (!ok || ret.length < 32) revert StepFailed(STEP_VALIDATE, ret);
        r.validationData = abi.decode(ret, (uint256));
    }

    function _validatePaymaster(PackedUserOperation calldata op, bytes32 h, uint256 maxCost, Result memory r)
        private
        returns (bytes memory context)
    {
        address paymaster = address(bytes20(op.paymasterAndData[:20]));
        uint256 g = gasleft();
        (bool ok, bytes memory ret) =
            paymaster.call(abi.encodeCall(IPaymasterLite.validatePaymasterUserOp, (op, h, maxCost)));
        r.paymasterValidationGas = g - gasleft();
        if (!ok) revert StepFailed(STEP_PAYMASTER, ret);
        (context, r.paymasterValidationData) = abi.decode(ret, (bytes, uint256));
    }

    function _execute(PackedUserOperation calldata op, bytes32 h, Result memory r) private {
        bytes memory callData = op.callData;
        r.callSuccess = true;
        if (callData.length == 0) return;
        if (callData.length >= 4 && bytes4(op.callData[:4]) == EXECUTE_USER_OP) {
            callData = abi.encodeCall(IAccountExecuteLite.executeUserOp, (op, h));
        }
        uint256 g = gasleft();
        (bool ok, bytes memory ret) = op.sender.call(callData);
        r.callGas = g - gasleft();
        r.callSuccess = ok;
        if (ret.length > 1024) {
            assembly ("memory-safe") {
                mstore(ret, 1024)
            }
        }
        r.callResult = ret;
    }

    function _postOp(PackedUserOperation calldata op, bytes memory context, uint256 maxCost, Result memory r)
        private
    {
        address paymaster = address(bytes20(op.paymasterAndData[:20]));
        uint256 g = gasleft();
        (bool ok,) = paymaster.call(
            abi.encodeCall(IPaymasterLite.postOp, (r.callSuccess ? 0 : 1, context, maxCost / 2, 1))
        );
        r.postOpGas = g - gasleft();
        (ok);
    }

    function _deploy(PackedUserOperation calldata op) private returns (uint256 used) {
        if (op.initCode.length < 20 || op.sender.code.length != 0) return 0;
        // EIP-7702 "initCode" marker (0x7702...) is not a factory call: nothing to deploy here.
        if (bytes2(op.initCode[:2]) == 0x7702) return 0;
        address creator = IEntryPointLite(address(this)).senderCreator();
        uint256 g = gasleft();
        try ISenderCreatorLite(creator).createSender(op.initCode) returns (address created) {
            used = g - gasleft();
            if (created != op.sender) revert StepFailed(STEP_DEPLOY, abi.encodePacked("sender mismatch"));
        } catch (bytes memory reason) {
            revert StepFailed(STEP_DEPLOY, reason);
        }
    }
}
