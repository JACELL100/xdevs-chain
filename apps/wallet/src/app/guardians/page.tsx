import type { Metadata } from 'next'
import { Guardians } from '../../views/Guardians.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'Guardians', description: 'Guardians, quorum, freeze and recovery.' }

export default function Page() {
  return (
    <Gate needsAccount={true} offlineOk={false}>
      <Guardians />
    </Gate>
  )
}
