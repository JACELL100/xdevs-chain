import { formatEther, parseEther } from 'viem'

/** Decimals of XDS / tXDS. */
export const XDS_DECIMALS = 18
/** 1 XDS = 10^18 dev (the smallest unit, plan §1.4). */
export const DEV_PER_XDS = 10n ** 18n

/** Parses a decimal XDS amount to dev: `parseXds('1.5')` → `1500000000000000000n`. */
export function parseXds(xds: string): bigint {
  return parseEther(xds)
}

/** Formats an amount in dev as a decimal XDS string: `formatXds(10n ** 18n)` → `"1"`. */
export function formatXds(dev: bigint): string {
  return formatEther(dev)
}
