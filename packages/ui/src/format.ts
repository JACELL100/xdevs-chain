/** Shortens a 0x-hex string to `0x1234…abcd` (`head` chars after 0x, `tail` chars at the end). */
export function truncateHex(value: string, head = 4, tail = 4): string {
  if (!value) return ''
  const prefixed = value.startsWith('0x') || value.startsWith('0X')
  const body = prefixed ? value.slice(2) : value
  if (body.length <= head + tail + 1) return value
  return `${prefixed ? '0x' : ''}${body.slice(0, head)}…${body.slice(-tail)}`
}

/** Joins class names, skipping falsy values. */
export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(' ')
}

/** Formats an integer with thin grouping (1,234,567). Accepts number | bigint | numeric string. */
export function formatInt(value: number | bigint | string): string {
  try {
    return BigInt(value).toLocaleString('en-US')
  } catch {
    return String(value)
  }
}
