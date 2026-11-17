import type { Metadata } from 'next'
import { Send } from '../../views/Send.tsx'
import { Gate } from '../../wallet/Gate.tsx'

export const metadata: Metadata = { title: 'Send', description: 'Send tXDS and tokens with recipient checks, a simulation preview and the Guardian Vault.' }

export default function Page() {
  return (
    <Gate needsAccount={true} offlineOk={false}>
      <Send />
    </Gate>
  )
}
