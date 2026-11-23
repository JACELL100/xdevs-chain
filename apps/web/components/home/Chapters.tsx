'use client'

/**
 * The safety features as scroll-driven chapters (Apple-style sticky scrollytelling):
 * the phone stays pinned while the chapters scroll past, and its screen changes with
 * the chapter that crosses the middle of the viewport. On narrow screens every chapter
 * carries its own phone instead.
 */
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { AnimatePresence, m } from 'motion/react'
import { Phone } from '@/components/Devices'
import { Check } from '@/components/icons'
import { docs } from '@/lib/site'
import { FinalityScreen, NamesScreen } from './Screens'

type Chapter = {
  id: string
  eyebrow: string
  title: ReactNode
  body: string
  points: string[]
  img?: string
  alt?: string
  screen?: 'names' | 'finality'
  tone?: 'alert'
  more: string
}

const CHAPTERS: Chapter[] = [
  {
    id: 'passkeys',
    eyebrow: 'Passkey accounts',
    title: 'No seed phrase. Not one word to write down.',
    body: 'Your account is a smart contract you unlock with Face ID, a fingerprint or Windows Hello. The key stays on your device, and activation is sponsored, so a new account starts without buying coins first.',
    points: ['Passkey signatures verified on-chain (P-256)', 'Recovery through guardians you choose', 'Runs in the browser, nothing to install'],
    img: '/img/phone-wallet-start.webp',
    alt: 'The xDEVS wallet start screen: create a passkey for wallet.xdevschain.tech',
    more: '/concepts/smart-accounts',
  },
  {
    id: 'vault',
    eyebrow: 'Guardian Vault',
    title: 'A stolen phone can’t empty your account.',
    body: 'Everyday payments stay instant. Anything above your limit, every token approval and every call to an unknown contract waits out a delay, 24 hours by default. You or your guardians can cancel it in that window.',
    points: ['You set the instant limit per transfer and per day', 'Turning protection off waits out the same delay', 'Never reverses a finalized transaction'],
    img: '/img/phone-wallet-vault.webp',
    alt: 'Guardian Vault in the wallet demo: two queued actions with countdowns and cancel buttons',
    more: '/concepts/guardian-vault',
  },
  {
    id: 'guardians',
    eyebrow: 'Guardians, freeze and recovery',
    title: 'Lose your device. Keep your account.',
    body: 'Pick friends, a second device or a hardware key as guardians. Together they can cancel queued transfers, freeze the account and recover it to a new passkey. No company, validator or vote can do that for them.',
    points: ['A quorum you choose, for example 2 of 3', 'Freeze ends an owner-versus-thief stalemate', 'No admin key over anyone’s funds'],
    img: '/img/phone-wallet-guardians.webp',
    alt: 'Guardians screen in the wallet demo: three guardians, two of three must agree, freeze and recovery status',
    more: '/concepts/guardian-vault#guardian-freeze',
  },
  {
    id: 'checks',
    eyebrow: 'Safety checks',
    title: 'See what you’re signing before you sign it.',
    body: 'The network simulates the transaction and checks the recipient before you confirm. You see what moves, which approvals it sets, and a plain warning when an address only looks like one you paid before.',
    points: ['Simulation: asset changes and the vault outcome', 'Recipient checks: first-time and look-alike addresses', 'Shown as advice, never as a promise of safety'],
    img: '/img/phone-wallet-lookalike.webp',
    alt: 'Send review in the wallet: a red warning that the address looks like one used before but is different',
    tone: 'alert',
    more: '/concepts/safety-layer#facts-and-advice',
  },
  {
    id: 'approvals',
    eyebrow: 'Expiring approvals',
    title: 'Approvals that end on their own.',
    body: 'A forgotten unlimited approval can be drained years later. XDRC-20 tokens and the Permit2Expiring contract put an expiry on every allowance, and the wallet shows them all with a one-tap revoke.',
    points: ['Every Permit2Expiring allowance has an expiry', 'Unlimited and never-expiring approvals are flagged', 'Revoke one, or revoke all'],
    img: '/img/phone-wallet-approvals.webp',
    alt: 'Approvals screen in the wallet demo: an unlimited approval flagged as never expiring, others expiring in days',
    more: '/concepts/approvals',
  },
  {
    id: 'names',
    eyebrow: '.xdevs names',
    title: 'Names that refuse to be faked.',
    body: 'XNS turns a long address into a name like aria.xdevs and rejects names that look like one that already exists. While paypal.xdevs exists, nobody can register paypa1.xdevs.',
    points: ['Only a to z, 0 to 9 and hyphens: no look-alike Unicode', '0, 1, rn, vv and cl count as o, l, m, w and d', 'The first registrant wins'],
    screen: 'names',
    more: '/concepts/xns',
  },
  {
    id: 'finality',
    eyebrow: 'One-block finality',
    title: 'Final in one block. About two seconds.',
    body: 'Malachite BFT consensus commits each block once validators holding more than two thirds of the voting power have signed it. A committed block is never reorganised, so a payment is done the moment it lands.',
    points: ['2-second blocks on Enclave', 'No reorgs after commit', 'Commit certificates readable over RPC'],
    screen: 'finality',
    more: '/guide/why',
  },
]

function Screen({ c, eager }: { c: Chapter; eager?: boolean }) {
  if (c.screen === 'names') return <Phone><NamesScreen /></Phone>
  if (c.screen === 'finality') return <Phone><FinalityScreen /></Phone>
  return <Phone src={c.img} alt={c.alt} eager={eager} />
}

export default function Chapters() {
  const [active, setActive] = useState(0)
  const refs = useRef<(HTMLElement | null)[]>([])

  useEffect(() => {
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) setActive(Number((e.target as HTMLElement).dataset.i))
      },
      { rootMargin: '-48% 0px -48% 0px' },
    )
    refs.current.forEach((el) => el && io.observe(el))
    return () => io.disconnect()
  }, [])

  const c = CHAPTERS[active]
  return (
    <div className="chapters">
      <div className="rail">
        {CHAPTERS.map((ch, i) => (
          <article
            key={ch.id}
            id={ch.id}
            data-i={i}
            ref={(el) => {
              refs.current[i] = el
            }}
            className={`chapter${i === active ? ' is-active' : ''}`}
            aria-labelledby={`${ch.id}-t`}
          >
            <p className="eyebrow">
              <span className="num">{String(i + 1).padStart(2, '0')}</span> {ch.eyebrow}
            </p>
            <h3 id={`${ch.id}-t`} className="h2">
              {ch.title}
            </h3>
            <p className="lead">{ch.body}</p>
            <ul className="points">
              {ch.points.map((p) => (
                <li key={p}>
                  <Check />
                  {p}
                </li>
              ))}
            </ul>
            <p>
              <a className="text-cta" href={docs(ch.more)}>
                How it works
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
                  <path d="M5 12h14M13 6l6 6-6 6" />
                </svg>
              </a>
            </p>
            <div className="mphone">
              <Screen c={ch} />
            </div>
          </article>
        ))}
      </div>
      <div className="stage" aria-hidden="true">
        <div className={`stage-glow${c.tone === 'alert' ? ' is-alert' : ''}`} />
        <div className="progress">
          {CHAPTERS.map((ch, i) => (
            <i key={ch.id} className={i === active ? 'is-on' : ''} />
          ))}
        </div>
        <AnimatePresence mode="popLayout" initial={false}>
          <m.div
            key={c.id}
            style={{ height: '100%', display: 'grid', placeItems: 'center' }}
            initial={{ opacity: 0, y: 40, scale: 0.94, rotateX: 8 }}
            animate={{ opacity: 1, y: 0, scale: 1, rotateX: 0 }}
            exit={{ opacity: 0, y: -40, scale: 0.96 }}
            transition={{ duration: 0.7, ease: [0.2, 0.8, 0.2, 1] }}
          >
            <Screen c={c} eager={active === 0} />
          </m.div>
        </AnimatePresence>
      </div>
    </div>
  )
}
