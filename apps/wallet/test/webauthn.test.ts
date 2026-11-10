// WebAuthn / P-256 signature encoding, checked against the chain's real P256VERIFY precompile (0x100) and against
// XdevsSmartAccount's own verifier (ERC-1271 path of a passkey-only factory account).
import assert from 'node:assert/strict'
import { after, before, describe, it } from 'node:test'
import {
  type Address,
  type Hex,
  type PublicClient,
  concat,
  createPublicClient,
  createWalletClient,
  decodeAbiParameters,
  encodeFunctionData,
  hexToBytes,
  http,
  keccak256,
  pad,
  sha256,
  toHex,
} from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { DEV_KEYS, ROLES } from '../../bundler/scripts/devnet.ts'
import { devChain } from '../../bundler/test/helpers.ts'
import { accountAbi, factoryAbi } from '../src/lib/abi.ts'
import { SMART_ACCOUNT_FACTORY, buildInitParams, saltFrom } from '../src/lib/account.ts'
import { SoftPasskey, passkeySignerStruct } from '../src/lib/signers.ts'
import {
  P256_HALF_N,
  P256_N,
  base64UrlDecode,
  base64UrlEncode,
  buildClientDataJSON,
  encodePasskeySignature,
  normalizeS,
  parseDerSignature,
  webauthnDigest,
  webauthnIndexes,
} from '../src/lib/webauthn.ts'

// RFC 6979 appendix A.2.5: ECDSA P-256 / SHA-256, message "sample".
const RFC = {
  d: 'C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721',
  x: '0x60FED4BA255A9D31C961EB74C6356D68C049B8923B61FA6CE669622E60F29FB6' as Hex,
  y: '0x7903FE1008B8BC99A41AE9E95628BC64F2F1B20C2D7E9F5177A3C294D4462299' as Hex,
  r: 0xefd48b2aacb6a8fd1140dd9cd45e81d69d2c877b56aaf991c34d0ea84eaf3716n,
  s: 0xf7cb1c942d657c41d436c7a1b6e29f65f3e900dbb9aff4064dc4ab2f843acda8n,
}
const P256_VERIFY: Address = '0x0000000000000000000000000000000000000100'

let chain: Awaited<ReturnType<typeof devChain>>['chain']
let client: PublicClient

before(async () => {
  ;({ chain } = await devChain({ token: false }))
  client = createPublicClient({ transport: http(chain.rpcUrl), pollingInterval: 200 })
})
after(async () => {
  await chain?.stop()
})

async function precompile(h: Hex, r: bigint, s: bigint, x: Hex, y: Hex): Promise<Hex | undefined> {
  const res = await client.call({ to: P256_VERIFY, data: concat([h, pad(toHex(r)), pad(toHex(s)), pad(x), pad(y)]) })
  return res.data
}

const ONE = pad('0x01')

describe('P-256 / WebAuthn encoding', () => {
  it('base64url round-trips and matches the account encoding (no padding, -_ alphabet)', () => {
    const b = hexToBytes('0xfbff00ff3e3f')
    assert.equal(base64UrlEncode(b), '-_8A_z4_')
    assert.deepEqual(base64UrlDecode('-_8A_z4_'), b)
  })

  it('parses DER signatures and normalizes to low-s', () => {
    const der = hexToBytes(
      concat(['0x3046', '0x022100', pad(toHex(RFC.r)), '0x022100', pad(toHex(RFC.s))]),
    )
    assert.deepEqual(parseDerSignature(der), { r: RFC.r, s: RFC.s })
    assert.equal(normalizeS(RFC.s), P256_N - RFC.s)
    assert.ok(normalizeS(RFC.s) <= P256_HALF_N)
  })

  it('RFC 6979 vector verifies with WebCrypto and with the chain P256VERIFY precompile (0x100)', async () => {
    const pub = await crypto.subtle.importKey(
      'jwk',
      { kty: 'EC', crv: 'P-256', x: base64UrlEncode(hexToBytes(RFC.x)), y: base64UrlEncode(hexToBytes(RFC.y)) },
      { name: 'ECDSA', namedCurve: 'P-256' },
      false,
      ['verify'],
    )
    const sig = new Uint8Array(hexToBytes(concat([pad(toHex(RFC.r)), pad(toHex(RFC.s))])))
    assert.ok(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, pub, sig, new TextEncoder().encode('sample')))
    const h = sha256(toHex(new TextEncoder().encode('sample')))
    assert.equal(await precompile(h, RFC.r, RFC.s, RFC.x, RFC.y), ONE)
    assert.equal(await precompile(h, RFC.r, normalizeS(RFC.s), RFC.x, RFC.y), ONE, 'low-s form is valid too')
    const bad = await precompile(h, RFC.r + 1n, RFC.s, RFC.x, RFC.y)
    assert.ok(!bad || bad === '0x', 'tampered signature must not verify')
  })

  it('a software-passkey WebAuthn assertion verifies at 0x100 and the encoding points at the right fields', async () => {
    const pk = await SoftPasskey.generate('wallet.example', 'https://wallet.example')
    const challenge = keccak256('0x1234')
    const a = await pk.assert(challenge)
    const digest = webauthnDigest(a.authenticatorData, a.clientDataJSON)
    assert.equal(await precompile(digest, a.r, normalizeS(a.s), pk.publicKey.x, pk.publicKey.y), ONE)

    const sig = encodePasskeySignature(pk.publicKey, a, challenge)
    assert.equal(sig.slice(0, 4), '0x02')
    const [x, y, auth] = decodeAbiParameters(
      [
        { type: 'bytes32' },
        { type: 'bytes32' },
        { type: 'tuple', components: [
          { name: 'r', type: 'bytes32' }, { name: 's', type: 'bytes32' }, { name: 'challengeIndex', type: 'uint256' },
          { name: 'typeIndex', type: 'uint256' }, { name: 'authenticatorData', type: 'bytes' }, { name: 'clientDataJSON', type: 'string' },
        ] },
      ],
      `0x${sig.slice(4)}`,
    )
    assert.equal(x, pad(pk.publicKey.x))
    assert.equal(y, pad(pk.publicKey.y))
    assert.ok(BigInt(auth.s) <= P256_HALF_N)
    const json = new TextEncoder().encode(auth.clientDataJSON)
    const at = (i: bigint, s: string) => new TextDecoder().decode(json.slice(Number(i), Number(i) + s.length)) === s
    assert.ok(at(auth.typeIndex, '"type":"webauthn.get"'))
    assert.ok(at(auth.challengeIndex, `"challenge":"${base64UrlEncode(hexToBytes(challenge))}"`))
    assert.equal(hexToBytes(auth.authenticatorData)[32] & 0x05, 0x05, 'UP and UV flags')
  })

  it('byte offsets (not UTF-16 offsets) are used when clientDataJSON contains non-ASCII', () => {
    const challenge = keccak256('0x01')
    const json = buildClientDataJSON(challenge, 'https://wället.example').replace('{"type"', '{"x":"ü","type"')
    const { typeIndex } = webauthnIndexes(json, challenge)
    assert.equal(typeIndex, BigInt(new TextEncoder().encode('{"x":"ü",').length))
  })

  it('XdevsSmartAccount accepts the encoded passkey signature (ERC-1271) and rejects a wrong challenge', async () => {
    const pk = await SoftPasskey.generate()
    const params = buildInitParams({ signers: [passkeySignerStruct(pk.publicKey)] })
    const salt = saltFrom(42)
    const deployer = createWalletClient({ account: privateKeyToAccount(DEV_KEYS[ROLES.faucet]), transport: http(chain.rpcUrl) })
    const hash = await deployer.sendTransaction({
      chain: null,
      to: SMART_ACCOUNT_FACTORY,
      data: encodeFunctionData({ abi: factoryAbi, functionName: 'createAccountWithConfig', args: [params, salt] }),
    })
    await client.waitForTransactionReceipt({ hash })
    const account = await client.readContract({ address: SMART_ACCOUNT_FACTORY, abi: factoryAbi, functionName: 'getAddressWithConfig', args: [params, salt] })

    const message = keccak256(toHex('hello xDEVS'))
    const safe = await client.readContract({ address: account, abi: accountAbi, functionName: 'replaySafeHash', args: [message] })
    const sig = await pk.signer().signHash(safe)
    assert.equal(await client.readContract({ address: account, abi: accountAbi, functionName: 'isValidSignature', args: [message, sig] }), '0x1626ba7e')
    const wrong = await pk.signer().signHash(keccak256(toHex('other')))
    assert.equal(await client.readContract({ address: account, abi: accountAbi, functionName: 'isValidSignature', args: [message, wrong] }), '0xffffffff')
  })
})
