// Web3 Secret Storage v3 keystore decryption (geth / Foundry `cast wallet new` format) with node:crypto only.
import { createDecipheriv, pbkdf2Sync, scryptSync, timingSafeEqual } from 'node:crypto'
import { type Hex, concat, keccak256, toHex } from 'viem'

type KeystoreV3 = {
  version?: number
  crypto?: KeystoreCrypto
  Crypto?: KeystoreCrypto
}
type KeystoreCrypto = {
  cipher: string
  cipherparams: { iv: string }
  ciphertext: string
  kdf: 'scrypt' | 'pbkdf2'
  kdfparams: { dklen: number; salt: string; n?: number; r?: number; p?: number; c?: number; prf?: string }
  mac: string
}

const hexBuf = (h: string) => Buffer.from(h.replace(/^0x/, ''), 'hex')

/** Returns the private key (`0x…`) of a v3 keystore. Throws "wrong password" on a MAC mismatch. */
export function decryptKeystore(json: KeystoreV3, password: string): Hex {
  const c = json.crypto ?? json.Crypto
  if (json.version !== 3 || !c) throw new Error('keystore: only Web3 Secret Storage version 3 is supported')
  if (c.cipher !== 'aes-128-ctr') throw new Error(`keystore: unsupported cipher ${c.cipher}`)
  const kp = c.kdfparams
  const salt = hexBuf(kp.salt)
  let derived: Buffer
  if (c.kdf === 'scrypt') {
    const N = kp.n ?? 0
    const r = kp.r ?? 8
    const p = kp.p ?? 1
    derived = scryptSync(Buffer.from(password.normalize('NFKC'), 'utf8'), salt, kp.dklen, {
      N,
      r,
      p,
      maxmem: 128 * N * r * p + 64 * 1024 * 1024,
    })
  } else if (c.kdf === 'pbkdf2') {
    if (kp.prf !== 'hmac-sha256') throw new Error(`keystore: unsupported prf ${kp.prf}`)
    derived = pbkdf2Sync(Buffer.from(password.normalize('NFKC'), 'utf8'), salt, kp.c ?? 0, kp.dklen, 'sha256')
  } else {
    throw new Error(`keystore: unsupported kdf ${String(c.kdf)}`)
  }
  const ciphertext = hexBuf(c.ciphertext)
  const mac = hexBuf(keccak256(concat([toHex(derived.subarray(16, 32)), toHex(ciphertext)])))
  const expected = hexBuf(c.mac)
  if (mac.length !== expected.length || !timingSafeEqual(mac, expected)) throw new Error('keystore: wrong password')
  const decipher = createDecipheriv('aes-128-ctr', derived.subarray(0, 16), hexBuf(c.cipherparams.iv))
  const key = Buffer.concat([decipher.update(ciphertext), decipher.final()])
  if (key.length !== 32) throw new Error('keystore: unexpected key length')
  return toHex(key)
}
