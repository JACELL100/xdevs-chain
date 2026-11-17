'use client'
import { useMemo } from 'react'
import type { Address } from 'viem'
import { config } from '../config.ts'
import { demoLabel, demoTokenMeta } from '../data/demo.ts'
import type { Asset } from '../data/types.ts'
import type { Describer } from '../lib/describe.ts'
import { useWallet } from './WalletProvider.tsx'

/** Labels and token metadata for human-readable call descriptions. */
export function useDescriber(assets: Asset[] | null = null): Describer {
  const { demo, rec, primaryName } = useWallet()
  return useMemo<Describer>(() => {
    const tokens = new Map((assets ?? []).filter((a) => a.token).map((a) => [(a.token as Address).toLowerCase(), { symbol: a.symbol, decimals: a.decimals }]))
    return {
      symbol: config.symbol,
      label: (a) => {
        if (rec && a.toLowerCase() === rec.address.toLowerCase()) return primaryName ?? 'you'
        return demo ? demoLabel(a) : null
      },
      token: (a) => tokens.get(a.toLowerCase()) ?? (demo ? demoTokenMeta(a) : null),
    }
  }, [assets, demo, rec, primaryName])
}
