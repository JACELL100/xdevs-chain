import { Hash } from '@xdevs/ui'
import { config, docsLinks, PLANNED } from '@/lib/config'

/** Network parameters as a definition list (used on the dashboard and on /rpc). */
export default function ChainParams({ compact = false }: { compact?: boolean }) {
  const rows: [string, React.ReactNode][] = [
    ['Network', config.networkName],
    [
      'Chain id',
      <span key="cid" className="xh-num">
        {config.chainId} <span className="xh-quiet">({config.chainIdHex})</span>
      </span>,
    ],
    ['Currency', `${config.currencyName} (${config.symbol}), 18 decimals`],
    ['JSON-RPC', <Hash key="rpc" value={config.rpcUrl} full />],
    ['WebSocket', <Hash key="ws" value={config.wsUrl} full />],
    [
      'Explorer',
      <a key="ex" className="xh-textlink" href={config.explorerUrl} target="_blank" rel="noopener">
        {config.explorerUrl.replace(/^https?:\/\//, '')}
      </a>,
    ],
  ]
  if (!compact) {
    rows.push(['Block time', `${PLANNED.blockTimeSeconds} s target`])
    rows.push(['Finality', PLANNED.finality])
    rows.push([
      'Docs',
      <a key="docs" className="xh-textlink" href={docsLinks.jsonRpc} target="_blank" rel="noopener">
        JSON-RPC reference
      </a>,
    ])
  }
  return (
    <dl className="st-params">
      {rows.map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  )
}
