import type { Metadata } from 'next'
import { SearchView } from '../../src/views/ListViews'

export const metadata: Metadata = { title: 'Search' }

export default function SearchPage() {
  return <SearchView />
}
