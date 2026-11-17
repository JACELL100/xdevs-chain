import type { Metadata } from 'next'
import { BlockView } from '../../../src/views/BlockView'

// Static export: one placeholder page ("_"). public/_redirects serves it for every block URL and the view reads the real
// value from window.location.
export const dynamicParams = false
export function generateStaticParams() {
  return [{ n: '_' }]
}

export const metadata: Metadata = { title: 'Block' }

export default function Page() {
  return <BlockView />
}
