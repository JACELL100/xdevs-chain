// Owner signers of an xDEVS smart account: WebAuthn passkeys (platform authenticator), a software P-256 key
// (tests and the dev-only "virtual passkey"), and secp256k1 keys (dev panel / EIP-7702 EOA).
import { type Hex, bytesToHex, concat, hexToBytes, sha256, toHex } from 'viem'
import type { LocalAccount } from 'viem/accounts'
import {
  type P256PublicKey,
  base64UrlDecode,
  base64UrlEncode,
  buildAuthenticatorData,
  buildClientDataJSON,
  dummyPasskeySignature,
  encodePasskeySignature,
  parseDerSignature,
  parseRawSignature,
  spkiToPublicKey,
} from './webauthn.ts'

/** Copies into a fresh ArrayBuffer-backed view (WebCrypto / WebAuthn `BufferSource` typing). */
const buf = (u: Uint8Array): Uint8Array<ArrayBuffer> => new Uint8Array(u)

export type OwnerSigner = {
  kind: 'passkey' | 'ecdsa'
  label: string
  /** Signature over a 32-byte userOpHash / replay-safe hash, encoded as the account expects. */
  signHash(hash: Hex): Promise<Hex>
  /** Same-shape placeholder used for gas estimation. */
  dummySignature(): Hex
}

/** Signer descriptor stored on the account (`Signer{kind, x, y}`; kind 1 = ECDSA, 2 = Passkey). */
export type SignerStruct = { kind: number; x: Hex; y: Hex }

export function passkeySignerStruct(key: P256PublicKey): SignerStruct {
  return { kind: 2, x: key.x, y: key.y }
}

export function ecdsaSignerStruct(address: Hex): SignerStruct {
  return { kind: 1, x: `0x${address.slice(2).toLowerCase().padStart(64, '0')}` as Hex, y: `0x${'0'.repeat(64)}` }
}

// ------------------------------------------------------------------------------------------ platform passkey
export type StoredPasskey = { credentialId: string; publicKey: P256PublicKey; rpId: string; createdAt: number }

/** Creates a WebAuthn P-256 passkey (user verification required). Browser only. */
export async function createPasskey(userName: string, rpName = 'xDEVS Wallet'): Promise<StoredPasskey> {
  if (!globalThis.navigator?.credentials) throw new Error('WebAuthn is not available in this browser')
  const userId = crypto.getRandomValues(new Uint8Array(16))
  const cred = (await navigator.credentials.create({
    publicKey: {
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      rp: { name: rpName, id: location.hostname },
      user: { id: userId, name: userName, displayName: userName },
      pubKeyCredParams: [{ type: 'public-key', alg: -7 }], // ES256 = P-256 only (the chain's P256VERIFY)
      authenticatorSelection: { residentKey: 'preferred', userVerification: 'required' },
      attestation: 'none',
      timeout: 120_000,
    },
  })) as PublicKeyCredential | null
  if (!cred) throw new Error('passkey creation was cancelled')
  const resp = cred.response as AuthenticatorAttestationResponse
  if (typeof resp.getPublicKey !== 'function') throw new Error('this browser cannot export the passkey public key')
  if (resp.getPublicKeyAlgorithm?.() !== -7) throw new Error('the authenticator did not create a P-256 (ES256) key')
  const spki = resp.getPublicKey()
  if (!spki) throw new Error('the authenticator returned no public key')
  return {
    credentialId: base64UrlEncode(new Uint8Array(cred.rawId)),
    publicKey: spkiToPublicKey(new Uint8Array(spki)),
    rpId: location.hostname,
    createdAt: Date.now(),
  }
}

/** Signer backed by the platform authenticator (Face ID / Touch ID / Windows Hello / security key). */
export function platformPasskeySigner(p: StoredPasskey): OwnerSigner {
  return {
    kind: 'passkey',
    label: 'Passkey',
    dummySignature: () => dummyPasskeySignature(p.publicKey),
    async signHash(hash: Hex) {
      const cred = (await navigator.credentials.get({
        publicKey: {
          challenge: buf(hexToBytes(hash)),
          rpId: p.rpId,
          allowCredentials: [{ type: 'public-key', id: buf(base64UrlDecode(p.credentialId)) }],
          userVerification: 'required',
          timeout: 120_000,
        },
      })) as PublicKeyCredential | null
      if (!cred) throw new Error('signing was cancelled')
      const r = cred.response as AuthenticatorAssertionResponse
      const { r: sr, s } = parseDerSignature(new Uint8Array(r.signature))
      return encodePasskeySignature(
        p.publicKey,
        {
          authenticatorData: bytesToHex(new Uint8Array(r.authenticatorData)),
          clientDataJSON: new TextDecoder().decode(r.clientDataJSON),
          r: sr,
          s,
        },
        hash,
      )
    },
  }
}

// ------------------------------------------------------------------------------------------ software P-256
/**
 * A P-256 key held in WebCrypto that produces real WebAuthn-format assertions (authenticatorData with UP|UV,
 * clientDataJSON with the challenge). Stands in for an authenticator in tests; in the UI it is the DEV-ONLY
 * "virtual passkey" (the private key sits in browser storage — no better than a hot key).
 */
export class SoftPasskey {
  readonly publicKey: P256PublicKey
  private readonly key: CryptoKey
  private counter = 0
  readonly rpId: string
  readonly origin: string

  private constructor(key: CryptoKey, publicKey: P256PublicKey, rpId: string, origin: string) {
    this.key = key
    this.publicKey = publicKey
    this.rpId = rpId
    this.origin = origin
  }

  static async generate(rpId = 'localhost', origin = 'http://localhost:5173'): Promise<SoftPasskey> {
    const pair = (await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify'])) as CryptoKeyPair
    const jwk = await crypto.subtle.exportKey('jwk', pair.privateKey)
    return SoftPasskey.fromJwk(jwk, rpId, origin)
  }

  static async fromJwk(jwk: JsonWebKey, rpId = 'localhost', origin = 'http://localhost:5173'): Promise<SoftPasskey> {
    const key = await crypto.subtle.importKey('jwk', { ...jwk, key_ops: ['sign'] }, { name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign'])
    const publicKey = {
      x: bytesToHex(base64UrlDecode(jwk.x as string)),
      y: bytesToHex(base64UrlDecode(jwk.y as string)),
    }
    return new SoftPasskey(key, publicKey, rpId, origin)
  }

  async exportJwk(): Promise<JsonWebKey> {
    return crypto.subtle.exportKey('jwk', this.key)
  }

  /** Raw WebAuthn assertion over `challenge` (signature not yet low-s normalized, like a real authenticator). */
  async assert(challenge: Hex) {
    const authenticatorData = buildAuthenticatorData(this.rpId, 0x05, ++this.counter)
    const clientDataJSON = buildClientDataJSON(challenge, this.origin)
    const message = concat([authenticatorData, sha256(toHex(new TextEncoder().encode(clientDataJSON)))])
    const sig = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, this.key, buf(hexToBytes(message))))
    return { authenticatorData, clientDataJSON, ...parseRawSignature(sig) }
  }

  signer(label = 'Virtual passkey (dev only)'): OwnerSigner {
    return {
      kind: 'passkey',
      label,
      dummySignature: () => dummyPasskeySignature(this.publicKey, this.origin),
      signHash: async (hash: Hex) => encodePasskeySignature(this.publicKey, await this.assert(hash), hash),
    }
  }
}

// ------------------------------------------------------------------------------------------ secp256k1
/** ECDSA owner (raw 65-byte signature over the hash, no EIP-191 prefix — `SignatureVerifier._verifyEcdsa`). */
export function ecdsaSigner(account: LocalAccount, label = 'Key'): OwnerSigner {
  return {
    kind: 'ecdsa',
    label,
    dummySignature: () => `0x${'11'.repeat(64)}1b` as Hex,
    signHash: async (hash: Hex) => {
      if (!account.sign) throw new Error('this account cannot sign raw hashes')
      return account.sign({ hash })
    },
  }
}
