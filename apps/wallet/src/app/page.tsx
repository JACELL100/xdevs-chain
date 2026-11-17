import { Home } from '../views/Home.tsx'
import { Gate } from '../wallet/Gate.tsx'

export default function Page() {
  return (
    <Gate needsAccount={false}>
      <Home />
    </Gate>
  )
}
