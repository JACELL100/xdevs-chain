// WebAuthn (P-256 passkey) signature encoding for XdevsSmartAccount.
//
// The account (contracts/src/accounts/base/SignatureVerifier.sol + libraries/WebAuthnP256.sol) expects
//   0x02 || abi.encode(bytes32 x, bytes32 y, WebAuthnAuth auth)
//   WebAuthnAuth = (bytes32 r, bytes32 s, uint256 challengeIndex, uint256 typeIndex, bytes authenticatorData,
//                   string clientDataJSON)                                  (OpenZeppelin WebAuthn struct order)
// and checks: `"type":"webauthn.get"` at typeIndex, `"challenge":"<base64url(hash)>"` at challengeIndex (BYTE offsets
// in the UTF-8 clientDataJSON), UP + UV flags, BE/BS consistency, low-s, and P256VERIFY over
// sha256(authenticatorData || sha256(clientDataJSON)).
import { type Hex, bytesToHex, concat, encodeAbiParameters, hexToBytes, pad, sha256, toHex } from 'viem'

export const P256_N = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551n
/** `WebAuthnP256.HALF_N` = floor(N / 2): signatures with s > HALF_N are rejected on-chain. */
export const P256_HALF_N = P256_N / 2n

export const SIG_TYPE_PASSKEY = '0x02'

export type P256PublicKey = { x: Hex; y: Hex }

export type WebAuthnAssertion = {
  authenticatorData: Hex
  clientDataJSON: string
  r: bigint
  s: bigint
}

export function base64UrlEncode(bytes: Uint8Array): string {
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

export function base64UrlDecode(s: string): Uint8Array {
  const b64 = s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4)
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

/** Byte offset of `needle` in `hay` (both UTF-8), or -1. */
export function indexOfBytes(hay: Uint8Array, needle: Uint8Array): number {
  outer: for (let i = 0; i + needle.length <= hay.length; i++) {
    for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer
    return i
  }
  return -1
}

/** Parses an ASN.1 DER ECDSA signature (what `AuthenticatorAssertionResponse.signature` holds). */
export function parseDerSignature(der: Uint8Array): { r: bigint; s: bigint } {
  let i = 0
  const expect = (tag: number) => {
    if (der[i++] !== tag) throw new Error('invalid DER signature')
  }
  const readLen = () => {
    let len = der[i++]
    if (len & 0x80) {
      const n = len & 0x7f
      len = 0
      for (let k = 0; k < n; k++) len = (len << 8) | der[i++]
    }
    return len
  }
  expect(0x30)
  readLen()
  expect(0x02)
  const rl = readLen()
  const r = BigInt(bytesToHex(der.slice(i, i + rl)))
  i += rl
  expect(0x02)
  const sl = readLen()
  const s = BigInt(bytesToHex(der.slice(i, i + sl)))
  return { r, s }
}

/** Parses a raw IEEE P1363 `r || s` signature (WebCrypto output). */
export function parseRawSignature(sig: Uint8Array): { r: bigint; s: bigint } {
  if (sig.length !== 64) throw new Error('expected a 64-byte r||s signature')
  return { r: BigInt(bytesToHex(sig.slice(0, 32))), s: BigInt(bytesToHex(sig.slice(32))) }
}

/** ECDSA malleability: (r, s) and (r, n - s) are both valid; the account only accepts the low-s form. */
export function normalizeS(s: bigint): bigint {
  return s > P256_HALF_N ? P256_N - s : s
}

/** Extracts (x, y) from an SPKI-encoded P-256 public key (`AuthenticatorAttestationResponse.getPublicKey()`). */
export function spkiToPublicKey(spki: Uint8Array): P256PublicKey {
  // SubjectPublicKeyInfo for P-256 ends with the 65-byte uncompressed point 0x04 || X || Y.
  const point = spki.slice(spki.length - 65)
  if (point[0] !== 0x04) throw new Error('not an uncompressed P-256 public key')
  return { x: bytesToHex(point.slice(1, 33)), y: bytesToHex(point.slice(33, 65)) }
}

/** The 32-byte digest P256VERIFY checks: sha256(authenticatorData || sha256(clientDataJSON)). */
export function webauthnDigest(authenticatorData: Hex, clientDataJSON: string): Hex {
  return sha256(concat([authenticatorData, sha256(toHex(new TextEncoder().encode(clientDataJSON)))]))
}

/** Locates the type and challenge fields the account checks; throws if the assertion does not commit to `challenge`. */
export function webauthnIndexes(clientDataJSON: string, challenge: Hex): { typeIndex: bigint; challengeIndex: bigint } {
  const json = new TextEncoder().encode(clientDataJSON)
  const typeIndex = indexOfBytes(json, new TextEncoder().encode('"type":"webauthn.get"'))
  const expected = `"challenge":"${base64UrlEncode(hexToBytes(challenge))}"`
  const challengeIndex = indexOfBytes(json, new TextEncoder().encode(expected))
  if (typeIndex < 0) throw new Error('clientDataJSON is not a webauthn.get assertion')
  if (challengeIndex < 0) throw new Error('clientDataJSON does not contain the expected challenge')
  return { typeIndex: BigInt(typeIndex), challengeIndex: BigInt(challengeIndex) }
}

const WEBAUTHN_AUTH = {
  type: 'tuple',
  components: [
    { name: 'r', type: 'bytes32' },
    { name: 's', type: 'bytes32' },
    { name: 'challengeIndex', type: 'uint256' },
    { name: 'typeIndex', type: 'uint256' },
    { name: 'authenticatorData', type: 'bytes' },
    { name: 'clientDataJSON', type: 'string' },
  ],
} as const

/** Encodes an assertion over `challenge` (the userOpHash or ERC-1271 replay-safe hash) as an owner signature. */
export function encodePasskeySignature(key: P256PublicKey, a: WebAuthnAssertion, challenge: Hex): Hex {
  const { typeIndex, challengeIndex } = webauthnIndexes(a.clientDataJSON, challenge)
  return encodePasskeySignatureRaw(key, { ...a, s: normalizeS(a.s) }, challengeIndex, typeIndex)
}

export function encodePasskeySignatureRaw(
  key: P256PublicKey,
  a: WebAuthnAssertion,
  challengeIndex: bigint,
  typeIndex: bigint,
): Hex {
  const body = encodeAbiParameters(
    [{ type: 'bytes32' }, { type: 'bytes32' }, WEBAUTHN_AUTH],
    [
      pad(key.x, { size: 32 }),
      pad(key.y, { size: 32 }),
      {
        r: pad(toHex(a.r), { size: 32 }),
        s: pad(toHex(a.s), { size: 32 }),
        challengeIndex,
        typeIndex,
        authenticatorData: a.authenticatorData,
        clientDataJSON: a.clientDataJSON,
      },
    ],
  )
  return concat([SIG_TYPE_PASSKEY, body])
}

/** Authenticator flags: UP (0x01) | UV (0x04). */
export const FLAGS_UP_UV = 0x05

/** authenticatorData = rpIdHash(32) || flags(1) || signCount(4, big-endian). */
export function buildAuthenticatorData(rpId: string, flags = FLAGS_UP_UV, signCount = 0): Hex {
  const rpIdHash = sha256(toHex(new TextEncoder().encode(rpId)))
  return concat([rpIdHash, toHex(flags, { size: 1 }), toHex(signCount, { size: 4 })])
}

export function buildClientDataJSON(challenge: Hex, origin: string): string {
  return JSON.stringify({
    type: 'webauthn.get',
    challenge: base64UrlEncode(hexToBytes(challenge)),
    origin,
    crossOrigin: false,
  })
}

/**
 * Dummy passkey signature with the same shape and length as a real one, for gas estimation. It fails verification
 * (the bundler ignores signature failures while estimating) but exercises the same decoding path.
 */
export function dummyPasskeySignature(key: P256PublicKey, origin = 'https://wallet.xdevs.invalid'): Hex {
  const challenge = pad('0x01', { size: 32 })
  const clientDataJSON = buildClientDataJSON(challenge, origin)
  const { typeIndex, challengeIndex } = webauthnIndexes(clientDataJSON, challenge)
  return encodePasskeySignatureRaw(
    key,
    { authenticatorData: buildAuthenticatorData('wallet.xdevs.invalid'), clientDataJSON, r: P256_HALF_N - 1n, s: P256_HALF_N - 1n },
    challengeIndex,
    typeIndex,
  )
}
