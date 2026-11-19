// Real finality sources: a Sepolia beacon node (light-client REST API) and an xDEVS node (via @xdevs/sdk).

import { ed25519 } from "@noble/curves/ed25519";
import { getCommit } from "@xdevs/sdk";
import { parseAbi, type Address, type Hex, type PublicClient } from "viem";
import { BeaconClient, committeeFromPubkeys, SLOTS_PER_PERIOD, toContractUpdate, type Committee } from "./beacon.ts";
import type { EthFinality, EthFinalitySource, XdevsCommitSource } from "./relayer.ts";
import { setFromStaking, STAKING_ADDRESS, toContractCommit, type Validator } from "./xdevs.ts";

export class BeaconEthSource implements EthFinalitySource {
  private readonly beacon: BeaconClient;
  private readonly sepolia: PublicClient;
  private readonly committees = new Map<bigint, Committee>();

  constructor(beacon: BeaconClient, sepolia: PublicClient) {
    this.beacon = beacon;
    this.sepolia = sepolia;
  }

  /** Committee of `period` = next_sync_committee of the best update of `period - 1`. */
  private async committee(period: bigint): Promise<Committee> {
    let c = this.committees.get(period);
    if (!c) {
      const [u] = await this.beacon.updates(Number(period - 1n), 1);
      if (!u?.next_sync_committee) throw new Error(`no sync committee for period ${period}`);
      c = committeeFromPubkeys(u.next_sync_committee.pubkeys, u.next_sync_committee.aggregate_pubkey);
      this.committees.set(period, c);
    }
    return c;
  }

  async finalityFor(minBlock: bigint): Promise<EthFinality | null> {
    const fu = await this.beacon.finalityUpdate();
    const hash = fu.finalized_header.execution_block_hash;
    const blk = await this.sepolia.getBlock({ blockHash: hash });
    if (blk.number < minBlock) return null;
    const period = BigInt(fu.signature_slot) / BigInt(SLOTS_PER_PERIOD);
    const { update, nonSigners } = toContractUpdate(fu, await this.committee(period), false);
    return { update: update as unknown as EthFinality["update"], nonSigners, executionBlockHash: hash };
  }

  async committeeKeys(period: bigint) {
    return (await this.committee(period)).keys;
  }

  async rotationInto(period: bigint) {
    const [u] = await this.beacon.updates(Number(period - 1n), 1);
    if (!u) return null;
    return toContractUpdate(u, await this.committee(period - 1n), true);
  }
}

const stakingAbi = parseAbi(["function consensusValidatorSet() view returns (address[], bytes32[], uint256[])"]);

/** ZIP-215 point check, as the node's `ValidatorKey::from_bytes`. */
export function isEd25519Point(key: Hex): boolean {
  try {
    // zip215 = true: permissive decoding, as consensus (ed25519-consensus) does.
    (ed25519.ExtendedPoint.fromHex as (hex: string, zip215?: boolean) => unknown)(key.slice(2), true);
    return true;
  } catch {
    return false;
  }
}

export class XdevsRpcSource implements XdevsCommitSource {
  private readonly client: PublicClient;
  private readonly epochLength: bigint;
  private readonly stakingStart: bigint | null;

  /** `epochLength` / `stakingStart`: the chain's `ChainConfig` (stakingStart null = staking never active). */
  constructor(client: PublicClient, epochLength: bigint, stakingStart: bigint | null) {
    this.client = client;
    this.epochLength = epochLength;
    this.stakingStart = stakingStart;
  }

  private boundary(h: bigint): bigint {
    if (this.stakingStart === null || this.epochLength === 0n || h < 2n) return 0n;
    const last = ((h - 1n) / this.epochLength) * this.epochLength;
    return last > 0n && last >= this.stakingStart ? last : 0n;
  }

  async stakingSet(b: bigint) {
    const [operators, keys, powersWei] = await this.client.readContract({
      address: STAKING_ADDRESS,
      abi: stakingAbi,
      functionName: "consensusValidatorSet",
      blockNumber: b,
    });
    return { operators: [...operators] as Address[], keys: [...keys] as Hex[], powersWei: [...powersWei] };
  }

  async validatorSet(h: bigint): Promise<Validator[]> {
    const s = await this.stakingSet(this.boundary(h));
    return setFromStaking(s.keys, s.powersWei, isEd25519Point);
  }

  async commitFor(height: bigint, set: Validator[]) {
    const cert = await getCommit(this.client as never, { height });
    if (!cert) throw new Error(`no commit certificate for xDEVS block ${height}`);
    return toContractCommit(cert, set);
  }
}
