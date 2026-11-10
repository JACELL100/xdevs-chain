// `merge`: put predeploys-alloc.json into an existing genesis (e.g. from `xdevs-cli testnet init`),
// keeping its config and validators.

import fs from 'node:fs';

import {SYSTEM_ADDRESS, TREASURY, checkFeeConfig, mergeAlloc, normalizeAlloc, orderedAccount, parseConsensusKey, totalBalance} from './genesis.ts';
import {addrKey, fail, formatTxds, parseQuantity, toJson} from './util.ts';

export interface MergeOptions {
  genesis: string;
  alloc: string;
  out: string;
  strict?: boolean; // no predeploy funding exception: any shared address is an error
  allowValidatorMismatch?: boolean;
  log: (s: string) => void;
}

export function merge(opts: MergeOptions) {
  const base = JSON.parse(fs.readFileSync(opts.genesis, 'utf8'));
  const doc = JSON.parse(fs.readFileSync(opts.alloc, 'utf8'));
  const meta = doc.meta ?? {};
  const extra = normalizeAlloc(doc.alloc ?? fail(`${opts.alloc}: no "alloc"`), opts.alloc);
  const baseAlloc = normalizeAlloc(base.alloc ?? {}, opts.genesis, true);

  const chainId = parseQuantity(base.config?.chainId ?? fail(`${opts.genesis}: config.chainId missing`));
  if (meta.chainId === undefined) fail(`${opts.alloc}: meta.chainId missing`);
  if (BigInt(meta.chainId) !== chainId)
    fail(
      `chain id mismatch: genesis ${chainId}, predeploy alloc built for ${meta.chainId} ` +
        `(EntryPoint's EIP-712 immutables are chain-specific: rebuild the alloc for this chain id)`,
    );
  const cfg = base.config;
  if (cfg.treasury && addrKey(cfg.treasury) !== addrKey(TREASURY)) fail(`config.treasury is not ${TREASURY}`);
  if (cfg.systemAddress && addrKey(cfg.systemAddress) !== addrKey(SYSTEM_ADDRESS)) fail(`config.systemAddress is not ${SYSTEM_ADDRESS}`);

  const notes: string[] = [];
  const staking: any[] = meta.stakingValidators ?? [];
  const nodeVals: any[] = base.validators ?? [];
  if (nodeVals.length) {
    const a = new Set(nodeVals.map((v) => parseConsensusKey(v.consensusKey)));
    const b = new Set(staking.map((v) => parseConsensusKey(v.consensusKey)));
    const same = a.size === b.size && [...a].every((k) => b.has(k));
    if (!same) {
      const msg =
        `genesis validators[] (${a.size}) and the Staking genesis validators in ${opts.alloc} (${b.size}) have different ` +
        `consensus keys. Rebuild the alloc from this genesis: node tools/genesis-builder/genesis-builder.ts build ` +
        `--config <cfg> --validators-from ${opts.genesis} --alloc-out <alloc.json>`;
      if (!opts.allowValidatorMismatch) fail(msg);
      notes.push('WARNING (allowed): ' + msg);
    } else {
      notes.push(`validators: ${a.size} genesis validators == Staking genesis set (same consensus keys)`);
    }
  } else {
    notes.push(
      `genesis has no validators[] (single-node dev chain): Staking holds ${staking.length} genesis validator(s) ` +
        'that do not drive consensus',
    );
  }

  const alloc = mergeAlloc(baseAlloc, extra, !!opts.strict, notes);
  const out: any = {};
  for (const [k, v] of Object.entries(base)) out[k] = k === 'alloc' ? null : v;
  out.alloc = Object.fromEntries(Object.entries(alloc).map(([k, v]) => [k, orderedAccount(v)]));
  if (!('alloc' in base)) {
    // keep `alloc` before `validators` like the node's serializer
    const {validators, ...rest} = out;
    Object.keys(out).forEach((k) => delete out[k]);
    Object.assign(out, rest, validators !== undefined ? {validators} : {});
  }
  const fee = checkFeeConfig(out, out.alloc);
  if (fee.length) fail('FeeConfig / genesis config mismatch:\n  ' + fee.join('\n  '));

  fs.writeFileSync(opts.out, toJson(out));
  notes.push(`total supply ${formatTxds(totalBalance(out.alloc))} tXDS over ${Object.keys(out.alloc).length} accounts`);
  return notes;
}
