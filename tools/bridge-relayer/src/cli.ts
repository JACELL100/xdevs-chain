// xdevs-bridge-relayer: forwards Sepolia finality + lock proofs to xDEVS Chain and xDEVS commit certificates +
// burn proofs to Sepolia. Permissionless: it needs gas on both chains, nothing else.
//
//   node src/cli.ts --sepolia-rpc URL --beacon URL --xdevs-rpc URL \
//     --vault 0x.. --xdevs-bridge 0x.. --eth-light-client 0x.. --xdevs-light-client 0x.. \
//     [--from-sepolia N] [--from-xdevs N] [--epoch-length 3600] [--staking-start N] \
//     [--once] [--interval 30] [--dry-run]
//
// PRIVATE_KEY (env) signs transactions; without it, or with --dry-run, nothing is sent: every step is simulated
// and printed.

import { parseArgs } from "node:util";
import { createPublicClient, createWalletClient, http, type Address, type Chain } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { sepolia } from "viem/chains";
import { xdevsEnclave } from "@xdevs/sdk";
import { BeaconClient } from "./beacon.ts";
import { Relayer } from "./relayer.ts";
import { BeaconEthSource, XdevsRpcSource } from "./sources.ts";

const { values: a } = parseArgs({
  options: {
    "sepolia-rpc": { type: "string" },
    beacon: { type: "string" },
    "xdevs-rpc": { type: "string", default: "https://rpc.xdevschain.tech" },
    vault: { type: "string" },
    "xdevs-bridge": { type: "string" },
    "eth-light-client": { type: "string" },
    "xdevs-light-client": { type: "string" },
    "from-sepolia": { type: "string", default: "0" },
    "from-xdevs": { type: "string", default: "0" },
    "epoch-length": { type: "string", default: "3600" },
    "staking-start": { type: "string" },
    interval: { type: "string", default: "30" },
    once: { type: "boolean", default: false },
    "dry-run": { type: "boolean", default: false },
  },
});

function need(k: keyof typeof a): string {
  const v = a[k];
  if (typeof v !== "string" || !v) throw new Error(`--${k} is required`);
  return v;
}

const sepoliaClient = createPublicClient({ chain: sepolia, transport: http(need("sepolia-rpc")) });
const xdevsClient = createPublicClient({ chain: xdevsEnclave as Chain, transport: http(need("xdevs-rpc")) });
const pk = process.env.PRIVATE_KEY as `0x${string}` | undefined;
const dryRun = a["dry-run"] || !pk;
const account = pk ? privateKeyToAccount(pk) : undefined;

const relayer = new Relayer({
  sepolia: sepoliaClient as never,
  xdevs: xdevsClient as never,
  sepoliaWallet: account && !dryRun ? createWalletClient({ account, chain: sepolia, transport: http(need("sepolia-rpc")) }) : undefined,
  xdevsWallet: account && !dryRun ? createWalletClient({ account, chain: xdevsEnclave as Chain, transport: http(need("xdevs-rpc")) }) : undefined,
  vault: need("vault") as Address,
  xdevsBridge: need("xdevs-bridge") as Address,
  ethLightClient: need("eth-light-client") as Address,
  xdevsLightClient: need("xdevs-light-client") as Address,
  eth: new BeaconEthSource(new BeaconClient(need("beacon")), sepoliaClient as never),
  xdevsSource: new XdevsRpcSource(
    xdevsClient as never,
    BigInt(a["epoch-length"]!),
    a["staking-start"] ? BigInt(a["staking-start"]) : null,
  ),
  dryRun,
});

if (dryRun) console.log("dry run: nothing will be sent");
for (;;) {
  try {
    const locks = await relayer.relayLocks(BigInt(a["from-sepolia"]!));
    const burns = await relayer.relayBurns(BigInt(a["from-xdevs"]!));
    console.log(`relayed ${locks} lock(s), ${burns} burn(s)`);
  } catch (e) {
    console.error(`relay round failed: ${(e as Error).message}`);
  }
  if (a.once) break;
  await new Promise((r) => setTimeout(r, Number(a.interval) * 1000));
}
