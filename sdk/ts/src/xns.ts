import { type Hash, labelhash, namehash } from 'viem'
import { InvalidXnsNameError } from './errors.ts'

/** Top-level suffix of every XNS name (plan §1.4). */
export const XNS_SUFFIX = '.xdevs'
/** Mirrors `XNSLabel.MIN_LENGTH`. */
export const XNS_MIN_LABEL_LENGTH = 3
/** Mirrors `XNSLabel.MAX_LENGTH`. */
export const XNS_MAX_LABEL_LENGTH = 63

/** Mirrors `XNSLabel.Invalid` (minus `None`). */
export type XnsLabelError = 'too-short' | 'too-long' | 'forbidden-character' | 'bad-hyphen'

/**
 * Checks a single label against the on-chain rules of `contracts/src/periphery/xns/XNSLabel.sol`:
 * 3..63 chars of `[a-z0-9-]`, no leading/trailing hyphen, no `--` (which also rules out punycode `xn--`).
 * Returns `null` if the label is registrable. Does NOT lowercase — use {@link normalizeXnsName} for input.
 */
export function checkXnsLabel(label: string): XnsLabelError | null {
  if (label.length < XNS_MIN_LABEL_LENGTH) return 'too-short'
  if (label.length > XNS_MAX_LABEL_LENGTH) return 'too-long'
  for (let i = 0; i < label.length; i++) {
    const c = label[i]!
    if (c === '-') {
      if (i === 0 || i === label.length - 1 || label[i - 1] === '-') return 'bad-hyphen'
    } else if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'))) {
      return 'forbidden-character'
    }
  }
  return null
}

/** True iff `label` is a normalized, registrable `.xdevs` label. */
export function isValidXnsLabel(label: string): boolean {
  return checkXnsLabel(label) === null
}

const LABEL_ERROR_TEXT: Record<XnsLabelError, string> = {
  'too-short': `labels must be at least ${XNS_MIN_LABEL_LENGTH} characters`,
  'too-long': `labels must be at most ${XNS_MAX_LABEL_LENGTH} characters`,
  'forbidden-character': 'only a-z, 0-9 and "-" are allowed',
  'bad-hyphen': 'a hyphen cannot start or end a label or follow another hyphen',
}

/**
 * Normalizes user input to a canonical XNS name: trims whitespace, lowercases ASCII, appends `.xdevs`
 * to a bare label and validates every label. Throws {@link InvalidXnsNameError} instead of "fixing"
 * anything else — in particular any non-ASCII character is rejected (homograph protection).
 *
 * `"Alice"` → `"alice.xdevs"`, `"pay.alice.xdevs"` → `"pay.alice.xdevs"`, `"alice.eth"` → throws.
 */
export function normalizeXnsName(input: string): string {
  // eslint-disable-next-line no-control-regex
  if (/[^\x00-\x7f]/.test(input)) throw new InvalidXnsNameError(input, 'non-ASCII characters are not allowed')
  let name = input.trim().toLowerCase()
  if (!name.includes('.')) name += XNS_SUFFIX
  if (!name.endsWith(XNS_SUFFIX)) throw new InvalidXnsNameError(input, `name must end with "${XNS_SUFFIX}"`)
  const labels = name.slice(0, -XNS_SUFFIX.length).split('.')
  for (const label of labels) {
    const error = checkXnsLabel(label)
    if (error) throw new InvalidXnsNameError(input, `label ${JSON.stringify(label)}: ${LABEL_ERROR_TEXT[error]}`)
  }
  return name
}

/** True if {@link normalizeXnsName} accepts `input`. */
export function isValidXnsName(input: string): boolean {
  try {
    normalizeXnsName(input)
    return true
  } catch {
    return false
  }
}

/** ENS/EIP-137 namehash of a normalized XNS name (the `node` used by `XNSRegistry`). */
export function xnsNamehash(name: string): Hash {
  return namehash(normalizeXnsName(name))
}

/** keccak256 of a single (validated) label. */
export function xnsLabelhash(label: string): Hash {
  const error = checkXnsLabel(label)
  if (error) throw new InvalidXnsNameError(label, LABEL_ERROR_TEXT[error])
  return labelhash(label)
}

/**
 * Confusable "skeleton" of a label, mirroring `XNSLabel.skeleton` (before hashing): greedily maps
 * `0→o`, `1→l`, `rn→m`, `vv→w`, `cl→d`. Two labels with the same skeleton look (nearly) identical.
 */
export function xnsSkeleton(label: string): string {
  let out = ''
  for (let i = 0; i < label.length; i++) {
    const c = label[i]!
    const n = label[i + 1]
    if (c === '0') out += 'o'
    else if (c === '1') out += 'l'
    else if (c === 'r' && n === 'n') (out += 'm'), i++
    else if (c === 'v' && n === 'v') (out += 'w'), i++
    else if (c === 'c' && n === 'l') (out += 'd'), i++
    else out += c
  }
  return out
}

/** True if two different labels share a confusable skeleton (e.g. `paypa1` vs `paypal`). */
export function areConfusableXnsLabels(a: string, b: string): boolean {
  return a !== b && xnsSkeleton(a) === xnsSkeleton(b)
}
