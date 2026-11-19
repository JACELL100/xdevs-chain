// The permissionless relayer. It holds no trust: everything it submits is checked on chain by the light clients
// (signatures, Merkle branches, receipt proofs) and by the bridge endpoints (emitter, event, nonce). A malicious
// relayer can only fail to relay; anyone can run another one.
//
//   Sepolia -> xDEVS: Locked(nonce) on the vault  -> finality update (+ committee) -> headers -> XdevsBridge.mint
//   xDEVS -> Sepolia: BurnedForRelease on xDEVS  -> validator set (if a new epoch) -> commit -> header -> vault.release

import {
  pad,
  toHex,
  type Abi,
  type Address,
  type Hex,
  type PublicClient,
  type WalletClient,
} from "viem";
import { ethereumLightClientAbi, vaultAbi, xdevsBridgeAbi, xdevsLightClientAbi } from "./abi.ts";
import { headerRlp, receiptProof, type RpcBlock, type RpcReceipt } from "./encoding.ts";
import type { G1 } from "./beacon.ts";
import type { Validator } from "./xdevs.ts";
import { STAKING_ADDRESS } from "./xdevs.ts";

const ZERO32 = pad("0x", { size: 32 });

/** Ethereum finality source: a contract-form update that finalizes an execution block >= `minBlock`. */
export interface EthFinalitySource {
  finalityFor(minBlock: bigint): Promise<EthFinality | null>;
  /** Uncompressed keys (committee order) of a period's sync committee, for `loadCommittee`. */
  committeeKeys(period: bigint): Promise<G1[]>;
  /** A rotation update proving the committee of `period` (signed in `period - 1`), if one is needed. */
  rotationInto?(period: bigint): Promise<{ update: unknown; nonSigners: unknown[] } | null>;
}

export interface EthFinality {
  update: { signatureSlot: bigint } & Record<string, unknown>;
  nonSigners: unknown[];
  executionBlockHash: Hex;
}

/** xDEVS finality source: the commit certificate of a height in contract form. */
export interface XdevsCommitSource {
  commitFor(height: bigint, set: Validator[]): Promise<{ height: bigint; round: number; blockHash: Hex; sigs: unknown[] }>;
  /** The consensus validator set in force at `height`, address-sorted. */
  validatorSet(height: bigint): Promise<Validator[]>;
  /** Staking.consensusValidatorSet() at block `boundary`, as the contract's StakingSetProof minus proofs. */
  stakingSet?(boundary: bigint): Promise<{ operators: Address[]; keys: Hex[]; powersWei: bigint[] }>;
}

export interface RelayerOptions {
  sepolia: PublicClient;
  xdevs: PublicClient;
  sepoliaWallet?: WalletClient;
  xdevsWallet?: WalletClient;
  vault: Address;
  xdevsBridge: Address;
  /** EthereumLightClient on xDEVS Chain. */
  ethLightClient: Address;
  /** XdevsLightClient on Sepolia. */
  xdevsLightClient: Address;
  eth: EthFinalitySource;
  xdevsSource: XdevsCommitSource;
  dryRun?: boolean;
  log?: (msg: string) => void;
}

type Side = "sepolia" | "xdevs";

export class Relayer {
  private readonly o: RelayerOptions;
  private readonly log: (msg: string) => void;
  /** Actions a dry run would have sent, in order. */
  readonly planned: string[] = [];

  constructor(o: RelayerOptions) {
    this.o = o;
    this.log = o.log ?? ((m) => console.log(m));
  }

  private client(side: Side): PublicClient {
    return side === "sepolia" ? this.o.sepolia : this.o.xdevs;
  }

  /** Send (or, in dry-run mode, simulate and record) a contract call. Returns false if a dry run stops here. */
  private async send(side: Side, address: Address, abi: Abi, functionName: string, args: unknown[]): Promise<boolean> {
    const client = this.client(side);
    const wallet = side === "sepolia" ? this.o.sepoliaWallet : this.o.xdevsWallet;
    const what = `${side}:${functionName}`;
    if (this.o.dryRun || !wallet) {
      try {
        const gas = await client.estimateContractGas({
          address,
          abi,
          functionName,
          args,
          account: wallet?.account ?? "0x000000000000000000000000000000000000dEaD",
        } as never);
        this.log(`[dry-run] ${what}: would send, ~${gas} gas`);
      } catch (e) {
        this.log(`[dry-run] ${what}: would send (gas not estimable before earlier steps: ${(e as Error).message.split("\n")[0]})`);
      }
      this.planned.push(what);
      return false;
    }
    const hash = await wallet.writeContract({ address, abi, functionName, args, chain: wallet.chain, account: wallet.account! } as never);
    const r = await client.waitForTransactionReceipt({ hash });
    if (r.status !== "success") throw new Error(`${what} reverted (${hash})`);
    this.log(`${what}: ${hash} (${r.gasUsed} gas)`);
    return true;
  }

  private async read<T>(side: Side, address: Address, abi: Abi, functionName: string, args: unknown[] = []): Promise<T> {
    return (await this.client(side).readContract({ address, abi, functionName, args } as never)) as T;
  }

  private async block(side: Side, n: bigint): Promise<RpcBlock> {
    return (await this.client(side).request({ method: "eth_getBlockByNumber", params: [toHex(n), false] } as never)) as RpcBlock;
  }

  private async receipts(side: Side, n: bigint): Promise<RpcReceipt[]> {
    return (await this.client(side).request({ method: "eth_getBlockReceipts", params: [toHex(n)] } as never)) as RpcReceipt[];
  }

  /** Receipt proof (contract EventProof) of the log with block-level index `logIndex` in block `n`. */
  private async eventProof(side: Side, n: bigint, txHash: Hex, logIndex: number) {
    const [blk, rcs] = await Promise.all([this.block(side, n), this.receipts(side, n)]);
    const txIndex = rcs.findIndex((r) => r.transactionHash === txHash);
    if (txIndex < 0) throw new Error(`tx ${txHash} not in block ${n}`);
    const inReceipt = rcs[txIndex]!.logs.findIndex((l) => Number(l.logIndex) === logIndex);
    return {
      blockNumber: n,
      txIndex: BigInt(txIndex),
      receiptProof: receiptProof(rcs, txIndex, blk.receiptsRoot),
      logIndex: BigInt(inReceipt),
    };
  }

  // ----------------------------------------------------------------------------------------------------------
  // Sepolia -> xDEVS
  // ----------------------------------------------------------------------------------------------------------

  /** Relay every unprocessed `Locked` event from `fromBlock` on. Returns the number of mints sent. */
  async relayLocks(fromBlock: bigint): Promise<number> {
    const logs = await this.o.sepolia.getContractEvents({ address: this.o.vault, abi: vaultAbi, eventName: "Locked", fromBlock });
    let done = 0;
    for (const l of logs) {
      const nonce = l.args.nonce!;
      if (await this.read<boolean>("xdevs", this.o.xdevsBridge, xdevsBridgeAbi, "processed", [nonce])) continue;
      this.log(`lock #${nonce} in Sepolia block ${l.blockNumber}`);
      if (!(await this.ensureSepoliaHeader(l.blockNumber!))) continue;
      const p = await this.eventProof("sepolia", l.blockNumber!, l.transactionHash!, l.logIndex!);
      if (await this.send("xdevs", this.o.xdevsBridge, xdevsBridgeAbi, "mint", [p])) done++;
    }
    return done;
  }

  /** Make the EthereumLightClient know Sepolia block `n` (finality update, then headers down to `n`). */
  private async ensureSepoliaHeader(n: bigint): Promise<boolean> {
    const lc = this.o.ethLightClient;
    const abi = ethereumLightClientAbi;
    if ((await this.read<Hex>("xdevs", lc, abi, "blockHashAt", [n])) !== ZERO32) return true;
    const f = await this.o.eth.finalityFor(n);
    if (!f) {
      this.log(`  Sepolia block ${n} is not finalized yet`);
      return false;
    }
    if ((await this.read<bigint>("xdevs", lc, abi, "trusted", [f.executionBlockHash])) === 0n) {
      const period = f.update.signatureSlot / 8192n;
      if (!(await this.read<boolean>("xdevs", lc, abi, "committeeLoaded", [period]))) {
        if ((await this.read<Hex>("xdevs", lc, abi, "committeeRoot", [period])) === ZERO32) {
          const rot = await this.o.eth.rotationInto?.(period);
          if (!rot) throw new Error(`sync committee of period ${period} unknown and no rotation update available`);
          if (!(await this.send("xdevs", lc, abi, "submitUpdate", [rot.update, rot.nonSigners]))) return false;
        }
        const keys = await this.o.eth.committeeKeys(period);
        if (!(await this.send("xdevs", lc, abi, "loadCommittee", [period, keys]))) return false;
      }
      if (!(await this.send("xdevs", lc, abi, "submitUpdate", [f.update, f.nonSigners]))) return false;
    }
    // Walk back from the finalized block to n via parent hashes.
    const top = BigInt((await this.o.sepolia.getBlock({ blockHash: f.executionBlockHash })).number);
    for (let k = top; k >= n; k--) {
      if ((await this.read<Hex>("xdevs", lc, abi, "blockHashAt", [k])) !== ZERO32) continue;
      if (!(await this.send("xdevs", lc, abi, "submitHeader", [headerRlp(await this.block("sepolia", k))]))) return false;
    }
    return true;
  }

  // ----------------------------------------------------------------------------------------------------------
  // xDEVS -> Sepolia
  // ----------------------------------------------------------------------------------------------------------

  /** Relay every unprocessed `BurnedForRelease` from `fromBlock` on. Returns the number of releases sent. */
  async relayBurns(fromBlock: bigint): Promise<number> {
    const logs = await this.o.xdevs.getContractEvents({ address: this.o.xdevsBridge, abi: xdevsBridgeAbi, eventName: "BurnedForRelease", fromBlock });
    let done = 0;
    for (const l of logs) {
      const nonce = l.args.nonce!;
      if (await this.read<boolean>("sepolia", this.o.vault, vaultAbi, "processed", [nonce])) continue;
      this.log(`burn #${nonce} in xDEVS block ${l.blockNumber}`);
      if (!(await this.ensureXdevsHeader(l.blockNumber!))) continue;
      const p = await this.eventProof("xdevs", l.blockNumber!, l.transactionHash!, l.logIndex!);
      if (await this.send("sepolia", this.o.vault, vaultAbi, "release", [p])) done++;
    }
    return done;
  }

  /** Make the XdevsLightClient know xDEVS block `h` (validator set of its epoch, commit, header). */
  private async ensureXdevsHeader(h: bigint): Promise<boolean> {
    const lc = this.o.xdevsLightClient;
    const abi = xdevsLightClientAbi;
    if ((await this.read<Hex>("sepolia", lc, abi, "blockHashAt", [h])) !== ZERO32) return true;
    const boundary = await this.read<bigint>("sepolia", lc, abi, "validatorSetBoundary", [h]);
    if (!(await this.ensureValidatorSet(boundary))) return false;
    const set = await this.o.xdevsSource.validatorSet(h);
    if ((await this.read<Hex>("sepolia", lc, abi, "certifiedBlock", [h])) === ZERO32) {
      const c = await this.o.xdevsSource.commitFor(h, set);
      if (!(await this.send("sepolia", lc, abi, "submitCommit", [c.height, c.round, c.blockHash, set, c.sigs]))) return false;
    }
    return this.send("sepolia", lc, abi, "submitHeader", [headerRlp(await this.block("xdevs", h))]);
  }

  /** Prove the validator set fixed by epoch boundary `b` (recursively the earlier ones first). */
  private async ensureValidatorSet(b: bigint): Promise<boolean> {
    const lc = this.o.xdevsLightClient;
    const abi = xdevsLightClientAbi;
    const [hash] = await this.read<[Hex, bigint]>("sepolia", lc, abi, "validatorSets", [b]);
    if (hash !== ZERO32 || b === 0n) return true;
    if (!this.o.xdevsSource.stakingSet) throw new Error(`validator set of boundary ${b} unknown`);
    if (!(await this.ensureXdevsHeader(b))) return false; // certifies the boundary block (older set)
    const s = await this.o.xdevsSource.stakingSet(b);
    const proof = await this.o.xdevs.getProof({
      address: STAKING_ADDRESS,
      storageKeys: [pad(toHex(3), { size: 32 }), pad(toHex(4), { size: 32 })],
      blockNumber: b,
    });
    return this.send("sepolia", lc, abi, "submitValidatorSet", [
      b,
      { ...s, accountProof: proof.accountProof, epochWordProof: proof.storageProof[0]!.proof, setHashProof: proof.storageProof[1]!.proof },
    ]);
  }
}
