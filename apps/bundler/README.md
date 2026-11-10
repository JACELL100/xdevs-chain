# xDEVS dev bundler (ERC-4337 v0.9) — DEV ONLY

A minimal, **single-operator** ERC-4337 bundler for xDEVS dev chains (`xdevs run --dev`, anvil). It exists so the
reference wallet (`apps/wallet`) and tests can send UserOperations to the canonical EntryPoint v0.9
(`0x433709009B8330FDa32311DF1C2AFA402eD8D009`). **It is not a production bundler**:

- no alt-mempool, no P2P, no ERC-7562 opcode / storage-access rules, no reputation or throttling;
- one UserOperation per bundle, bundles strictly sequential (the next op is simulated only after the previous bundle
  is mined), so a second op from the same sender must wait for the first receipt;
- a single funded key (`BUNDLER_PRIVATE_KEY`) pays every bundle and receives the fees;
- in-memory state (receipts of ops it did not submit are still found through `UserOperationEvent` logs, last ~9,000 blocks).

Run it on localhost only. Never expose it to the internet: anyone who can reach it can make the operator key spend gas.

## Methods

| Method | Notes |
|---|---|
| `eth_chainId`, `eth_supportedEntryPoints` | |
| `eth_estimateUserOperationGas(op, ep)` | Gas fields may be omitted. Send a **dummy signature of the real shape** (signature failures are ignored while estimating). State overrides are not supported. Returns `preVerificationGas`, `verificationGasLimit`, `callGasLimit` (+ `paymasterVerificationGasLimit`, `paymasterPostOpGasLimit` with a paymaster). |
| `eth_sendUserOperation(op, ep)` | Validates by simulating `handleOps([op])` with `eth_call` from the operator (EntryPoint `FailedOp` → ERC-7769 codes: `-32500` AA1x/AA2x, `-32501` AA3x, `-32503` AA22/AA32, `-32507` AA24/AA34), checks `maxFeePerGas ≥ base fee`, then submits `handleOps` with the op's own fee caps. Returns the userOpHash. |
| `eth_getUserOperationReceipt(hash)` | `{ userOpHash, entryPoint, sender, nonce, paymaster, actualGasCost, actualGasUsed, success, reason, logs, receipt }` (one op per bundle: `logs` = all logs of the transaction). |
| `eth_getUserOperationByHash(hash)` | `{ userOperation, entryPoint, transactionHash, blockHash, blockNumber }`. |

UserOperations use the standard v0.7+/v0.9 JSON shape (`factory`/`factoryData`, `paymaster`/`paymaster…GasLimit`/`paymasterData`).

### How estimation works without state overrides

The xDEVS node rejects `eth_call` state overrides, so the usual `EntryPointSimulations` override trick is not
available. Instead the bundler deploys a tiny helper, `contracts/XdevsBundlerGasMeter.sol` (CREATE2 through
`0x4e59b448…`, which the dev genesis and anvil both have; plain CREATE otherwise), and calls
`EntryPoint.delegateAndRevert(meter, measure(op, maxCost))` in an `eth_call`. The meter runs **inside the EntryPoint**,
so every call it makes has `msg.sender == EntryPoint`: SenderCreator deploys the account, the account validates and
executes, the paymaster validates and runs `postOp`. It measures each step and reverts with the numbers; nothing
persists. Margins: +20 % and a fixed amount (passkey dummy signatures skip SHA-256 / P256VERIFY work). The bytecode is
committed in `src/gas-meter-artifact.ts`; `npm run build:meter` regenerates it with Foundry.

## Run

```bash
cd apps/bundler && npm install
# 1. a dev chain with the predeploys, a paymaster sponsor and a test token (see below)
npm run devnet                      # xDEVS node if its binary is found (XDEVS_BIN), else anvil
# 2. the bundler (key = a funded dev key; the devnet script prints the exact command)
BUNDLER_PRIVATE_KEY=0x5de4…365a RPC_URL=http://127.0.0.1:8545 npm start   # listens on 127.0.0.1:4337
```

Environment: `BUNDLER_PRIVATE_KEY` (required), `RPC_URL` (default `http://127.0.0.1:8545`), `PORT` (4337), `HOST`
(127.0.0.1), `ENTRY_POINT` (canonical v0.9), `BENEFICIARY` (operator), `QUIET`.

### `npm run devnet` (`scripts/devnet.ts`)

- `--chain xdevs`: starts `xdevs run --dev --genesis deploy/configs/dev-genesis-with-predeploys.json --block-time 1s` in a
  temporary data dir (binary from `XDEVS_BIN`, `CARGO_TARGET_DIR`, `target/{debug,release}` or `~/.xdevs-target`).
  That genesis already contains the smart-account implementation, factory, paymaster, EntryPoint v0.9 + SenderCreator,
  XNS, Permit2Expiring, WXDS and Multicall3, and funds the 10 Anvil dev keys.
- `--chain anvil`: `anvil --hardfork osaka --chain-id 7117` (Osaka = P256VERIFY at `0x100`) with the **same genesis
  alloc** copied in (`anvil_setCode` / `anvil_setStorageAt`).
- `--rpc <url>`: use an already-running chain.
- Then: configures dev key #1 as an open XdevsPaymaster sponsor (0.05 tXDS per op cap, 20 tXDS deposit), stakes the
  paymaster, and deploys an XDRC-20 test token from `contracts/out-wallet` (built on demand with
  `forge build --root contracts --out out-wallet --cache-path cache-wallet`). Prints the wallet `.env.local`.

## Tests

`npm test` — integration tests against a real dev chain (`XDEVS_TEST_CHAIN=xdevs|anvil|auto`, default auto): userOpHash
equality with `EntryPoint.getUserOpHash` (with and without a paymaster signature), counterfactual deployment + first op,
paymaster-sponsored deployment, AA24 / wrong EntryPoint / execution-revert errors, receipt and by-hash lookup from logs
by a fresh instance. `npm run typecheck` — TypeScript.
