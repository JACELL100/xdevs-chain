import type { Metadata } from 'next'
import { Names } from '../../views/Names.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'Names', description: 'Claim a look-alike-proof .xdevs name.' }

export default function Page() {
  return (
    <Gate needsAccount={true} offlineOk={false}>
      <Names />
    </Gate>
  )
}
