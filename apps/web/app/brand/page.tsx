import type { Metadata } from 'next'
import Mark from '@/components/Mark'
import { PageHead } from '@/components/Sections'
import { Download } from '@/components/icons'
import { rv } from '@/lib/rv'

export const metadata: Metadata = {
  title: 'Brand assets',
  description: 'Download the xDEVS Chain mark and logos, and read how to use them: clear space, minimum sizes and colours.',
  alternates: { canonical: '/brand' },
}

const LOGOS: { name: string; file: string; light?: boolean; use: string }[] = [
  { name: 'Mark', file: 'xdevs-chain-mark.svg', use: 'Silver, for dark backgrounds' },
  { name: 'Mark, ink', file: 'xdevs-chain-mark-ink.svg', light: true, use: 'Graphite, for light backgrounds' },
  { name: 'Mark, solid', file: 'xdevs-chain-mark-solid.svg', light: true, use: 'Below 32 px, currentColor' },
  { name: 'Logo', file: 'xdevs-chain-logo.svg', use: 'Horizontal lockup, dark backgrounds' },
  { name: 'Logo, ink', file: 'xdevs-chain-logo-ink.svg', light: true, use: 'Horizontal lockup, light backgrounds' },
  { name: 'Logo, stacked', file: 'xdevs-chain-logo-stacked.svg', use: 'Stacked lockup for square spaces' },
  { name: 'App icon', file: 'xdevs-chain-icon-256.png', use: 'PNG, 256 × 256, for wallets and listings' },
  { name: 'Favicon', file: 'favicon.svg', use: 'Mark on a graphite tile' },
  { name: 'Stacked, ink', file: 'xdevs-chain-logo-stacked-ink.svg', light: true, use: 'Stacked lockup, light backgrounds' },
]

const COLORS = [
  { n: 'Canvas', v: '#05060A', bg: '#05060a' },
  { n: 'Surface', v: '#0B0D12', bg: '#0b0d12' },
  { n: 'Text', v: '#F3F5F8', bg: '#f3f5f8' },
  { n: 'Muted', v: '#8F96A1', bg: '#8f96a1' },
  { n: 'Finality teal', v: '#4FE3CF', bg: '#4fe3cf' },
  { n: 'Platinum', v: '#FFFFFF → #7F8792', bg: 'linear-gradient(180deg,#fff,#eef1f4 42%,#a9b0ba)' },
]

export default function BrandPage() {
  return (
    <>
      <section className="page-top" aria-labelledby="brand-title">
        <div className="wrap">
          <div className="hero-split">
            <div className="hero-copy">
              <PageHead
                eyebrow="Brand"
                title={
                  <span id="brand-title">
                    <span className="metal">Two links,</span>
                    <br />
                    <span className="quiet">one X.</span>
                  </span>
                }
                lead="The mark is a > and a < from the THE xDEVS X, drawn as hollow links and interlocked the way two chain links hold each other. It reads as code brackets, as a chain, and as the parent company’s X."
              />
            </div>
            <div style={{ width: 'min(320px, 60vw)', margin: '0 auto' }} {...rv(200, 'scale')}>
              <Mark uid="brand" animate />
            </div>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="files-title">
        <div className="wrap">
          <header className="head">
            <h2 id="files-title" className="h2" {...rv()}>
              Logo files
            </h2>
            <p className="lead" {...rv(60)}>
              SVG unless noted. The wordmark is outlined, so no font is needed.
            </p>
          </header>
          <div className="grid-3">
            {LOGOS.map((l, i) => {
              const href = l.file === 'favicon.svg' ? '/favicon.svg' : `/brand/${l.file}`
              return (
                <div key={l.file} className="card logo-tile" {...rv((i % 3) * 60)}>
                  <div className={`art${l.light ? ' is-light' : ''}`}>
                    <img src={href} alt={`${l.name} preview`} loading="lazy" style={l.file.includes('solid') ? { color: '#0d1015' } : undefined} />
                  </div>
                  <div className="meta">
                    <div>
                      <strong>{l.name}</strong>
                      <br />
                      <span>{l.use}</span>
                    </div>
                    <a className="copy-btn" href={href} download aria-label={`Download ${l.name}`}>
                      <Download />
                      Download
                    </a>
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="rules-title">
        <div className="wrap">
          <div className="split" style={{ alignItems: 'start' }}>
            <header className="copy">
              <h2 id="rules-title" className="h2" {...rv()}>
                Use it well.
              </h2>
            </header>
            <ul className="checks" {...rv(80)}>
              {[
                'Keep clear space around the symbol of one quarter of its height on every side.',
                'Hollow mark at 24 px or larger. Below that, use the solid mark (down to 16 px).',
                'Horizontal lockup at least 20 px tall.',
                'Don’t recolour the metallic gradient, add effects, rotate, stretch or separate the two links.',
                'Don’t put the silver mark on light or busy backgrounds. Use the ink or mono files.',
              ].map((t) => (
                <li key={t}>
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
                    <path d="m5 12.5 4.5 4.5L19 7.5" />
                  </svg>
                  {t}
                </li>
              ))}
            </ul>
          </div>
        </div>
      </section>

      <section className="section is-tight" aria-labelledby="colors-title">
        <div className="wrap">
          <header className="head">
            <h2 id="colors-title" className="h2" {...rv()}>
              Colour
            </h2>
            <p className="lead" {...rv(60)}>
              Platinum on near-black, with one accent. Finality teal marks what is final, safe or live, and is used sparingly. Type
              is Geist and Geist Mono.
            </p>
          </header>
          <div className="grid-3">
            {COLORS.map((c, i) => (
              <div key={c.n} className="swatch" {...rv((i % 3) * 60)}>
                <div className="chipc" style={{ background: c.bg, borderBottom: '1px solid var(--line)' }} />
                <div className="info">
                  {c.n}
                  <code>{c.v}</code>
                </div>
              </div>
            ))}
          </div>
        </div>
      </section>
    </>
  )
}
