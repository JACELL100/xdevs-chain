import type { Metadata } from 'next'
import { Onboarding } from '../../views/Onboarding.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'Create or sign in', description: 'Create a passkey wallet with no seed phrase, or sign in with your passkey.' }

export default function Page() {
  return (
    <Gate needsAccount={false} offlineOk={false}>
      <Onboarding />
    </Gate>
  )
}
