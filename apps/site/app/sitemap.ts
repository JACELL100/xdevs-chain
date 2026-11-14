import type { MetadataRoute } from 'next'
import { DOCS_FLAT } from '@/lib/docs/nav'

export const dynamic = 'force-static'

const SITE = 'https://xdevschain.tech'

export default function sitemap(): MetadataRoute.Sitemap {
  return [
    { url: SITE, changeFrequency: 'weekly', priority: 1 },
    ...DOCS_FLAT.map((d) => ({
      url: `${SITE}${d.href}`,
      changeFrequency: 'weekly' as const,
      priority: d.href.startsWith('/guide/') ? 0.9 : 0.7,
    })),
  ]
}
