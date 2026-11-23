import type { MetadataRoute } from 'next'
import { SITE_URL } from '@/lib/site'

export const dynamic = 'force-static'

const PAGES: [string, number][] = [
  ['', 1],
  ['/enclave', 0.9],
  ['/wallet', 0.9],
  ['/developers', 0.9],
  ['/security', 0.7],
  ['/roadmap', 0.7],
  ['/brand', 0.4],
]

export default function sitemap(): MetadataRoute.Sitemap {
  return PAGES.map(([p, priority]) => ({ url: `${SITE_URL}${p}`, changeFrequency: 'weekly', priority }))
}
