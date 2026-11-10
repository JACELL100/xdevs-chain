// `build`: config -> forge script (in-memory EVM at block 0) -> predeploy alloc + full genesis.

import {spawnSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

import {
  type Account,
  type Alloc,
  SYSTEM_ADDRESS,
  TREASURY,
  checkFeeConfig,
  formatConsensusKey,
  mergeAlloc,
  normalizeAccount,
  normalizeAlloc,
  orderedAccount,
  parseConsensusKey,
  sortAlloc,
  totalBalance,
} from './genesis.ts';
import {
  WEI_PER_TXDS,
  addrKey,
  checksumAddress,
  fail,
  formatTxds,
  hexToBytes,
  keccakHex,
  parseQuantity,
  parseTxds,
  toJson,
} from './util.ts';

export const BUILDER_VERSION = '1';
export const SCRIPT = 'tools/genesis-builder/script/GenesisPredeploys.s.sol';
export const FORGE_OUT = 'out-genesis'; // relative to contracts/ (gitignored)
export const FORGE_CACHE = 'cache-genesis';

/** Predeploys and where their runtime code comes from (artifact under contracts/out-genesis). */
export const PREDEPLOYS: [string, string, string][] = [
  ['0x0000000000000000000000000000000000001000', 'Staking', 'Staking.sol/Staking.json'],
  ['0x0000000000000000000000000000000000001001', 'Governance', 'Governance.sol/Governance.json'],
  ['0x0000000000000000000000000000000000001002', 'Treasury', 'Treasury.sol/Treasury.json'],
  ['0x0000000000000000000000000000000000001003', 'FeeConfig', 'FeeConfig.sol/FeeConfig.json'],
  ['0x0000000000000000000000000000000000001004', 'XNSRegistry', 'XNSRegistry.sol/XNSRegistry.json'],
  ['0x0000000000000000000000000000000000001005', 'XdevsSmartAccount', 'XdevsSmartAccount.sol/XdevsSmartAccount.json'],
  ['0x0000000000000000000000000000000000001006', 'SmartAccountFactory', 'SmartAccountFactory.sol/SmartAccountFactory.json'],
  ['0x0000000000000000000000000000000000001007', 'XdevsPaymaster', 'XdevsPaymaster.sol/XdevsPaymaster.json'],
  ['0x0000000000000000000000000000000000001008', 'Permit2Expiring', 'Permit2Expiring.sol/Permit2Expiring.json'],
  ['0x0000000000000000000000000000000000001009', 'WXDS', 'WXDS.sol/WXDS.json'],
];
export const CANONICAL: [string, string][] = [
  ['0x4e59b44847b379578588920cA78FbF26c0B4956C', 'DeterministicDeploymentProxy (CREATE2 deployer)'],
  ['0x433709009B8330FDa32311DF1C2AFA402eD8D009', 'EntryPoint v0.9'],
  ['0x0A630a99Df908A81115A3022927Be82f9299987e', 'SenderCreator (created by EntryPoint v0.9)'],
  ['0xcA11bde05977b3631167028862bE2a173976CA11', 'Multicall3'],
];
/** Immutables whose zero-filled placeholder is intended (docs/specs/xns-and-approvals.md §1.1). */
const ALLOWED_IMMUTABLES: Record<string, number> = {XNSRegistry: 1};

export interface ValidatorIn {
  name?: string;
  operator: string;
  consensusKey: string; // bytes32
  commissionBps: number;
  stake: bigint; // wei
  p2p?: string;
  power?: number; // only when taken verbatim from an existing genesis
  passthrough?: any; // the original genesis entry (validatorsFrom)
}

export interface BuildOptions {
  root: string; // repo root
  config: string;
  out?: string; // full genesis
  allocOut?: string; // predeploys-alloc.json
  validatorsFrom?: string; // take validators[] from an existing genesis (xdevs-cli testnet init)
  forge?: string;
  log: (s: string) => void;
}

export interface ForgeResult {
  accounts: Record<string, {balance: string; nonce: string; code: string; storage: Record<string, string>}>;
  validatorSet: {operators: string[]; consensusKeys: string[]; powers: string[]};
  validatorSetHash: string;
  senderCreator: string;
  feeConfigVersion: number;
}

function readJson(file: string): any {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (e) {
    fail(`cannot read ${file}: ${(e as Error).message}`);
  }
}

function resolveValidators(cfg: any, opts: BuildOptions): {vals: ValidatorIn[]; baseValidators?: any[]} {
  const defaults = cfg.validatorDefaults ?? {};
  if (opts.validatorsFrom) {
    const g = readJson(opts.validatorsFrom);
    const list: any[] = g.validators ?? [];
    if (!list.length) fail(`${opts.validatorsFrom} has no validators[] (dev chains have none: build without --validators-from)`);
    const vals = list.map((v, i): ValidatorIn => {
      const operator = v.operator ?? defaults.operators?.[i];
      if (!operator) fail(`validator #${i} (${v.name ?? '?'}) has no "operator" and validatorDefaults.operators[${i}] is not set`);
      const stake = v.stake !== undefined ? parseQuantity(v.stake) : defaults.stake !== undefined ? parseTxds(defaults.stake) : fail(`validator #${i}: no stake (set validatorDefaults.stake)`);
      return {
        name: v.name,
        operator: checksumAddress(operator),
        consensusKey: parseConsensusKey(v.consensusKey),
        commissionBps: Number(v.commissionBps ?? defaults.commissionBps ?? 1000),
        stake,
        p2p: v.p2p,
        power: Number(v.power),
        passthrough: v,
      };
    });
    return {vals, baseValidators: list};
  }
  const list: any[] = cfg.validators ?? fail('config has no validators[] (Staking needs at least one genesis validator)');
  const vals = list.map((v, i): ValidatorIn => ({
    name: v.name,
    operator: checksumAddress(v.operator ?? fail(`validators[${i}].operator missing`)),
    consensusKey: parseConsensusKey(v.consensusKey ?? fail(`validators[${i}].consensusKey missing`)),
    commissionBps: Number(v.commissionBps ?? defaults.commissionBps ?? 1000),
    stake: parseTxds(v.stake ?? defaults.stake ?? fail(`validators[${i}].stake missing`)),
    p2p: v.p2p,
  }));
  return {vals};
}

function forgeParams(cfg: any, genesis: any, vals: ValidatorIn[]): any {
  const bf = genesis.config?.baseFee ?? {};
  // FeeConfig defaults follow the genesis header/config so the two can never disagree; explicit `fee` overrides
  // win and are then checked against the genesis (checkFeeConfig).
  const derivedFee: Record<string, string> = {};
  if (genesis.gasLimit !== undefined) derivedFee.blockGasLimit = parseQuantity(genesis.gasLimit).toString();
  if (bf.minimum !== undefined) derivedFee.minBaseFee = parseQuantity(bf.minimum).toString();
  if (bf.burnBps !== undefined) derivedFee.burnBps = String(bf.burnBps);
  if (bf.maxChangeDenominator !== undefined) derivedFee.baseFeeMaxChangeDenominator = String(bf.maxChangeDenominator);
  if (bf.elasticity !== undefined) derivedFee.elasticityMultiplier = String(bf.elasticity);
  const stringify = (o: any) =>
    Object.fromEntries(Object.entries(o ?? {}).map(([k, v]) => [k, typeof v === 'number' ? String(v) : v]));
  const p: any = {
    chainId: String(parseQuantity(genesis.config.chainId)),
    timestamp: String(parseQuantity(genesis.timestamp ?? '0')),
    stakingBalance: vals.reduce((s, v) => s + v.stake, 0n).toString(),
    validators: {
      operators: vals.map((v) => v.operator),
      consensusKeys: vals.map((v) => v.consensusKey),
      commissionBps: vals.map((v) => String(v.commissionBps)),
      stakes: vals.map((v) => v.stake.toString()),
    },
    fee: {...derivedFee, ...stringify(cfg.fee)},
  };
  if (cfg.staking) p.staking = stringify(cfg.staking);
  if (cfg.governance) {
    const {guardian, ...rest} = cfg.governance;
    p.governance = stringify(rest);
    if (guardian) p.guardian = checksumAddress(guardian);
  }
  if (cfg.xns) p.xns = stringify(cfg.xns);
  // Large uints go as decimal strings; forge's JSON parser reads them as uint256.
  for (const k of ['staking', 'governance', 'fee'])
    if (p[k]) for (const [kk, vv] of Object.entries(p[k])) p[k][kk] = String(parseQuantity(String(vv)));
  return p;
}

export function runForge(root: string, params: any, forge: string, log: (s: string) => void): ForgeResult {
  const args = ['script', SCRIPT, '--root', 'contracts', '--out', FORGE_OUT, '--cache-path', FORGE_CACHE, '--json'];
  log(`$ GENESIS_PARAMS=<${JSON.stringify(params).length} bytes> ${forge} ${args.join(' ')}`);
  const r = spawnSync(forge, args, {
    cwd: root,
    env: {...process.env, GENESIS_PARAMS: JSON.stringify(params), FOUNDRY_PROFILE: 'default'},
    encoding: 'utf8',
    maxBuffer: 1 << 30,
  });
  if (r.error) fail(`cannot run forge (${forge}): ${r.error.message} — add ~/.foundry/bin to PATH or set FORGE`);
  const line = (r.stdout ?? '').split('\n').find((l) => l.trim().startsWith('{') && l.includes('"returned"'));
  if (r.status !== 0 || !line) {
    fail(`forge script failed (exit ${r.status}):\n${(r.stderr ?? '').slice(-4000)}\n${(r.stdout ?? '').slice(-2000)}`);
  }
  const j = JSON.parse(line);
  if (j.success === false) fail('forge script reverted');
  // run() returns (string json): ABI-decode the return data.
  const b = Buffer.from(String(j.returned).replace(/^0x/, ''), 'hex');
  const off = Number(b.readBigUInt64BE(24));
  const len = Number(b.readBigUInt64BE(off + 24));
  return JSON.parse(b.subarray(off + 32, off + 32 + len).toString('utf8'));
}

function checkImmutables(root: string, alloc: Alloc): string[] {
  const notes: string[] = [];
  for (const [addr, name, file] of PREDEPLOYS) {
    const art = readJson(path.join(root, 'contracts', FORGE_OUT, file));
    const refs: Record<string, {start: number; length: number}[]> = art.deployedBytecode?.immutableReferences ?? {};
    const all = Object.values(refs).flat();
    const allowed = ALLOWED_IMMUTABLES[name] ?? 0;
    if (all.length !== allowed) fail(`${name}: ${all.length} immutable reference(s) in the runtime code, expected ${allowed}`);
    const code = hexToBytes(alloc[checksumAddress(addr)].code ?? '0x');
    if (('0x' + Buffer.from(code).toString('hex')) !== String(art.deployedBytecode.object).toLowerCase())
      fail(`${name}: genesis code differs from ${FORGE_OUT}/${file}`);
    for (const r of all) {
      if (code.subarray(r.start, r.start + r.length).some((x) => x !== 0)) fail(`${name}: immutable at ${r.start} not zero`);
    }
    if (all.length) notes.push(`${name}: ${all.length} zero-filled immutable (intended)`);
  }
  return notes;
}

export function build(opts: BuildOptions) {
  const {root, log} = opts;
  const cfgPath = path.resolve(opts.config);
  const cfg = readJson(cfgPath);
  if (opts.validatorsFrom && opts.out)
    fail('--validators-from builds a predeploy alloc for an existing genesis: use --alloc-out, then `merge`');
  // With --validators-from the chain parameters (chain id, timestamp, gas limit, base fee) come from that genesis,
  // so the alloc is guaranteed to match it (EntryPoint chain id, FeeConfig values).
  const genesisIn = opts.validatorsFrom
    ? readJson(opts.validatorsFrom)
    : (cfg.genesis ?? fail('config.genesis (skeleton: config, timestamp, gasLimit, extraData) missing'));
  const chainId = parseQuantity(genesisIn.config?.chainId ?? fail('config.genesis.config.chainId missing'));
  if (genesisIn.config.treasury && addrKey(genesisIn.config.treasury) !== addrKey(TREASURY))
    fail(`config.treasury must be the Treasury predeploy ${TREASURY}`);
  if (genesisIn.config.systemAddress && addrKey(genesisIn.config.systemAddress) !== addrKey(SYSTEM_ADDRESS))
    fail(`config.systemAddress must be ${SYSTEM_ADDRESS}`);

  const {vals, baseValidators} = resolveValidators(cfg, opts);
  const params = forgeParams(cfg, genesisIn, vals);
  const res = runForge(root, params, opts.forge ?? process.env.FORGE ?? 'forge', log);

  // ---- predeploy alloc
  const predeploys: Alloc = {};
  for (const [a, acc] of Object.entries(res.accounts)) {
    predeploys[checksumAddress(a)] = normalizeAccount(acc, `forge[${a}]`);
  }
  const expected = [...PREDEPLOYS.map(([a]) => a), ...CANONICAL.map(([a]) => a)];
  for (const a of expected) {
    const acc = predeploys[checksumAddress(a)];
    if (!acc?.code) fail(`predeploy ${a} has no code`);
  }
  if (Object.keys(predeploys).length !== expected.length) fail('unexpected account in forge output');
  if (addrKey(res.senderCreator) !== addrKey(CANONICAL[2][0])) fail(`SenderCreator at ${res.senderCreator}`);
  const notes = checkImmutables(root, predeploys);

  // ---- Staking set == configured validators
  const set = res.validatorSet;
  const want = new Map(vals.map((v) => [v.consensusKey, v]));
  if (set.consensusKeys.length !== vals.length) fail(`Staking selected ${set.consensusKeys.length} of ${vals.length} validators (maxValidators / eligibility?)`);
  set.consensusKeys.forEach((k, i) => {
    const v = want.get(k.toLowerCase()) ?? fail(`Staking active key ${k} not in the configured validators`);
    if (addrKey(v.operator) !== addrKey(set.operators[i])) fail(`operator mismatch for ${k}`);
    if (BigInt(set.powers[i]) !== v.stake) fail(`power mismatch for ${k}: ${set.powers[i]} != ${v.stake}`);
  });
  if (res.feeConfigVersion !== 1) fail(`FeeConfig.version = ${res.feeConfigVersion}`);

  // ---- genesis validators[] (node, Phase 2)
  let validators: any[];
  if (baseValidators) {
    validators = baseValidators; // verbatim: the node's set is the one testnet init generated
    // powers must be proportional to stakes so both views rank validators the same way
    const p0 = BigInt(vals[0].power ?? 1), s0 = vals[0].stake;
    for (const v of vals) {
      if (BigInt(v.power ?? 1) * s0 !== p0 * v.stake)
        fail(`genesis power ${v.power} of ${v.name ?? v.consensusKey} is not proportional to its Staking stake ${formatTxds(v.stake)} tXDS`);
    }
  } else {
    validators = set.consensusKeys.map((k, i) => {
      const v = want.get(k.toLowerCase())!;
      if (v.stake % WEI_PER_TXDS !== 0n) fail(`stake of ${v.name ?? k} must be a whole number of tXDS (power = stake / 1e18)`);
      const power = v.stake / WEI_PER_TXDS;
      if (power < 1n || power > BigInt(Number.MAX_SAFE_INTEGER)) fail(`power ${power} out of range`);
      const o: any = {};
      if (v.name) o.name = v.name;
      o.consensusKey = formatConsensusKey(k);
      o.power = Number(power);
      if (v.p2p) o.p2p = v.p2p;
      o.operator = v.operator;
      o.stake = v.stake.toString();
      o.commissionBps = v.commissionBps;
      return o;
    });
  }

  // ---- plain balances
  const balances: Alloc = {};
  for (const [i, a] of (cfg.allocations ?? []).entries()) {
    const addr = checksumAddress(a.address ?? fail(`allocations[${i}].address missing`));
    const prev = balances[addr] ? BigInt(balances[addr].balance) : 0n;
    balances[addr] = {balance: (prev + parseTxds(a.amount)).toString()};
  }
  const mergeNotes: string[] = [];
  let alloc = mergeAlloc(sortAlloc(balances), predeploys, false, mergeNotes);
  alloc = Object.fromEntries(Object.entries(alloc).map(([k, v]) => [k, orderedAccount(v)]));

  const total = totalBalance(alloc);
  if (cfg.totalSupply !== undefined && total !== parseTxds(cfg.totalSupply))
    fail(`total supply ${formatTxds(total)} tXDS != configured totalSupply ${cfg.totalSupply}`);

  const genesis: any = {
    config: genesisIn.config,
    timestamp: genesisIn.timestamp ?? '0x0',
    gasLimit: genesisIn.gasLimit,
    extraData: genesisIn.extraData,
    alloc,
  };
  for (const k of Object.keys(genesis)) if (genesis[k] === undefined) delete genesis[k];
  if (cfg.emitValidators !== false) genesis.validators = validators;
  const feeProblems = checkFeeConfig(genesis, alloc);
  if (feeProblems.length) fail('FeeConfig / genesis config mismatch:\n  ' + feeProblems.join('\n  '));

  // ---- predeploys-alloc.json (mergeable into any genesis with the same chain id)
  const contracts: any = {};
  for (const [a, name] of [...PREDEPLOYS.map(([a, n]) => [a, n]), ...CANONICAL]) {
    const acc = predeploys[checksumAddress(a)];
    contracts[checksumAddress(a)] = {name, codeHash: keccakHex(acc.code!), codeSize: hexToBytes(acc.code!).length};
  }
  const allocDoc = {
    meta: {
      generator: `tools/genesis-builder v${BUILDER_VERSION}`,
      config: path.relative(root, cfgPath).split(path.sep).join('/'),
      chainId: Number(chainId),
      timestamp: params.timestamp,
      note:
        'Chain-specific: EntryPoint v0.9 caches its EIP-712 domain (chain id) in immutables, and Staking storage ' +
        'encodes the genesis validators below. Merge only into a genesis with this chainId and these validators ' +
        '(node genesis-builder.ts merge checks both).',
      stakingValidators: set.consensusKeys.map((k, i) => ({
        operator: set.operators[i],
        consensusKey: formatConsensusKey(k),
        stake: set.powers[i],
      })),
      validatorSetHash: res.validatorSetHash,
      stakingBalance: params.stakingBalance,
      contracts,
    },
    alloc: Object.fromEntries(Object.entries(sortAlloc(predeploys)).map(([k, v]) => [k, orderedAccount(v)])),
  };

  if (opts.allocOut) fs.writeFileSync(opts.allocOut, toJson(allocDoc));
  if (opts.out) fs.writeFileSync(opts.out, toJson(genesis));

  return {genesis, allocDoc, notes: [...notes, ...mergeNotes], total, validators};
}

export type {Account};
export {normalizeAlloc};
