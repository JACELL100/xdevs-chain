import type { Metadata } from 'next'
import { AddressView } from '../../../src/views/AddressView'

// Static export: one placeholder page ("_"). public/_redirects serves it for every address URL and the view reads the real
// value from window.location.
export const dynamicParams = false
export function generateStaticParams() {
  return [{ addr: '_' }]
}

export const metadata: Metadata = { title: 'Address' }

export default function Page() {
  return <AddressView />
}
