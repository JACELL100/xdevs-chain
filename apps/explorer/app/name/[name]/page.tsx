import type { Metadata } from 'next'
import { NameView } from '../../../src/views/NameView'

// Static export: one placeholder page ("_"). public/_redirects serves it for every name URL and the view reads the real
// value from window.location.
export const dynamicParams = false
export function generateStaticParams() {
  return [{ name: '_' }]
}

export const metadata: Metadata = { title: '.xdevs name' }

export default function Page() {
  return <NameView />
}
