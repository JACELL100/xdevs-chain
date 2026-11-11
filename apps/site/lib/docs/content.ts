import fs from 'node:fs'
import path from 'node:path'
import { cache } from 'react'
import { DOCS_FLAT, DOCS_NAV, EDIT_BASE } from './nav'
import { renderMarkdown, type RenderedDoc } from './markdown'

/** Markdown sources: content/<path>.md is served at /<path>. */
const CONTENT_DIR = path.join(process.cwd(), 'content')

export type DocPage = RenderedDoc & {
  slug: string[]
  href: string
  file: string
  editUrl: string
  group: string
  navTitle: string
  prev?: { text: string; href: string }
  next?: { text: string; href: string }
}

function walk(dir: string, prefix: string[] = []): string[][] {
  const out: string[][] = []
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) out.push(...walk(path.join(dir, entry.name), [...prefix, entry.name]))
    else if (entry.isFile() && entry.name.endsWith('.md')) out.push([...prefix, entry.name.slice(0, -3)])
  }
  return out
}

/** Every docs page, as route segments. Fails the build if content and sidebar disagree. */
export const getAllSlugs = cache((): string[][] => {
  const slugs = walk(CONTENT_DIR)
  const files = new Set(slugs.map((s) => '/' + s.join('/')))
  const nav = new Set(DOCS_FLAT.map((i) => i.href))
  const missing = [...files].filter((f) => !nav.has(f))
  const orphan = [...nav].filter((n) => !files.has(n))
  if (missing.length || orphan.length) {
    throw new Error(
      `Docs sidebar (lib/docs/nav.ts) and content/ disagree. Not in sidebar: ${missing.join(', ') || '-'}; no file: ${orphan.join(', ') || '-'}`,
    )
  }
  return DOCS_FLAT.map((i) => i.href.slice(1).split('/'))
})

export const getDoc = cache(async (slug: string[]): Promise<DocPage | undefined> => {
  const href = '/' + slug.join('/')
  const idx = DOCS_FLAT.findIndex((i) => i.href === href)
  if (idx === -1 || slug.some((s) => !/^[a-z0-9-]+$/.test(s))) return undefined
  const rel = slug.join('/') + '.md'
  const source = fs.readFileSync(path.join(CONTENT_DIR, rel), 'utf8')
  const rendered = await renderMarkdown(source, href)
  const item = DOCS_FLAT[idx]
  const prev = DOCS_FLAT[idx - 1]
  const next = DOCS_FLAT[idx + 1]
  return {
    ...rendered,
    title: rendered.title || item.text,
    slug,
    href,
    file: `content/${rel}`,
    editUrl: `${EDIT_BASE}/${rel}`,
    group: item.group,
    navTitle: item.text,
    prev: prev && { text: prev.text, href: prev.href },
    next: next && { text: next.text, href: next.href },
  }
})

export { DOCS_NAV }
