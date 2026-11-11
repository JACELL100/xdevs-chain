import FinalityStage from './FinalityStage'
import { rv } from './util'
import './finality.css'

export default function Finality() {
  return (
    <section id="finality" className="xh-section s-fin" aria-labelledby="finality-title">
      <div className="xh-wrap">
        <header className="head">
          <p className="xh-label" {...rv()}>
            <b>01</b>
            <i>//</i> finality
          </p>
          <h2 id="finality-title" className="xh-h2" {...rv(60)}>
            <span className="xh-metal">Final in one block.</span>
            <br />
            <span className="xh-quiet">No reorgs.</span>
          </h2>
          <p className="xh-lede" {...rv(120)}>
            Tendermint-style BFT consensus (Malachite). A block is final the moment validators holding more than
            two-thirds of the voting power commit it. Nothing to wait out, nothing to roll back.
          </p>
        </header>

        <FinalityStage />

        <div className="facts">
          <div {...rv()}>
            <h3>Final on commit</h3>
            <p className="xh-body">
              A committed block never reverts. Every block carries a stored commit certificate you can query over RPC.
            </p>
          </div>
          <div {...rv(80)}>
            <h3>One validator down? Still final.</h3>
            <p className="xh-body">
              Tested: with 1 of 4 validators offline the chain keeps finalizing. With 2 of 4 it halts safely and
              resumes, with no forks.
            </p>
          </div>
          <div {...rv(160)}>
            <h3>Execute, then vote</h3>
            <p className="xh-body">
              Validators execute each block before voting on it, so a committed block always comes with the state it
              produced.
            </p>
          </div>
        </div>
      </div>
    </section>
  )
}
