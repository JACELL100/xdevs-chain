// ABIs of the xDEVS contracts the wallet talks to (subset). Sources: contracts/src/accounts/**, contracts/src/periphery/**.
import { parseAbi } from 'viem'

const SIGNER = '(uint8 kind, bytes32 x, bytes32 y)'
const POLICY =
  '(bool enabled, bool blockUnlimitedApprovals, bool allowInstantSignatures, uint64 delay, uint128 nativeThreshold, uint128 dailyLimit, uint128 dailyGasBudget)'

/** InitParams as a JSON ABI parameter (too deep for viem's type-level human-readable parser). */
const INIT_PARAM = {
  name: 'params',
  type: 'tuple',
  components: [
    { name: 'signers', type: 'tuple[]', components: [{ name: 'kind', type: 'uint8' }, { name: 'x', type: 'bytes32' }, { name: 'y', type: 'bytes32' }] },
    { name: 'guardians', type: 'address[]' },
    { name: 'guardianThreshold', type: 'uint8' },
    { name: 'recoveryDelay', type: 'uint64' },
    {
      name: 'policy',
      type: 'tuple',
      components: [
        { name: 'enabled', type: 'bool' },
        { name: 'blockUnlimitedApprovals', type: 'bool' },
        { name: 'allowInstantSignatures', type: 'bool' },
        { name: 'delay', type: 'uint64' },
        { name: 'nativeThreshold', type: 'uint128' },
        { name: 'dailyLimit', type: 'uint128' },
        { name: 'dailyGasBudget', type: 'uint128' },
      ],
    },
    { name: 'whitelist', type: 'address[]' },
    { name: 'tokenLimits', type: 'tuple[]', components: [{ name: 'token', type: 'address' }, { name: 'perTx', type: 'uint128' }, { name: 'daily', type: 'uint128' }] },
  ],
} as const
const SALT = { name: 'salt', type: 'bytes32' } as const
const CALL = '(address target, uint256 value, bytes data)'

/** XdevsSmartAccount core + AccountExtension (served through the account's fallback) + IGuardianFreeze. */
export const accountAbi = [
  { type: 'function', name: 'initialize', stateMutability: 'nonpayable', inputs: [INIT_PARAM], outputs: [] },
  ...parseAbi([
  // execution / ERC-4337
  'function execute(address target, uint256 value, bytes data) returns (uint256)',
  `function executeBatch(${CALL}[] calls) returns (uint256)`,
  'function execute(uint256 id)',
  'function cancel(uint256 id)',
  'function getNonce() view returns (uint256)',
  'function isValidSignature(bytes32 hash, bytes signature) view returns (bytes4)',
  'function replaySafeHash(bytes32 hash) view returns (bytes32)',
  // init
  'function isInitialized() view returns (bool)',
  // signers
  `function getSigners() view returns (${SIGNER}[])`,
  `function addSigner(${SIGNER} s)`,
  'function removeSigner(bytes32 id)',
  // vault
  `function vaultPolicy() view returns (${POLICY})`,
  `function setVaultPolicy(${POLICY} p)`,
  'function setPolicy(uint256 nativeThreshold, uint64 delaySeconds, address[] whitelist)',
  'function setWhitelisted(address who, bool trusted)',
  'function setTokenLimit(address token, uint128 perTx, uint128 daily)',
  'function getWhitelist() view returns (address[])',
  'function isWhitelisted(address who) view returns (bool)',
  'function tokenLimit(address token) view returns (uint128 perTx, uint128 daily, uint64 day, uint128 spent)',
  `function getAction(uint256 id) view returns (uint64 executeAfter, uint8 status, bool valid, ${CALL}[] calls)`,
  'function lastActionId() view returns (uint256)',
  'function spentToday() view returns (uint128)',
  // recovery
  'function getGuardians() view returns (address[] guardians, uint8 threshold)',
  'function setGuardians(address[] guardians, uint8 threshold)',
  'function recoveryDelay() view returns (uint64)',
  'function recoveryNonce() view returns (uint64)',
  `function getRecoveryProposal(${SIGNER}[] newSigners) view returns (bytes32 id, uint8 approvals, uint64 readyAt)`,
  `function approveRecovery(${SIGNER}[] newSigners)`,
  `function executeRecovery(${SIGNER}[] newSigners)`,
  'function cancelRecovery()',
  // freeze
  'function freeze()',
  'function unfreeze()',
  'function isFrozen() view returns (bool)',
  'function freezeState() view returns (uint64 frozenUntil, uint64 freezeDuration, uint64 nextFreezeAllowedAt)',
  'function guardianVotes(uint8 kind, uint256 actionId) view returns (bytes32 voteId, uint8 votes)',
  // XIP-0001 EOA lock
  'function enableEoaLock()',
  'function disableEoaLock()',
  'function eoaLockState() view returns (uint256)',
  'function refreshEoaLock()',
  'function isEoaKeyActive() view returns (bool)',
  // events
  'event VaultActionQueued(uint256 indexed id, address target, uint256 value, bytes data, uint64 executeAfter)',
  'event VaultActionCancelled(uint256 indexed id, address by)',
  'event VaultActionExecuted(uint256 indexed id)',
  'event GuardianVoted(bytes32 indexed voteId, address indexed guardian, uint8 votes)',
  'event AccountFrozen(uint64 frozenUntil)',
  'event AccountUnfrozen()',
  `event RecoveryApproved(bytes32 indexed id, address indexed guardian, uint8 approvals, ${SIGNER}[] newSigners)`,
  'event RecoveryReady(bytes32 indexed id, uint64 readyAt)',
  'event RecoveryExecuted(bytes32 indexed id)',
  'event RecoveryCancelled(uint64 newNonce)',
  'event EoaLockUpdated(uint256 state)',
  // errors (IAccountEvents)
  'error Unauthorized()',
  'error InvalidParameter()',
  'error AlreadyInitialized()',
  'error InvalidSigner()',
  'error SignerAlreadyExists(bytes32 id)',
  'error SignerNotFound(bytes32 id)',
  'error TooManySigners()',
  'error LastSigner()',
  'error ActionNotQueued(uint256 id)',
  'error ActionNotReady(uint256 id, uint64 executeAfter)',
  'error ActionExpired(uint256 id)',
  'error ActionInvalidated(uint256 id)',
  'error UnlimitedApprovalBlocked(address spender)',
  'error SessionActionNeedsDelay()',
  'error TooManyWhitelisted()',
  'error NotGuardian()',
  'error AlreadyApproved()',
  'error RecoveryNotReady()',
  'error InvalidGuardians()',
  'error AccountIsFrozen()',
  'error NotFrozen()',
  'error FreezeCooldown(uint64 until)',
  'error NotDelegatedEoa()',
  'error VaultDisabled()',
  'error EoaKeyInactive()',
  'error NotProxy()',
  'error InvalidImplementation()',
  ]),
] as const

export const factoryAbi = [
  { type: 'function', name: 'getAddressWithConfig', stateMutability: 'view', inputs: [INIT_PARAM, SALT], outputs: [{ name: '', type: 'address' }] },
  { type: 'function', name: 'createAccountWithConfig', stateMutability: 'nonpayable', inputs: [INIT_PARAM, SALT], outputs: [{ name: '', type: 'address' }] },
  ...parseAbi([
    `function getAddress(${SIGNER}[] signers, bytes32 salt) view returns (address)`,
    `function createAccount(${SIGNER}[] signers, bytes32 salt) returns (address)`,
  ]),
] as const

export const entryPointAbi = parseAbi([
  'function getNonce(address sender, uint192 key) view returns (uint256)',
  'function balanceOf(address account) view returns (uint256)',
  'event UserOperationEvent(bytes32 indexed userOpHash, address indexed sender, address indexed paymaster, uint256 nonce, bool success, uint256 actualGasCost, uint256 actualGasUsed)',
])

export const paymasterAbi = parseAbi([
  'function sponsorInfo(address sponsor) view returns ((uint128 balance, uint128 maxCostPerOp, uint128 perAccountCap, bool active, bool openToAll, address signer))',
  'function accountSpent(address sponsor, address account) view returns (uint256)',
  'function isAllowlisted(address sponsor, address account) view returns (bool)',
])

export const xnsAbi = parseAbi([
  'function resolveAddr(bytes32 node) view returns (address)',
  'function primaryName(address addr) view returns (string)',
  'function available(string label) view returns (bool)',
  'function register(string label, address owner, uint256 duration) payable returns (bytes32, uint256)',
  'function rentPrice(uint256 duration) view returns (uint256)',
])

export const erc20Abi = parseAbi([
  'function name() view returns (string)',
  'function symbol() view returns (string)',
  'function decimals() view returns (uint8)',
  'function balanceOf(address) view returns (uint256)',
  'function allowance(address owner, address spender) view returns (uint256)',
  'function approve(address spender, uint256 amount) returns (bool)',
  'function increaseAllowance(address spender, uint256 addedValue) returns (bool)',
  'function transfer(address to, uint256 amount) returns (bool)',
  'function transferFrom(address from, address to, uint256 amount) returns (bool)',
  'event Approval(address indexed owner, address indexed spender, uint256 value)',
  'event Transfer(address indexed from, address indexed to, uint256 value)',
])

/** XDRC-20 extensions (expiring approvals, O(1) revoke-all). */
export const xdrc20Abi = parseAbi([
  'function approve(address spender, uint256 value, uint64 expiry) returns (bool)',
  'function allowanceExpiry(address owner, address spender) view returns (uint64)',
  'function revokeAll()',
  'function approvalEpoch(address owner) view returns (uint32)',
  'function defaultApprovalDuration() view returns (uint64)',
  'event ApprovalWithExpiry(address indexed owner, address indexed spender, uint256 value, uint64 expiry)',
  'event AllApprovalsRevoked(address indexed owner, uint32 newEpoch)',
])

export const nftAbi = parseAbi([
  'function setApprovalForAll(address operator, bool approved)',
  'function safeTransferFrom(address from, address to, uint256 tokenId)',
  'function transferOwnership(address newOwner)',
])

export const permit2Abi = parseAbi([
  'function approve(address token, address spender, uint160 amount, uint48 expiration)',
  'function allowance(address user, address token, address spender) view returns (uint160 amount, uint48 expiration, uint48 nonce)',
  'function lockdown((address token, address spender)[] approvals)',
  'function revokeAll()',
  'function epochOf(address user) view returns (uint48)',
  'event Approval(address indexed owner, address indexed token, address indexed spender, uint160 amount, uint48 expiration)',
  'event Permit(address indexed owner, address indexed token, address indexed spender, uint160 amount, uint48 expiration, uint48 nonce)',
  'event Lockdown(address indexed owner, address token, address spender)',
  'event AllRevoked(address indexed owner, uint48 newEpoch)',
])
