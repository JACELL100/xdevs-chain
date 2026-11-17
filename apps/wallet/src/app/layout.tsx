import type { Metadata, Viewport } from 'next'
import { Geist, Geist_Mono } from 'next/font/google'
import '@xdevs/ui/styles.css'
import './wallet.css'
import Shell from './Shell'

// Same fonts as xdevschain.tech: next/font exposes them as CSS variables that @xdevs/ui/tokens.css reads.
const geist = Geist({ subsets: ['latin'], variable: '--font-geist', display: 'swap' })
const geistMono = Geist_Mono({ subsets: ['latin'], variable: '--font-geist-mono', display: 'swap' })

export const metadata: Metadata = {
  metadataBase: new URL('https://wallet.xdevschain.tech'),
  title: { default: 'Wallet · xDEVS Chain Enclave', template: '%s · xDEVS Wallet' },
  description:
    'The xDEVS Chain wallet: a passkey smart account with no seed phrase, a Guardian Vault that delays large transfers, guardians, expiring approvals and .xdevs names. Enclave testnet.',
  applicationName: 'xDEVS Wallet',
  authors: [{ name: 'THE xDEVS', url: 'https://xdevs.in' }],
  icons: { icon: [{ url: '/favicon.svg', type: 'image/svg+xml' }] },
  robots: { index: true, follow: false },
  referrer: 'no-referrer',
  formatDetection: { telephone: false, address: false, email: false },
}

export const viewport: Viewport = { themeColor: '#05060a', colorScheme: 'dark', width: 'device-width', initialScale: 1 }

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${geist.variable} ${geistMono.variable}`}>
      <body>
        <Shell>{children}</Shell>
      </body>
    </html>
  )
}
