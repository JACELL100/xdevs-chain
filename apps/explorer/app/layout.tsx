import type { Metadata, Viewport } from 'next'
import { Geist, Geist_Mono } from 'next/font/google'
import '@xdevs/ui/styles.css'
import './explorer.css'
import Shell from './Shell'

// Same fonts as xdevschain.tech: next/font exposes them as CSS variables that @xdevs/ui/tokens.css reads.
const geist = Geist({ subsets: ['latin'], variable: '--font-geist', display: 'swap' })
const geistMono = Geist_Mono({ subsets: ['latin'], variable: '--font-geist-mono', display: 'swap' })

export const metadata: Metadata = {
  metadataBase: new URL('https://explorer.xdevschain.tech'),
  title: { default: 'xDEVS Explorer · Enclave testnet', template: '%s · xDEVS Explorer' },
  description:
    'Block explorer for xDEVS Chain Enclave: live blocks with BFT commit certificates, transactions, addresses, XDRC-20 tokens, validators and .xdevs names.',
  applicationName: 'xDEVS Explorer',
  authors: [{ name: 'THE xDEVS', url: 'https://xdevs.in' }],
  icons: { icon: [{ url: '/favicon.svg', type: 'image/svg+xml' }] },
  openGraph: { title: 'xDEVS Explorer', description: 'Explore xDEVS Chain Enclave, the public testnet.', images: ['https://xdevschain.tech/og.png'] },
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
