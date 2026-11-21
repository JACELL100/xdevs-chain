'use client'
import { useState } from 'react'
import { useWallet } from '../wallet/WalletProvider.tsx'
import { Dashboard } from './Dashboard.tsx'
import { Onboarding } from './Onboarding.tsx'

/**
 * "/": the dashboard when this device has an account, otherwise onboarding. Onboarding saves
 * a new account before its "Your wallet is ready" step and holds this view until the user
 * opens the wallet, so that step is shown.
 */
export function Home() {
  const { rec } = useWallet()
  const [held, setHeld] = useState(false)
  return rec && !held ? <Dashboard /> : <Onboarding hold={setHeld} />
}
