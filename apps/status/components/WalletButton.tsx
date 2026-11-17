'use client'

import { AddToWallet, type AddToWalletProps } from '@xdevs/ui'
import { config } from '@/lib/config'

/** The kit's AddToWallet adds Enclave's parameters, so it is only offered when this build watches Enclave. */
export default function WalletButton(props: Omit<AddToWalletProps, 'label'>) {
  if (!config.isEnclave) return null
  return <AddToWallet label="Add Enclave to wallet" {...props} />
}
