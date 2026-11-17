import type { Metadata } from 'next'
import { Settings } from '../../views/Settings.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'Settings', description: 'Network, this device, and public account information.' }

export default function Page() {
  return (
    <Gate needsAccount={false} offlineOk={true}>
      <Settings />
    </Gate>
  )
}
