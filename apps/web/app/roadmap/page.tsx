import type { Metadata } from 'next'
import RoadmapRail from '@/components/RoadmapRail'
import { Cta, PageHead } from '@/components/Sections'
import { Arrow } from '@/components/icons'
import { LINKS, docs } from '@/lib/site'
import { rv } from '@/lib/rv'

export const metadata: Metadata = {
  title: 'Roadmap',
  description:
    'The xDEVS Chain roadmap: phases 0 to 5 are built and the Enclave testnet is live. Independent validators, audits, light-client bridges and mainnet preparation come next. No dates promised.',
  alternates: { canonical: '/roadmap' },
}

export default function RoadmapPage() {
  return (
    <>
      <section className="page-top" aria-labelledby="road-title">
        <div className="wrap">
          <PageHead
            eyebrow="Roadmap"
            title={
              <span id="road-title">
                <span className="metal">Built in phases,</span>
                <br />
                <span className="quiet">each with exit criteria.</span>
              </span>
            }
            lead="Every phase ends with a test it must pass, not a date. The project is built by a small team, so we don’t promise when. We do say exactly what is live, what is built and what comes next."
          >
            <div className="btns" style={{ marginTop: 8 }}>
              <a className="btn is-ghost" href={docs('/roadmap')}>
                Detailed roadmap in the docs <Arrow />
              </a>
              <a className="btn is-ghost" href={LINKS.plan}>
                Implementation plan
              </a>
            </div>
          </PageHead>
        </div>
      </section>

      <section className="section" aria-label="Phases">
        <div className="wrap" style={{ maxWidth: 1080 }}>
          <RoadmapRail detailed />
        </div>
      </section>

      <section className="section is-joined" aria-labelledby="mainnet-title">
        <div className="wrap">
          <div className="card" style={{ padding: 'clamp(28px, 4vw, 56px)' }} {...rv(0, 'scale')}>
            <p className="eyebrow">Mainnet</p>
            <h2 id="mainnet-title" className="h2" style={{ marginTop: 14 }}>
              Coming next. <span className="quiet">No date, no token sale.</span>
            </h2>
            <p className="lead" style={{ marginTop: 18 }}>
              There is no mainnet and no token sale. Testnet tXDS has no value and will not convert to mainnet XDS. Mainnet comes
              after independent validators, external audits and a long stretch of public testnet operation.
            </p>
          </div>
        </div>
      </section>

      <Cta />
    </>
  )
}
