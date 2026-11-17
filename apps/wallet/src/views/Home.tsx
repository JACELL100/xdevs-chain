'use client'
import { useWallet } from '../wallet/WalletProvider.tsx'
import { Dashboard } from './Dashboard.tsx'
import { Onboarding } from './Onboarding.tsx'

/** "/": the dashboard when this device has an account, otherwise onboarding. */
export function Home() {
  const { rec } = useWallet()
  return rec ? <Dashboard /> : <Onboarding />
}
