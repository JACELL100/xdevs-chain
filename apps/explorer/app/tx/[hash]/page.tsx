import type { Metadata } from 'next'
import { TxView } from '../../../src/views/TxView'

// Static export: one placeholder page ("_"). public/_redirects serves it for every tx URL and the view reads the real
// value from window.location.
export const dynamicParams = false
export function generateStaticParams() {
  return [{ hash: '_' }]
}

export const metadata: Metadata = { title: 'Transaction' }

export default function Page() {
  return <TxView />
}
