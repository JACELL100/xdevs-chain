import type { Metadata } from 'next'
import { EoaLock } from '../../views/EoaLock.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'EOA lock', description: 'Protect an existing MetaMask address with EIP-7702 and the XIP-0001 EOA lock.' }

export default function Page() {
  return (
    <Gate needsAccount={false} offlineOk={false}>
      <EoaLock />
    </Gate>
  )
}
