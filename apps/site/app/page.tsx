import type { Metadata } from 'next'
import Header from '@/components/site/Header'
import Footer from '@/components/site/Footer'
import RevealObserver from '@/components/site/RevealObserver'
import Hero from '@/components/home/Hero'
import Finality from '@/components/home/Finality'
import Vault from '@/components/home/Vault'
import Passkeys from '@/components/home/Passkeys'
import Safety from '@/components/home/Safety'
import Evm from '@/components/home/Evm'
import Proof from '@/components/home/Proof'
import Roadmap from '@/components/home/Roadmap'
import Closing from '@/components/home/Closing'
import '@/components/home/landing.css'

export const metadata: Metadata = {
  title: { absolute: 'xDEVS Chain: the chain that has your back' },
  description:
    'xDEVS Chain is an EVM-equivalent Proof-of-Stake Layer 1 with user protection built into the chain. Passkey accounts, Guardian Vault, expiring approvals and look-alike-proof .xdevs names. Built by THE xDEVS.',
  alternates: { canonical: '/' },
}

export default function Home() {
  return (
    <>
      <Header variant="landing" />
      <main id="main" className="xh">
        <Hero />
        <Finality />
        <Vault />
        <Passkeys />
        <Safety />
        <Evm />
        <Proof />
        <Roadmap />
        <Closing />
      </main>
      <Footer />
      <RevealObserver />
    </>
  )
}
