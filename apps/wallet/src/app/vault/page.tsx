import type { Metadata } from 'next'
import { Vault } from '../../views/Vault.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'Guardian Vault', description: 'Queued actions with countdown and cancel, vault policy and trusted addresses.' }

export default function Page() {
  return (
    <Gate needsAccount={true} offlineOk={false}>
      <Vault />
    </Gate>
  )
}
