import { ENCLAVE } from '@xdevs/ui'
import FaucetApp from '@/components/FaucetApp'

export default function Page() {
  return (
    <div className="xu-page fx-page">
      <header className="fx-hero">
        <p className="xh-label">
          <b>{ENCLAVE.name}</b>
          <i>/</i>
          testnet faucet
        </p>
        <h1 className="fx-title">
          Free <span className="xh-metal">tXDS</span> for building on Enclave.
        </h1>
        <p className="fx-lede">
          Claim {ENCLAVE.nativeCurrency.name} to deploy contracts, test smart accounts and try Guardian Vault on the public
          testnet. One claim every 24 hours.
        </p>
      </header>
      <FaucetApp />
    </div>
  )
}
