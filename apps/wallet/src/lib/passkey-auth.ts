// Passkey sign-in helpers: discoverable-credential assertion, and local verification of an assertion against a P-256
// public key with WebCrypto (no extra dependency). Used to match a passkey to an account's on-chain signers.
import { type Hex, bytesToHex, concat, hexToBytes, sha256, toHex } from 'viem'
import type { SignerStruct } from './signers.ts'
import { type P256PublicKey, base64UrlEncode, parseDerSignature } from './webauthn.ts'

const buf = (u: Uint8Array): Uint8Array<ArrayBuffer> => new Uint8Array(u)

export type DiscoveredPasskey = {
  credentialId: string
  rpId: string
  authenticatorData: Hex
  clientDataJSON: string
  /** DER-decoded signature components (not low-s normalized; verification accepts both). */
  r: bigint
  s: bigint
}

/** Asks the platform for ANY passkey of this site (discoverable credential, user verification required). */
export async function discoverPasskey(rpId: string): Promise<DiscoveredPasskey> {
  if (!globalThis.navigator?.credentials) throw new Error('Passkeys are not available in this browser')
  const cred = (await navigator.credentials.get({
    publicKey: {
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      rpId,
      userVerification: 'required',
      timeout: 120_000,
    },
  })) as PublicKeyCredential | null
  if (!cred) throw new Error('Sign-in was cancelled')
  const r = cred.response as AuthenticatorAssertionResponse
  const sig = parseDerSignature(new Uint8Array(r.signature))
  return {
    credentialId: base64UrlEncode(new Uint8Array(cred.rawId)),
    rpId,
    authenticatorData: bytesToHex(new Uint8Array(r.authenticatorData)),
    clientDataJSON: new TextDecoder().decode(r.clientDataJSON),
    r: sig.r,
    s: sig.s,
  }
}

/** True iff (r, s) is a valid P-256 signature by `key` over sha256(authenticatorData || sha256(clientDataJSON)). */
export async function verifyAssertion(
  key: P256PublicKey,
  a: Pick<DiscoveredPasskey, 'authenticatorData' | 'clientDataJSON' | 'r' | 's'>,
): Promise<boolean> {
  try {
    const raw = hexToBytes(concat(['0x04', padTo32(key.x), padTo32(key.y)]))
    const pub = await crypto.subtle.importKey('raw', buf(raw), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify'])
    const message = hexToBytes(concat([a.authenticatorData, sha256(toHex(new TextEncoder().encode(a.clientDataJSON)))]))
    const sig = hexToBytes(concat([padTo32(toHex(a.r)), padTo32(toHex(a.s))]))
    return await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, pub, buf(sig), buf(message))
  } catch {
    return false
  }
}

function padTo32(h: Hex): Hex {
  return `0x${h.slice(2).padStart(64, '0')}` as Hex
}

/** Returns the passkey signer of `signers` that produced the assertion, if any. */
export async function matchPasskeySigner(
  signers: readonly SignerStruct[],
  a: Pick<DiscoveredPasskey, 'authenticatorData' | 'clientDataJSON' | 'r' | 's'>,
): Promise<P256PublicKey | null> {
  for (const s of signers) {
    if (Number(s.kind) !== 2) continue
    const key = { x: s.x, y: s.y }
    if (await verifyAssertion(key, a)) return key
  }
  return null
}

/** RP ID sanity check: WebAuthn requires the RP ID to equal the page's host or be a registrable suffix of it. */
export function rpIdMatchesHost(rpId: string, host: string): boolean {
  const r = rpId.toLowerCase()
  const h = host.toLowerCase()
  return h === r || h.endsWith(`.${r}`)
}
