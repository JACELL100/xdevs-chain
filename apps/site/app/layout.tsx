import type { Metadata, Viewport } from 'next'
import { Geist, Geist_Mono } from 'next/font/google'
import { SITE_DESCRIPTION, SITE_NAME, SITE_URL, TAGLINE } from '@/components/site/nav'
import './globals.css'

const geist = Geist({ subsets: ['latin'], variable: '--font-geist', display: 'swap' })
const geistMono = Geist_Mono({ subsets: ['latin'], variable: '--font-geist-mono', display: 'swap' })

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: {
    default: `${SITE_NAME}: ${TAGLINE.toLowerCase()}`,
    template: `%s · ${SITE_NAME}`,
  },
  description: SITE_DESCRIPTION,
  applicationName: SITE_NAME,
  authors: [{ name: 'THE xDEVS', url: 'https://xdevs.in' }],
  icons: {
    icon: [{ url: '/favicon.svg', type: 'image/svg+xml' }],
    other: [{ rel: 'mask-icon', url: '/brand/xdevs-chain-mark-solid.svg', color: '#05060a' }],
  },
  openGraph: {
    type: 'website',
    siteName: SITE_NAME,
    title: `${SITE_NAME}: ${TAGLINE.toLowerCase()}`,
    description: 'An EVM-equivalent PoS Layer 1 with user protection built into the chain. Built by THE xDEVS.',
    url: SITE_URL,
    images: [{ url: '/og.png', width: 1200, height: 630, alt: `${SITE_NAME}: ${TAGLINE.toLowerCase()}` }],
  },
  twitter: {
    card: 'summary_large_image',
    title: `${SITE_NAME}: ${TAGLINE.toLowerCase()}`,
    description: 'An EVM-equivalent PoS Layer 1 with user protection built into the chain. Built by THE xDEVS.',
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
        {/* Scroll-reveal content is only hidden when JS is there to reveal it. */}
        <script dangerouslySetInnerHTML={{ __html: "document.documentElement.classList.add('xh-js')" }} />
      </head>
      <body>{children}</body>
    </html>
  )
}
