'use client'

import { Badge } from '@xdevs/ui'
import { useEffect, useState } from 'react'
import { config } from '@/lib/config'
import { rpcBatch } from '@/lib/rpc'
import { hexToNumber } from '@/lib/stats'

type Check =
  | { kind: 'checking' }
  | { kind: 'live'; chainId: number | null; block: number; ms: number }
  | { kind: 'down' }

/** One-shot eth_chainId + eth_blockNumber from the visitor's browser. */
export default function RpcLiveCheck() {
  const [c, setC] = useState<Check>({ kind: 'checking' })
  useEffect(() => {
    if (config.demo) {
      setC({ kind: 'live', chainId: 717171, block: 1_289_342, ms: 84 })
      return
    }
    let alive = true
    rpcBatch(config.rpcUrl, [{ method: 'eth_chainId' }, { method: 'eth_blockNumber' }], 6000)
      .then(({ results, latencyMs }) => {
        const block = results[1]?.ok ? hexToNumber(results[1].value) : null
        if (!alive) return
        if (block === null) setC({ kind: 'down' })
        else setC({ kind: 'live', chainId: results[0]?.ok ? hexToNumber(results[0].value) : null, block, ms: latencyMs })
      })
      .catch(() => alive && setC({ kind: 'down' }))
    return () => {
      alive = false
    }
  }, [])

  if (c.kind === 'checking') {
    return (
      <Badge tone="dim" dot>
        checking the endpoint…
      </Badge>
    )
  }
  if (c.kind === 'down') {
    return (
      <Badge tone="dim" dot>
        {config.launched ? 'not answering right now' : 'not live yet: launches with Enclave'}
      </Badge>
    )
  }
  return (
    <Badge tone="final" dot>
      live · block #{c.block.toLocaleString('en-US')} · {Math.round(c.ms)} ms{config.demo ? ' · demo' : ''}
    </Badge>
  )
}
