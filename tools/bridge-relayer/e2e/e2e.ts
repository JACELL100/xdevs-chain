// Local end-to-end test of both bridge directions with the real contracts and the real relayer:
//
//   anvil A = "Sepolia" (chain 11155111)    anvil B = "xDEVS Chain" (chain 717171)
//   lock on A -> relayer: dev finality update + headers + receipt proof -> mint on B
//   burn on B -> relayer: dev commit certificate + header + receipt proof -> release on A
//
// Consensus is simulated with LOCAL dev keys (src/dev.ts): a 512-member BLS sync committee signs synthetic
// beacon headers whose SSZ branches commit to anvil A's real block hashes, and 4 Ed25519 dev validators sign
// precommits over anvil B's real block hashes. Everything else (headers, receipt tries, contracts, relayer) is
// the production path. Requires `forge build` in contracts/ and anvil on PATH.
//
// Usage: node e2e/e2e.ts

import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import {
  createPublicClient,
  createWalletClient,
  defineChain,
  http,
  keccak256,
  parseAbi,
  toHex,
  type Abi,
  type Chain,
  type Address,
  type Hex,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { Relayer, type EthFinalitySource, type XdevsCommitSource } from "../src/relayer.ts";
import { ethereumLightClientAbi, xdevsBridgeAbi, vaultAbi } from "../src/abi.ts";
import { devCommit, devCommittee, devCommitteeRoot, devFinalityUpdate, devSet, devValidator } from "../src/dev.ts";

const OUT = path.resolve(import.meta.dirname, "../../../contracts/out");
// anvil's first default dev account (a public test key).
const DEPLOYER = privateKeyToAccount("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80");
const USER = privateKeyToAccount(keccak256(toHex("xdevs-e2e-user"))); // funded below
const SEPOLIA_ID = 11155111;
const XDEVS_ID = 717171;
const FORK_VERSION = "0x00000001" as Hex;
const GVR = keccak256(toHex("xdevs-dev-genesis-validators-root"));

function artifact(file: string, name: string): { abi: Abi; bytecode: Hex } {
  const j = JSON.parse(fs.readFileSync(path.join(OUT, file, `${name}.json`), "utf8"));
  return { abi: j.abi, bytecode: j.bytecode.object };
}

function startAnvil(port: number, chainId: number): Promise<ChildProcess> {
  const p = spawn("anvil", ["--port", String(port), "--chain-id", String(chainId), "--hardfork", "prague", "--gas-limit", "60000000"], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  return new Promise((resolve, reject) => {
    p.on("error", reject);
    p.stdout!.on("data", (d: Buffer) => {
      if (d.toString().includes("Listening on")) resolve(p);
    });
  });
}

const anvils: ChildProcess[] = [];
try {
  anvils.push(await startAnvil(18545, SEPOLIA_ID), await startAnvil(18546, XDEVS_ID));
  const chainA = defineChain({ id: SEPOLIA_ID, name: "sepolia-anvil", nativeCurrency: { name: "ETH", symbol: "ETH", decimals: 18 }, rpcUrls: { default: { http: ["http://127.0.0.1:18545"] } } });
  const chainB = defineChain({ id: XDEVS_ID, name: "xdevs-anvil", nativeCurrency: { name: "tXDS", symbol: "tXDS", decimals: 18 }, rpcUrls: { default: { http: ["http://127.0.0.1:18546"] } } });
  const a = createPublicClient({ chain: chainA, transport: http() });
  const b = createPublicClient({ chain: chainB, transport: http() });
  const wallet = (chain: Chain, account = DEPLOYER) => createWalletClient({ chain, account, transport: http() });
  const [wa, wb, ua, ub] = [wallet(chainA), wallet(chainB), wallet(chainA, USER), wallet(chainB, USER)];

  async function deploy(w: ReturnType<typeof wallet>, c: ReturnType<typeof createPublicClient>, file: string, name: string, args: unknown[]): Promise<Address> {
    const { abi, bytecode } = artifact(file, name);
    const hash = await w.deployContract({ abi, bytecode, args } as never);
    const r = await c.waitForTransactionReceipt({ hash });
    assert.equal(r.status, "success", `deploy ${name}`);
    return r.contractAddress!;
  }
  async function send(w: ReturnType<typeof wallet>, c: ReturnType<typeof createPublicClient>, address: Address, abi: Abi, functionName: string, args: unknown[]) {
    const hash = await w.writeContract({ address, abi, functionName, args } as never);
    const r = await c.waitForTransactionReceipt({ hash });
    assert.equal(r.status, "success", functionName);
    return r;
  }

  for (const [w, c] of [[wa, a], [wb, b]] as const) {
    await c.waitForTransactionReceipt({ hash: await w.sendTransaction({ to: USER.address, value: 10n ** 19n } as never) });
  }

  // ---- dev consensus ---------------------------------------------------------------------------------------
  const validators = [0, 1, 2, 3].map((i) => devValidator(`e2e-${i}`));
  const vset = devSet(validators, [10n, 10n, 10n, 10n]);
  const committee = devCommittee("e2e");
  const now = BigInt((await b.getBlock()).timestamp);
  let slot = now / 12n - 2000n; // genesisTime = 0: the contract clock is block.timestamp / 12
  if (slot / 8192n !== (slot + 500n) / 8192n) slot -= 1000n; // stay in one period
  const period = (slot + 2n) / 8192n;

  // ---- deploy "Sepolia" ------------------------------------------------------------------------------------
  const token = await deploy(wa, a, "BridgeEndpoints.t.sol", "TestToken", []);
  const xlc = await deploy(wa, a, "XdevsLightClient.sol", "XdevsLightClient", [BigInt(XDEVS_ID), 3600n, 2n ** 64n - 1n, "0x0000000000000000000000000000000000001000", 0n, vset]);
  const vault = await deploy(wa, a, "SepoliaTokenVault.sol", "SepoliaTokenVault", [xlc, BigInt(XDEVS_ID), DEPLOYER.address]);
  // ---- deploy "xDEVS" --------------------------------------------------------------------------------------
  const elc = await deploy(wb, b, "EthereumLightClient.sol", "EthereumLightClient", [GVR, 0n, [0n], [FORK_VERSION], 0n, period, devCommitteeRoot(committee)]);
  const factory = await deploy(wb, b, "CanonicalTokenFactory.sol", "CanonicalTokenFactory", [DEPLOYER.address, 30n * 86400n, 365n * 86400n]);
  const bridge = await deploy(wb, b, "XdevsBridge.sol", "XdevsBridge", [elc, BigInt(SEPOLIA_ID), DEPLOYER.address, factory]);
  await send(wa, a, vault, vaultAbi, "setCounterpart", [bridge]);
  await send(wb, b, bridge, xdevsBridgeAbi, "setCounterpart", [vault]);
  const factoryAbi = parseAbi([
    "function createCanonicalToken(uint256,address,string,string,uint8,address) returns (address)",
    "function canonicalToken(uint256,address) view returns (address)",
  ]);
  await send(wb, b, factory, factoryAbi, "createCanonicalToken", [BigInt(SEPOLIA_ID), token, "Test (Sepolia)", "TST", 18, bridge]);
  const bridged = await b.readContract({ address: factory, abi: factoryAbi, functionName: "canonicalToken", args: [BigInt(SEPOLIA_ID), token] });
  console.log(`deployed: vault ${vault}, XdevsLightClient ${xlc} | bridge ${bridge}, EthereumLightClient ${elc}, bridged token ${bridged}`);

  // ---- sources ---------------------------------------------------------------------------------------------
  let nextSlot = slot;
  const eth: EthFinalitySource = {
    async finalityFor(minBlock) {
      const blk = await a.getBlock();
      if (blk.number < minBlock) return null;
      nextSlot += 4n;
      const f = devFinalityUpdate(committee, blk.hash!, nextSlot, FORK_VERSION, GVR);
      return { update: f.update as never, nonSigners: f.nonSigners, executionBlockHash: blk.hash! };
    },
    async committeeKeys() {
      return committee.keys;
    },
  };
  const xdevsSource: XdevsCommitSource = {
    async validatorSet() {
      return vset;
    },
    async commitFor(height, set) {
      const blk = await b.getBlock({ blockNumber: height });
      return devCommit(BigInt(XDEVS_ID), height, 0, blk.hash!, set, validators.slice(0, 3));
    },
  };
  const relayerFor = (dryRun: boolean) =>
    new Relayer({ sepolia: a as never, xdevs: b as never, sepoliaWallet: wa, xdevsWallet: wb, vault, xdevsBridge: bridge, ethLightClient: elc, xdevsLightClient: xlc, eth, xdevsSource, dryRun, log: (m) => console.log(`  ${m}`) });

  const erc20 = parseAbi([
    "function mint(address,uint256)",
    "function approve(address,uint256) returns (bool)",
    "function balanceOf(address) view returns (uint256)",
  ]);

  // ---- lock -> mint ----------------------------------------------------------------------------------------
  await send(wa, a, token, erc20, "mint", [USER.address, 1000n * 10n ** 18n]);
  await send(ua, a, token, erc20, "approve", [vault, 100n * 10n ** 18n]);
  await send(ua, a, vault, vaultAbi, "lock", [token, 100n * 10n ** 18n, USER.address]);
  console.log("locked 100 TST on Sepolia-anvil");

  const dry = relayerFor(true);
  assert.equal(await dry.relayLocks(0n), 0);
  assert.ok(dry.planned.length > 0, "dry run plans actions");
  assert.equal(await b.readContract({ address: bridged, abi: erc20, functionName: "balanceOf", args: [USER.address] }), 0n);
  console.log(`dry run planned ${dry.planned.join(", ")} and sent nothing`);

  const relayer = relayerFor(false);
  assert.equal(await relayer.relayLocks(0n), 1);
  assert.equal(await b.readContract({ address: bridged, abi: erc20, functionName: "balanceOf", args: [USER.address] }), 100n * 10n ** 18n);
  assert.equal(await relayer.relayLocks(0n), 0, "a processed lock is skipped");
  console.log("minted 100 bridged TST on xDEVS-anvil");

  // ---- burn -> release -------------------------------------------------------------------------------------
  const before = await a.readContract({ address: token, abi: erc20, functionName: "balanceOf", args: [USER.address] });
  await send(ub, b, bridge, xdevsBridgeAbi, "burn", [bridged, 40n * 10n ** 18n, USER.address]);
  assert.equal(await relayer.relayBurns(0n), 1);
  const after = await a.readContract({ address: token, abi: erc20, functionName: "balanceOf", args: [USER.address] });
  assert.equal(after - before, 40n * 10n ** 18n);
  assert.equal(await relayer.relayBurns(0n), 0, "a processed burn is skipped");
  assert.equal(await b.readContract({ address: bridged, abi: erc20, functionName: "balanceOf", args: [USER.address] }), 60n * 10n ** 18n);
  console.log("released 40 TST on Sepolia-anvil");

  // ---- light-client state ----------------------------------------------------------------------------------
  const finalized = (await b.readContract({ address: elc, abi: ethereumLightClientAbi, functionName: "latestNumber" })) as bigint;
  assert.ok(finalized > 0n);
  console.log("e2e OK: lock -> mint -> burn -> release, replays rejected, dry run sends nothing");
} finally {
  for (const p of anvils) p.kill();
}
