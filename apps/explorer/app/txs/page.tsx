import type { Metadata } from 'next'
import { TxsView } from '../../src/views/ListViews'

export const metadata: Metadata = { title: 'Transactions' }

export default function TxsPage() {
  return <TxsView />
}
