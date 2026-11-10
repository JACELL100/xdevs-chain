// Minimal ABIs: EntryPoint v0.9 (the parts a bundler needs) and the dev gas meter.
import { parseAbi } from 'viem'

const PACKED = '(address sender, uint256 nonce, bytes initCode, bytes callData, bytes32 accountGasLimits, uint256 preVerificationGas, bytes32 gasFees, bytes paymasterAndData, bytes signature)'

export const entryPointAbi = parseAbi([
  `function handleOps(${PACKED}[] ops, address beneficiary)`,
  `function getUserOpHash(${PACKED} userOp) view returns (bytes32)`,
  'function getNonce(address sender, uint192 key) view returns (uint256)',
  'function balanceOf(address account) view returns (uint256)',
  'function depositTo(address account) payable',
  'function senderCreator() view returns (address)',
  'function delegateAndRevert(address target, bytes data)',
  'function getDepositInfo(address account) view returns ((uint256 deposit, bool staked, uint112 stake, uint32 unstakeDelaySec, uint48 withdrawTime) info)',
  'event UserOperationEvent(bytes32 indexed userOpHash, address indexed sender, address indexed paymaster, uint256 nonce, bool success, uint256 actualGasCost, uint256 actualGasUsed)',
  'event UserOperationRevertReason(bytes32 indexed userOpHash, address indexed sender, uint256 nonce, bytes revertReason)',
  'event PostOpRevertReason(bytes32 indexed userOpHash, address indexed sender, uint256 nonce, bytes revertReason)',
  'event UserOperationPrefundTooLow(bytes32 indexed userOpHash, address indexed sender, uint256 nonce)',
  'event AccountDeployed(bytes32 indexed userOpHash, address indexed sender, address factory, address paymaster)',
  'event BeforeExecution()',
  'error FailedOp(uint256 opIndex, string reason)',
  'error FailedOpWithRevert(uint256 opIndex, string reason, bytes inner)',
  'error PostOpReverted(bytes returnData)',
  'error SignatureValidationFailed(address aggregator)',
  'error SenderAddressResult(address sender)',
  'error DelegateAndRevert(bool success, bytes ret)',
  'error InvalidBeneficiary(address beneficiary)',
  'error FailedSendToBeneficiary(address beneficiary, uint256 amount, bytes revertData)',
  'error InvalidPaymaster(address paymaster)',
  'error InvalidPaymasterData(uint256 paymasterAndDataLength)',
  'error Reentrancy()',
])

export const gasMeterAbi = parseAbi([
  `function measure(${PACKED} op, uint256 maxCost)`,
  'error Measurement(uint256 deployGas, uint256 validationGas, uint256 validationData, uint256 paymasterValidationGas, uint256 paymasterValidationData, uint256 postOpGas, bool callSuccess, uint256 callGas, bytes callResult)',
  'error StepFailed(uint8 step, bytes reason)',
])

/** Arachnid deterministic-deployment proxy (in the xDEVS dev genesis and in anvil by default). */
export const CREATE2_DEPLOYER = '0x4e59b44847b379578588920cA78FbF26c0B4956C' as const
