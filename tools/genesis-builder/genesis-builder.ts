#!/usr/bin/env node
// xDEVS genesis predeploy builder (plan §21.2). Node >= 22.18 / 24 (runs this TypeScript directly), Foundry.
// Docs: docs/specs/genesis-builder.md

import path from 'node:path';
import {fileURLToPath} from 'node:url';

import {build} from './src/build.ts';
import {verifyCanonical} from './src/canonical.ts';
import {merge} from './src/merge.ts';
import {formatTxds} from './src/util.ts';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

const USAGE = `usage: node tools/genesis-builder/genesis-builder.ts <command> [options]

  build   --config <cfg.json> [--out <genesis.json>] [--alloc-out <predeploys-alloc.json>]
          [--validators-from <genesis.json>]
      Compile (forge, contracts/out-genesis), compute every predeploy's genesis state in an in-memory EVM at
      block 0 and write the full genesis and/or the predeploy-only alloc.
      --validators-from: register the validators[] of an existing genesis (xdevs-cli testnet init) in Staking.

  merge   --genesis <in.json> --alloc <predeploys-alloc.json> --out <out.json>
          [--strict] [--allow-validator-mismatch]
      Add the predeploy alloc to an existing genesis, keeping its config and validators. Fails on chain-id
      mismatch, address collisions (except funding a predeploy from a balance-only entry, unless --strict),
      Staking/genesis validator mismatch and FeeConfig/config mismatch.

  verify-canonical [--solc <solc-0.8.28>] [--oz <dir> | --fetch-oz] [--rpc <ethereum mainnet rpc>]
      Re-derive the vendored EntryPoint v0.9 / Multicall3 / CREATE2-deployer code (script/CanonicalCode.sol).

Paths are relative to the current directory. Env: FORGE, CAST (binaries; default from PATH).`;

function parseArgs(argv: string[]) {
  const flags: Record<string, string | boolean> = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith('--')) throw new Error(`unexpected argument ${a}\n\n${USAGE}`);
    const k = a.slice(2);
    if (['strict', 'allow-validator-mismatch', 'fetch-oz', 'help'].includes(k)) flags[k] = true;
    else {
      const v = argv[++i];
      if (v === undefined) throw new Error(`--${k} needs a value`);
      flags[k] = v;
    }
  }
  return flags;
}

function main() {
  const [cmd, ...rest] = process.argv.slice(2);
  const f = parseArgs(rest);
  const s = (k: string) => (typeof f[k] === 'string' ? path.resolve(f[k] as string) : undefined);
  const log = (m: string) => console.error(m);
  switch (cmd) {
    case 'build': {
      if (!f.config) throw new Error(`--config is required\n\n${USAGE}`);
      if (!f.out && !f['alloc-out']) throw new Error('nothing to do: pass --out and/or --alloc-out');
      const r = build({root: ROOT, config: s('config')!, out: s('out'), allocOut: s('alloc-out'), validatorsFrom: s('validators-from'), log});
      const g = r.genesis;
      log(`chain id ${g.config.chainId}: ${Object.keys(g.alloc).length} accounts, total supply ${formatTxds(r.total)} tXDS`);
      log(`validators: ${r.validators.length}; validatorSetHash ${r.allocDoc.meta.validatorSetHash}`);
      for (const n of r.notes) log('note: ' + n);
      if (f.out) log(`wrote ${f.out}`);
      if (f['alloc-out']) log(`wrote ${f['alloc-out']}`);
      break;
    }
    case 'merge': {
      for (const k of ['genesis', 'alloc', 'out']) if (!f[k]) throw new Error(`--${k} is required\n\n${USAGE}`);
      const notes = merge({
        genesis: s('genesis')!,
        alloc: s('alloc')!,
        out: s('out')!,
        strict: !!f.strict,
        allowValidatorMismatch: !!f['allow-validator-mismatch'],
        log,
      });
      for (const n of notes) log('note: ' + n);
      log(`wrote ${f.out}`);
      break;
    }
    case 'verify-canonical':
      verifyCanonical(ROOT, {solc: s('solc'), oz: s('oz'), fetchOz: !!f['fetch-oz'], rpc: f.rpc as string | undefined, log});
      break;
    default:
      console.log(USAGE);
      process.exit(cmd === undefined || cmd === 'help' || cmd === '--help' ? 0 : 2);
  }
}

try {
  main();
} catch (e) {
  console.error('error: ' + (e as Error).message);
  process.exit(1);
}
