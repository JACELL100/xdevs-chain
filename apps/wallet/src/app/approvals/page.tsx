import type { Metadata } from 'next'
import { Approvals } from '../../views/Approvals.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'Approvals', description: 'Live token approvals with expiry, and one-tap revoke.' }

export default function Page() {
  return (
    <Gate needsAccount={true} offlineOk={false}>
      <Approvals />
    </Gate>
  )
}
