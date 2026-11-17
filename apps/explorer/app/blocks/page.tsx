import type { Metadata } from 'next'
import { BlocksView } from '../../src/views/ListViews'

export const metadata: Metadata = { title: 'Blocks' }

export default function BlocksPage() {
  return <BlocksView />
}
