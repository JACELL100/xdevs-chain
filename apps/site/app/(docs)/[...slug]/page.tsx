import type { Metadata } from 'next'
import Link from 'next/link'
import { notFound } from 'next/navigation'
import { getAllSlugs, getDoc } from '@/lib/docs/content'
import DocContent from '@/components/docs/DocContent'
import Toc from '@/components/docs/Toc'

type Props = { params: Promise<{ slug: string[] }> }

export const dynamicParams = false

export function generateStaticParams() {
  return getAllSlugs().map((slug) => ({ slug }))
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const doc = await getDoc((await params).slug)
  if (!doc) return {}
  const title = `${doc.title} · xDEVS Chain`
  return {
    title: { absolute: title },
    description: doc.description,
    alternates: { canonical: doc.href },
    openGraph: {
      type: 'article',
      siteName: 'xDEVS Chain',
      title,
      description: doc.description,
      url: doc.href,
      images: [{ url: '/og.png', width: 1200, height: 630, alt: 'xDEVS Chain: the chain that has your back' }],
    },
    twitter: { card: 'summary_large_image', title, description: doc.description, images: ['/og.png'] },
  }
}

function Arrow({ dir }: { dir: 'left' | 'right' }) {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <path
        d={dir === 'left' ? 'M9.5 3.5 5 8l4.5 4.5' : 'M6.5 3.5 11 8l-4.5 4.5'}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

export default async function DocPage({ params }: Props) {
  const doc = await getDoc((await params).slug)
  if (!doc) notFound()

  return (
    <div className="d-page">
      <article className="d-article" data-pagefind-body data-pagefind-meta={`section:${doc.group}`}>
        <nav className="d-crumbs" aria-label="Breadcrumb" data-pagefind-ignore="all">
          <ol>
            <li>
              <Link href="/guide/introduction">Docs</Link>
            </li>
            <li>
              <span>{doc.group}</span>
            </li>
            <li aria-current="page">{doc.navTitle}</li>
          </ol>
        </nav>

        {doc.toc.length > 1 ? (
          <details className="d-toc-mobile" data-pagefind-ignore="all">
            <summary>
              On this page
              <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                <path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
              </svg>
            </summary>
            <ul>
              {doc.toc.map((t) => (
                <li key={t.id} data-depth={t.depth}>
                  <a href={`#${t.id}`}>{t.text}</a>
                </li>
              ))}
            </ul>
          </details>
        ) : null}

        <div className="d-prose">
          <DocContent tree={doc.tree} />
        </div>

        <footer className="d-article-foot" data-pagefind-ignore="all">
          <a className="d-edit" href={doc.editUrl} target="_blank" rel="noopener noreferrer">
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path
                d="M10.6 2.6a1.4 1.4 0 0 1 2 0l.8.8a1.4 1.4 0 0 1 0 2L6 12.8l-3 .7.7-3 6.9-7.9Z"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.3"
                strokeLinejoin="round"
              />
            </svg>
            Edit this page on GitHub
          </a>

          <nav className="d-pager" aria-label="Previous and next page">
            {doc.prev ? (
              <Link className="d-pager-link" data-dir="prev" href={doc.prev.href} rel="prev">
                <span className="d-pager-label">
                  <Arrow dir="left" /> Previous
                </span>
                <span className="d-pager-title">{doc.prev.text}</span>
              </Link>
            ) : (
              <span />
            )}
            {doc.next ? (
              <Link className="d-pager-link" data-dir="next" href={doc.next.href} rel="next">
                <span className="d-pager-label">
                  Next <Arrow dir="right" />
                </span>
                <span className="d-pager-title">{doc.next.text}</span>
              </Link>
            ) : null}
          </nav>
        </footer>
      </article>

      <aside className="d-aside" data-pagefind-ignore="all">
        <div className="d-aside-inner">
          <Toc items={doc.toc} />
          <div className="d-aside-links">
            <a href={doc.editUrl} target="_blank" rel="noopener noreferrer">
              Edit on GitHub
            </a>
            <a href="#main">Back to top</a>
          </div>
        </div>
      </aside>
    </div>
  )
}
