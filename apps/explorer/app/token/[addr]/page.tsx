import type { Metadata } from 'next'
import { TokenView } from '../../../src/views/TokenView'

// Static export: one placeholder page ("_"). public/_redirects serves it for every token URL and the view reads the real
// value from window.location.
export const dynamicParams = false
export function generateStaticParams() {
  return [{ addr: '_' }]
}

export const metadata: Metadata = { title: 'Token' }

export default function Page() {
  return <TokenView />
}
