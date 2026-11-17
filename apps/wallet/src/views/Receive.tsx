'use client'
// Receive: address (grouped for checking), .xdevs name, QR code, faucet link.
import { Button, CopyButton } from '@xdevs/ui'
import Link from 'next/link'
import { IconDrop, IconTag } from '../components/icons.tsx'
import { QrCode } from '../components/QrCode.tsx'
import { AddressFull, DemoTag, Notice, Page } from '../components/ui.tsx'
import { config, explorerAddress, faucetLink } from '../config.ts'
import { useWallet } from '../wallet/WalletProvider.tsx'

export function Receive() {
  const { rec, primaryName, demo } = useWallet()
  if (!rec) return null
  return (
    <Page title={<>Receive {demo ? <DemoTag /> : null}</>} sub={`Share your address or name to receive ${config.symbol} and tokens on ${config.chainName}.`} narrow>
      <section className="w-panel w-receive">
        <div className="w-receive-qr">
          <QrCode value={rec.address} label={`QR code of the address ${rec.address}`} size={232} />
        </div>
        <div className="w-receive-info">
          {primaryName ? (
            <div className="w-receive-name">
              <span className="w-label">Name</span>
              <p>
                {primaryName}
                <CopyButton value={primaryName} title="Copy name" />
              </p>
            </div>
          ) : null}
          <span className="w-label">Address</span>
          <AddressFull address={rec.address} className="is-large" />
          <div className="w-flow-actions is-left">
            <CopyButton value={rec.address} label="Copy address" />
            <Button variant="ghost" size="sm" href={explorerAddress(rec.address)} external>
              View in explorer
            </Button>
          </div>
          <Notice tone="info">
            Only send assets on {config.chainName} (chain {config.chainId}). When you paste the address somewhere, check the first and last four characters
            (highlighted above).
          </Notice>
        </div>
      </section>
      <div className="w-dash-foot">
        <a className="w-tile" href={faucetLink(rec.address)} target="_blank" rel="noopener noreferrer">
          <IconDrop />
          <div>
            <b>Get test {config.symbol}</b>
            <span>Free from the Enclave faucet. Test tokens have no value.</span>
          </div>
        </a>
        {!primaryName ? (
          <Link className="w-tile" href="/names">
            <IconTag />
            <div>
              <b>Claim a .xdevs name</b>
              <span>Easier to share than an address, and look-alike proof.</span>
            </div>
          </Link>
        ) : null}
      </div>
    </Page>
  )
}
