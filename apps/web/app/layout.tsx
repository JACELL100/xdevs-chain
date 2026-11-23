import type { Metadata, Viewport } from 'next'
import { Geist, Geist_Mono } from 'next/font/google'
import Header from '@/components/Header'
import Footer from '@/components/Footer'
import MotionRuntime from '@/components/MotionRuntime'
import Providers from '@/components/Providers'
import { SITE_DESCRIPTION, SITE_NAME, SITE_URL } from '@/lib/site'
import './globals.css'

const geist = Geist({ subsets: ['latin'], variable: '--font-geist', display: 'swap' })
const geistMono = Geist_Mono({ subsets: ['latin'], variable: '--font-geist-mono', display: 'swap' })

const TITLE = 'xDEVS Chain: the chain that has your back'

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: { default: TITLE, template: '%s · xDEVS Chain' },
  description: SITE_DESCRIPTION,
  applicationName: SITE_NAME,
  authors: [{ name: 'THE xDEVS', url: 'https://xdevs.in' }],
  alternates: { canonical: '/' },
  icons: {
    icon: [{ url: '/favicon.svg', type: 'image/svg+xml' }],
    apple: [{ url: '/brand/xdevs-chain-icon-256.png' }],
  },
  openGraph: {
    type: 'website',
    siteName: SITE_NAME,
    title: TITLE,
    description: 'An EVM-equivalent Layer 1 with user protection built into the chain. The Enclave public testnet is live.',
    url: SITE_URL,
    images: [{ url: '/og.png', width: 1200, height: 630, alt: TITLE }],
  },
  twitter: {
    card: 'summary_large_image',
    title: TITLE,
    description: 'An EVM-equivalent Layer 1 with user protection built into the chain. The Enclave public testnet is live.',
    images: ['/og.png'],
  },
}

export const viewport: Viewport = {
  themeColor: '#05060a',
  colorScheme: 'dark',
}

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${geist.variable} ${geistMono.variable}`} suppressHydrationWarning>
      <head>
        {/* Reveal animations only hide content when JS is there to show it again. */}
        <script dangerouslySetInnerHTML={{ __html: "document.documentElement.classList.add('js')" }} />
      </head>
      <body>
        <Providers>
          <Header />
          <main id="main" tabIndex={-1}>
            {children}
          </main>
          <Footer />
        </Providers>
        <MotionRuntime />
      </body>
    </html>
  )
}
