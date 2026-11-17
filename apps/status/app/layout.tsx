import type { Metadata, Viewport } from 'next'
import { Geist, Geist_Mono } from 'next/font/google'
import '@xdevs/ui/styles.css'
import './status.css'
import Shell from './Shell'

// Same fonts as xdevschain.tech: next/font exposes them as CSS variables that @xdevs/ui/tokens.css reads.
const geist = Geist({ subsets: ['latin'], variable: '--font-geist', display: 'swap' })
const geistMono = Geist_Mono({ subsets: ['latin'], variable: '--font-geist-mono', display: 'swap' })

const DESCRIPTION =
  'Live status of the xDEVS Chain Enclave testnet: block production, finality, validators, RPC, WebSocket, explorer, faucet and bundler.'

export const metadata: Metadata = {
  metadataBase: new URL('https://status.xdevschain.tech'),
  title: { default: 'Enclave Status · xDEVS Chain', template: '%s · xDEVS Chain Status' },
  description: DESCRIPTION,
  applicationName: 'xDEVS Chain Status',
  authors: [{ name: 'THE xDEVS', url: 'https://xdevs.in' }],
  icons: { icon: [{ url: '/favicon.svg', type: 'image/svg+xml' }] },
  openGraph: {
    type: 'website',
    siteName: 'xDEVS Chain',
    title: 'Enclave Status · xDEVS Chain',
    description: DESCRIPTION,
    url: 'https://status.xdevschain.tech',
    images: [{ url: 'https://xdevschain.tech/og.png', width: 1200, height: 630, alt: 'xDEVS Chain' }],
  },
}

export const viewport: Viewport = { themeColor: '#05060a', colorScheme: 'dark' }

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${geist.variable} ${geistMono.variable}`}>
      <body>
        <Shell>{children}</Shell>
      </body>
    </html>
  )
}
