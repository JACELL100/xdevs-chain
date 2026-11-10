// `verify-canonical`: re-derive the vendored canonical code in script/CanonicalCode.sol.
//   1. always: CREATE2(deployer, salt, keccak(EntryPoint init code)) == 0x433709…D009 and the pinned hashes;
//   2. --solc/--oz (or auto): recompile EntryPoint v0.9 from contracts/lib/account-abstraction exactly as the
//      upstream hardhat build did and compare the init code byte for byte;
//   3. --rpc <ethereum mainnet RPC>: compare Multicall3 / deployer runtime code with mainnet, and EntryPoint +
//      SenderCreator runtime code produced by our forge pipeline at chain id 1 with mainnet's.

import {execFileSync, spawnSync} from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import {runForge} from './build.ts';
import {bytesToHex, checksumAddress, fail, hexToBytes, keccak256, keccakHex} from './util.ts';

const SOL = 'tools/genesis-builder/script/CanonicalCode.sol';

export function readCanonical(root: string) {
  const src = fs.readFileSync(path.join(root, SOL), 'utf8');
  const fn = (name: string) => {
    const m = new RegExp(`function ${name}\\(\\)[^{]*\\{\\s*return\\s*([^;]*);`).exec(src) ?? fail(`${name} not found`);
    return '0x' + [...m[1].matchAll(/hex"([0-9a-f]*)"/g)].map((x) => x[1]).join('');
  };
  const c = (name: string) => (new RegExp(`${name} = (0x[0-9a-fA-F]+);`).exec(src) ?? fail(`${name} not found`))[1];
  return {
    entryPointInit: fn('entryPointInitCode'),
    multicall3: fn('multicall3RuntimeCode'),
    deployer: fn('create2DeployerRuntimeCode'),
    salt: c('ENTRY_POINT_SALT'),
    initHash: c('ENTRY_POINT_INITCODE_HASH'),
    multicallHash: c('MULTICALL3_CODEHASH'),
    deployerHash: c('CREATE2_DEPLOYER_CODEHASH'),
    deployerAddr: c('CREATE2_DEPLOYER'),
    entryPoint: c('ENTRY_POINT'),
    multicallAddr: c('MULTICALL3'),
  };
}

function create2(deployer: string, salt: string, initHash: string): string {
  const buf = Buffer.concat([Buffer.from([0xff]), hexToBytes(deployer), hexToBytes(salt), hexToBytes(initHash)]);
  return checksumAddress(bytesToHex(keccak256(buf).subarray(12)));
}

function defaultSolc(): string | undefined {
  const home = os.homedir();
  const cands = [
    process.env.APPDATA && path.join(process.env.APPDATA, 'svm', '0.8.28', 'solc-0.8.28'),
    process.env.APPDATA && path.join(process.env.APPDATA, 'svm', '0.8.28', 'solc-0.8.28.exe'),
    path.join(home, '.svm', '0.8.28', 'solc-0.8.28'),
    path.join(home, '.foundry', 'svm', '0.8.28', 'solc-0.8.28'),
  ].filter(Boolean) as string[];
  return cands.find((c) => fs.existsSync(c));
}

/** Hardhat-style standard-JSON build of contracts/core/EntryPoint.sol (settings of the upstream hardhat.config.ts). */
export function compileEntryPoint(root: string, solc: string, ozDir: string): string {
  const aa = path.join(root, 'contracts', 'lib', 'account-abstraction');
  const sources: Record<string, {content: string}> = {};
  const resolve = (n: string) =>
    n.startsWith('@openzeppelin/contracts/') ? path.join(ozDir, n.slice('@openzeppelin/contracts/'.length)) : path.join(aa, n);
  const visit = (name: string) => {
    if (sources[name]) return;
    // Upstream sources are LF; a Windows checkout (core.autocrlf) may have CRLF, which changes the metadata hash.
    const content = fs.readFileSync(resolve(name), 'utf8').replaceAll('\r\n', '\n');
    sources[name] = {content};
    for (const m of content.matchAll(/^\s*import\s+(?:[^"']*from\s+)?["']([^"']+)["']/gm)) {
      const imp = m[1].startsWith('.') ? path.posix.normalize(path.posix.join(path.posix.dirname(name), m[1])) : m[1];
      visit(imp);
    }
  };
  visit('contracts/core/EntryPoint.sol');
  const input = {
    language: 'Solidity',
    sources,
    settings: {
      evmVersion: 'cancun',
      optimizer: {enabled: true, runs: 1000000},
      viaIR: true,
      outputSelection: {'*': {'*': ['evm.bytecode.object']}},
    },
  };
  const out = JSON.parse(execFileSync(solc, ['--standard-json'], {input: JSON.stringify(input), maxBuffer: 1 << 28}).toString());
  for (const e of out.errors ?? []) if (e.severity === 'error') fail(e.formattedMessage);
  return '0x' + out.contracts['contracts/core/EntryPoint.sol'].EntryPoint.evm.bytecode.object;
}

function fetchOz(log: (s: string) => void): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'oz510-'));
  log(`npm pack @openzeppelin/contracts@5.1.0 -> ${dir}`);
  // npm is a .cmd shim on Windows, so it needs a shell; the command line is a constant.
  const r = spawnSync('npm pack @openzeppelin/contracts@5.1.0 --silent', {cwd: dir, shell: true, encoding: 'utf8'});
  if (r.status !== 0) fail(`npm pack failed: ${r.stderr}`);
  execFileSync('tar', ['xzf', 'openzeppelin-contracts-5.1.0.tgz'], {cwd: dir});
  return path.join(dir, 'package');
}

function castCode(addr: string, rpc: string): string {
  return execFileSync(process.env.CAST ?? 'cast', ['code', addr, '--rpc-url', rpc], {maxBuffer: 1 << 26}).toString().trim();
}

export function verifyCanonical(root: string, o: {solc?: string; oz?: string; fetchOz?: boolean; rpc?: string; log: (s: string) => void}) {
  const {log} = o;
  const c = readCanonical(root);
  const ok = (cond: boolean, msg: string) => {
    if (!cond) fail('FAIL ' + msg);
    log('ok   ' + msg);
  };
  ok(keccakHex(c.entryPointInit) === c.initHash.toLowerCase(), `EntryPoint init code hash ${c.initHash}`);
  ok(keccakHex(c.multicall3) === c.multicallHash.toLowerCase(), `Multicall3 code hash ${c.multicallHash}`);
  ok(keccakHex(c.deployer) === c.deployerHash.toLowerCase(), `CREATE2 deployer code hash ${c.deployerHash}`);
  ok(create2(c.deployerAddr, c.salt, c.initHash) === checksumAddress(c.entryPoint), `CREATE2(deployer, salt, initHash) = ${c.entryPoint}`);

  const solc = o.solc ?? defaultSolc();
  const oz = o.oz ?? (o.fetchOz ? fetchOz(log) : undefined);
  if (solc && oz) {
    const mine = compileEntryPoint(root, solc, oz);
    if (!o.oz) fs.rmSync(path.dirname(oz), {recursive: true, force: true});
    ok(mine === c.entryPointInit.toLowerCase(), `EntryPoint init code reproduced from contracts/lib/account-abstraction (${(mine.length - 2) / 2} bytes, solc ${solc})`);
  } else {
    log(`skip source reproduction (need --solc <solc 0.8.28> and --oz <@openzeppelin/contracts 5.1.0 dir> or --fetch-oz; solc found: ${solc ?? 'no'})`);
  }

  if (o.rpc) {
    ok(castCode(c.multicallAddr, o.rpc).toLowerCase() === c.multicall3.toLowerCase(), 'Multicall3 runtime == mainnet');
    ok(castCode(c.deployerAddr, o.rpc).toLowerCase() === c.deployer.toLowerCase(), 'CREATE2 deployer runtime == mainnet');
    const params = {
      chainId: '1',
      timestamp: '0',
      stakingBalance: '10000000000000000000000',
      validators: {
        operators: ['0x0000000000000000000000000000000000A11CE0'],
        consensusKeys: ['0x' + '11'.repeat(32)],
        commissionBps: ['1000'],
        stakes: ['10000000000000000000000'],
      },
    };
    const res = runForge(root, params, process.env.FORGE ?? 'forge', log);
    const get = (a: string) => Object.entries(res.accounts).find(([k]) => k.toLowerCase() === a.toLowerCase())![1].code.toLowerCase();
    ok(get(c.entryPoint) === castCode(c.entryPoint, o.rpc).toLowerCase(), 'EntryPoint runtime built at chain id 1 == mainnet (immutables included)');
    ok(get(res.senderCreator) === castCode(res.senderCreator, o.rpc).toLowerCase(), `SenderCreator ${res.senderCreator} runtime == mainnet`);
  }
}
