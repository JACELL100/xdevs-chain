// Client-side recipient validation. Mirrors apps/faucet/src/address.ts so the user
// sees the same verdict before spending a captcha solve; the Worker re-checks everything.

const HEX_ADDRESS = /^0x[0-9a-fA-F]{40}$/
/** XNS names: label(s) + ".xdevs" (same pattern as the Worker). */
const XNS_NAME = /^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+xdevs$/
const SYSTEM_CALLER = '0xfffffffffffffffffffffffffffffffffffffffe'
const RESERVED_LOW_MAX = 0xffffn

export type RecipientCheck =
  | { ok: true; kind: 'address'; value: `0x${string}` }
  | { ok: true; kind: 'name'; value: string }
  | { ok: false; empty: true; message: '' }
  | { ok: false; empty: false; message: string }

export function checkRecipient(raw: string): RecipientCheck {
  const input = raw.trim()
  if (input.length === 0) return { ok: false, empty: true, message: '' }
  if (input.length > 256) return { ok: false, empty: false, message: 'That is too long to be an address or a .xdevs name.' }
  const lower = input.toLowerCase()
  if (lower.endsWith('.xdevs')) {
    if (!XNS_NAME.test(lower)) {
      return { ok: false, empty: false, message: 'That is not a valid .xdevs name. Use letters, digits and hyphens, e.g. alice.xdevs.' }
    }
    return { ok: true, kind: 'name', value: lower }
  }
  if (!lower.startsWith('0x')) {
    return { ok: false, empty: false, message: 'Addresses start with 0x. Names end with .xdevs.' }
  }
  if (!HEX_ADDRESS.test(input)) {
    const body = input.slice(2)
    if (/[^0-9a-fA-F]/.test(body)) return { ok: false, empty: false, message: 'An address uses only 0–9 and a–f after 0x.' }
    return {
      ok: false,
      empty: false,
      message: `An address has 40 hex characters after 0x; this one has ${body.length}.`,
    }
  }
  const value = BigInt(lower)
  if (value === 0n) return { ok: false, empty: false, message: 'The zero address cannot receive tokens.' }
  if (value <= RESERVED_LOW_MAX) {
    return { ok: false, empty: false, message: 'That is a system contract or precompile address, not a wallet.' }
  }
  if (lower === SYSTEM_CALLER) return { ok: false, empty: false, message: 'That is the protocol system address, not a wallet.' }
  return { ok: true, kind: 'address', value: lower as `0x${string}` }
}

/** 0x1234…abcd */
export function shortHex(hex: string, head = 6, tail = 4): string {
  return hex.length <= head + tail + 1 ? hex : `${hex.slice(0, head)}…${hex.slice(-tail)}`
}
