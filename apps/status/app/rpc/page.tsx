// What a browser sees when it opens https://rpc.xdevschain.tech directly: the coordinator redirects
// browser GETs on the RPC host here (JSON-RPC POSTs keep going to the node).

import type { Metadata } from 'next'
import { Button, Card, CopyButton, ExternalIcon } from '@xdevs/ui'
import WalletButton from '@/components/WalletButton'
import ChainParams from '@/components/ChainParams'
import RpcLiveCheck from '@/components/RpcLiveCheck'
import { config, docsLinks } from '@/lib/config'

export const metadata: Metadata = {
  title: 'JSON-RPC endpoint',
  description: `The public JSON-RPC endpoint of ${config.networkName}: ${config.rpcUrl}. Add it to your wallet or point your tooling at it.`,
}

const CURL = `curl -s ${config.rpcUrl} \\
  -H 'content-type: application/json' \\
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}'`

const CAST = `cast chain-id --rpc-url ${config.rpcUrl}
cast balance 0xYourAddress --ether --rpc-url ${config.rpcUrl}`

const VIEM = `import { createPublicClient, http } from 'viem'
import { xdevsEnclave } from '@xdevs/sdk'

const client = createPublicClient({ chain: xdevsEnclave, transport: http() })
const block = await client.getBlockNumber()`

function Code({ label, code }: { label: string; code: string }) {
  return (
    <figure className="st-code">
      <figcaption>
        <span>{label}</span>
        <CopyButton value={code} label="Copy" />
      </figcaption>
      <pre>
        <code>{code}</code>
      </pre>
    </figure>
  )
}

const LIMITS: [string, string][] = [
  ['Batch size', '50 requests'],
  ['eth_getLogs', '10,000 blocks / 10,000 results'],
  ['eth_call gas cap', '50,000,000'],
  ['WebSocket subscriptions', '20 per connection'],
  ['Disabled namespaces', 'debug_, txpool_, admin_'],
]

export default function RpcPage() {
  return (
    <div className="xu-page st-page">
      <header className="st-rpc-hero">
        <p className="xh-label">
          <b>JSON-RPC</b>
          <i>/</i>
          {config.networkName}
        </p>
        <h1 className="st-rpc-title">This is the JSON-RPC endpoint for {config.networkName}.</h1>
        <p className="st-rpc-lede">
          You opened it in a browser tab, but it is meant for machines: it answers JSON-RPC over HTTPS POST. Use it in your
          wallet, or point Foundry, Hardhat, viem or ethers at it.
        </p>
        <div className="st-rpc-url">
          <code>{config.rpcUrl}</code>
          <CopyButton value={config.rpcUrl} label="Copy URL" />
        </div>
        <div className="st-rpc-actions">
          <WalletButton variant="primary" size="md" />
          <Button href="/" variant="ghost" size="md">
            Network status
          </Button>
          <RpcLiveCheck />
        </div>
      </header>

      <div className="st-row st-row-rpc">
        <Card title="Network parameters">
          <ChainParams />
        </Card>
        <Card title="Try it">
          <div className="st-codes">
            <Code label="curl" code={CURL} />
            <Code label="Foundry (cast)" code={CAST} />
            <Code label="viem + @xdevs/sdk" code={VIEM} />
          </div>
        </Card>
      </div>

      <div className="st-row st-row-even">
        <Card title="Public endpoint limits">
          <dl className="st-params">
            {LIMITS.map(([k, v]) => (
              <div key={k}>
                <dt>{k}</dt>
                <dd>{v}</dd>
              </div>
            ))}
          </dl>
          <p className="st-note">Requests are rate limited per IP. For heavy indexing, run your own full node.</p>
        </Card>
        <Card title="What it speaks">
          <p className="st-note st-note-top">
            The standard Ethereum API (<span className="xh-mono">eth_</span>, <span className="xh-mono">net_</span>,{' '}
            <span className="xh-mono">web3_</span>) plus the <span className="xh-mono">xdevs_</span> namespace, for example{' '}
            <span className="xh-mono">xdevs_getCommitCertificate</span> and <span className="xh-mono">xdevs_getEoaLock</span>.
            WebSocket subscriptions live at <span className="xh-mono">{config.wsUrl}</span>.
          </p>
          <div className="st-pre-links">
            <Button href={docsLinks.jsonRpc} external variant="ghost" size="sm" icon={<ExternalIcon />}>
              JSON-RPC reference
            </Button>
            <Button href={docsLinks.endpoints} external variant="quiet" size="sm" icon={<ExternalIcon />}>
              All endpoints
            </Button>
          </div>
          <p className="st-warn-line">
            {config.currencyName} ({config.symbol}) has no monetary value. Never buy it; get it free from the{' '}
            <a className="xh-textlink" href={config.faucetUrl}>
              faucet
            </a>
            .
          </p>
        </Card>
      </div>
    </div>
  )
}
