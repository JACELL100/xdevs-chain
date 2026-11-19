// ABI fragments of the bridge contracts (contracts/src/bridge), shared by the relayer, the fixture generator and
// the end-to-end test.

import { parseAbi, parseAbiParameters } from "viem";

const BEACON_HEADER = "(uint64 slot, uint64 proposerIndex, bytes32 parentRoot, bytes32 stateRoot, bytes32 bodyRoot)";
const G1 = "(uint256 xHi, uint256 xLo, uint256 yHi, uint256 yLo)";
const UPDATE =
  `(${BEACON_HEADER} attested, ${BEACON_HEADER} finalized, bytes32[] finalityBranch, bytes32 finalizedExecutionBlockHash, ` +
  "bytes32[] executionBranch, bytes32 nextSyncCommitteeRoot, bytes32[] nextSyncCommitteeBranch, bytes32[2] syncCommitteeBits, " +
  "uint256[8] signature, uint64 signatureSlot)";
const NON_SIGNER = `(uint256 index, ${G1} pubkey, bytes32[] proof)`;
const EVENT_PROOF = "(uint256 blockNumber, uint256 txIndex, bytes[] receiptProof, uint256 logIndex)";
const VALIDATOR = "(bytes32 key, uint64 power)";
const SIGNATURE = "(uint256 validatorIndex, bytes32 r, bytes32 s)";
const STAKING_PROOF =
  "(address[] operators, bytes32[] keys, uint256[] powersWei, bytes[] accountProof, bytes[] epochWordProof, bytes[] setHashProof)";

/** abi.encode(Update, NonSigner[]) / abi.encode(G1[]) blobs used by the Foundry fixtures. */
export const updateParams = parseAbiParameters(`${UPDATE} u, ${NON_SIGNER}[] nonSigners`);
export const committeeParams = parseAbiParameters(`${G1}[] keys`);
export const commitParams = parseAbiParameters(
  `uint64 height, uint32 round, bytes32 blockHash, ${VALIDATOR}[] set, ${SIGNATURE}[] sigs`,
);

const HEADER_STORE = [
  "function submitHeader(bytes rlp)",
  "function verifiedRoots(uint256 number) view returns (bytes32 stateRoot, bytes32 receiptsRoot)",
  "function trusted(bytes32 blockHash) view returns (uint256)",
  "function blockHashAt(uint256 number) view returns (bytes32)",
  "function latestNumber() view returns (uint256)",
  "function frozen() view returns (bool)",
] as const;

export const headerStoreAbi = parseAbi(HEADER_STORE);

export const ethereumLightClientAbi = parseAbi([
  ...HEADER_STORE,
  `function submitUpdate(${UPDATE} u, ${NON_SIGNER}[] nonSigners)`,
  `function loadCommittee(uint256 period, ${G1}[] keys)`,
  "function committeeRoot(uint256 period) view returns (bytes32)",
  "function committeeLoaded(uint256 period) view returns (bool)",
  "function finalizedSlot() view returns (uint64)",
  "event FinalizedHeader(uint64 indexed slot, bytes32 indexed beaconRoot, bytes32 executionBlockHash)",
]);

export const xdevsLightClientAbi = parseAbi([
  ...HEADER_STORE,
  `function submitCommit(uint64 height, uint32 round, bytes32 blockHash, ${VALIDATOR}[] set, ${SIGNATURE}[] sigs)`,
  `function submitValidatorSet(uint64 boundary, ${STAKING_PROOF} p)`,
  "function validatorSetBoundary(uint64 height) view returns (uint64)",
  "function validatorSets(uint64 boundary) view returns (bytes32 hash, uint64 totalPower)",
  "function certifiedBlock(uint64 height) view returns (bytes32)",
]);

export const vaultAbi = parseAbi([
  "function lock(address token, uint256 amount, address recipient)",
  `function release(${EVENT_PROOF} p)`,
  "function processed(uint256 nonce) view returns (bool)",
  "function setCounterpart(address counterpart)",
  "event Locked(uint256 indexed nonce, address indexed token, address indexed sender, address recipient, uint256 amount)",
]);

export const xdevsBridgeAbi = parseAbi([
  `function mint(${EVENT_PROOF} p)`,
  "function burn(address token, uint256 amount, address recipient)",
  "function processed(uint256 nonce) view returns (bool)",
  "function setCounterpart(address counterpart)",
  "event BurnedForRelease(uint256 indexed nonce, address indexed originToken, address indexed sender, address recipient, uint256 amount)",
]);
