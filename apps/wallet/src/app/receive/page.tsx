import type { Metadata } from 'next'
import { Receive } from '../../views/Receive.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'Receive', description: 'Your address, .xdevs name and QR code.' }

export default function Page() {
  return (
    <Gate needsAccount={true} offlineOk={false}>
      <Receive />
    </Gate>
  )
}
