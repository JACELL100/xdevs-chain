import { unified } from 'unified'
import remarkParse from 'remark-parse'
import remarkGfm from 'remark-gfm'
import remarkDirective from 'remark-directive'
import remarkRehype from 'remark-rehype'
import rehypeRaw from 'rehype-raw'
import rehypeShikiFromHighlighter from '@shikijs/rehype/core'
import { createHighlighter, type Highlighter } from 'shiki'
import { visit, SKIP } from 'unist-util-visit'
import { toString as mdToString } from 'mdast-util-to-string'
import { toString as hastToString } from 'hast-util-to-string'
import type { Root as MdRoot, Paragraph, Text } from 'mdast'
import type { Root as HastRoot, Element, ElementContent } from 'hast'
import { slugify } from './slugify'
import { xdevsCodeTheme } from './shiki-theme'

export type TocItem = { id: string; text: string; depth: 2 | 3 }
export type RenderedDoc = { tree: HastRoot; title: string; description: string; toc: TocItem[] }

const CODE_LANGS = ['bash', 'shellscript', 'ts', 'typescript', 'js', 'json', 'toml', 'solidity', 'text']
const CALLOUTS = new Set(['tip', 'info', 'note', 'warning', 'caution', 'danger', 'details'])

let highlighter: Promise<Highlighter> | undefined
function getHighlighter() {
  highlighter ??= createHighlighter({ themes: [xdevsCodeTheme], langs: CODE_LANGS })
  return highlighter
}

/**
 * `:::warning[Title]` ... `:::` containers (the VitePress `::: warning Title`
 * blocks, in generic-directive syntax) become `<aside data-callout>` /
 * `<details>`, rendered by components/docs/Callout. Any other `:name` that the
 * directive syntax picked up in prose is turned back into the literal text.
 */
function remarkCallouts() {
  return (tree: MdRoot) => {
    visit(tree, (node, index, parent) => {
      if (node.type === 'containerDirective' && CALLOUTS.has(node.name)) {
        const first = node.children[0]
        let title = ''
        if (first && first.type === 'paragraph' && first.data && (first.data as { directiveLabel?: boolean }).directiveLabel) {
          title = mdToString(first)
          node.children.shift()
        }
        const type = node.name === 'note' ? 'info' : node.name === 'caution' ? 'warning' : node.name
        node.data = {
          hName: type === 'details' ? 'details' : 'aside',
          hProperties: type === 'details' ? { 'data-title': title || 'Details' } : { 'data-callout': type, 'data-title': title },
        }
        return
      }
      if (
        (node.type === 'textDirective' || node.type === 'leafDirective' || node.type === 'containerDirective') &&
        parent &&
        typeof index === 'number'
      ) {
        const label = 'children' in node && node.children.length ? `[${mdToString(node)}]` : ''
        const prefix = node.type === 'textDirective' ? ':' : node.type === 'leafDirective' ? '::' : ':::'
        const text: Text = { type: 'text', value: `${prefix}${node.name}${label}` }
        if (node.type === 'textDirective') {
          parent.children.splice(index, 1, text as never)
        } else {
          const p: Paragraph = { type: 'paragraph', children: [text] }
          parent.children.splice(index, 1, p as never)
        }
        return [SKIP, index]
      }
    })
  }
}

function el(tagName: string, properties: Element['properties'], children: ElementContent[] = []): Element {
  return { type: 'element', tagName, properties, children }
}

/** Resolve a markdown link relative to the page and drop `.md` / `.html`. */
export function normalizeHref(href: string, pagePath: string): string {
  if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith('//') || href.startsWith('#')) return href
  const [pathPart, hash = ''] = href.split('#', 2)
  let resolved = pathPart
  if (!pathPart.startsWith('/')) {
    const base = pagePath.split('/').slice(0, -1)
    for (const seg of pathPart.split('/')) {
      if (seg === '..') base.pop()
      else if (seg !== '.' && seg !== '') base.push(seg)
    }
    resolved = '/' + base.filter(Boolean).join('/')
  }
  resolved = resolved.replace(/\.(md|html)$/, '').replace(/\/index$/, '') || '/'
  if (resolved.length > 1) resolved = resolved.replace(/\/$/, '')
  return hash ? `${resolved}#${hash}` : resolved
}

function rehypeDocs(opts: { pagePath: string; out: { toc: TocItem[]; title: string; description: string } }) {
  return (tree: HastRoot) => {
    const used = new Map<string, number>()
    visit(tree, 'element', (node, index, parent) => {
      const t = node.tagName
      if (/^h[1-6]$/.test(t)) {
        const text = hastToString(node).trim()
        let id = slugify(text)
        const n = used.get(id) ?? 0
        used.set(id, n + 1)
        if (n > 0) id = `${id}-${n}`
        node.properties.id = id
        if (t === 'h1') {
          if (!opts.out.title) opts.out.title = text
          return
        }
        if (t === 'h2' || t === 'h3') {
          // TOC label: heading text without status badges.
          const label = node.children
            .filter((c) => !(c.type === 'element' && String(c.properties?.className ?? '').includes('status')))
            .map((c) => hastToString(c as never))
            .join('')
            .trim()
          opts.out.toc.push({ id, text: label || text, depth: t === 'h2' ? 2 : 3 })
        }
        node.children.push(
          el('a', { className: ['heading-anchor'], href: `#${id}`, ariaLabel: `Link to this section: ${text}`, dataPagefindIgnore: '' }, [
            { type: 'text', value: '#' },
          ]),
        )
        return
      }
      if (t === 'p' && parent?.type === 'root') {
        // Meta description: the first real sentence-length paragraph (not a status-badge line).
        const text = hastToString(node).replace(/\s+/g, ' ').trim()
        if (!opts.out.description || (opts.out.description.length < 60 && text.length >= 60)) {
          opts.out.description = text
        }
      }
      if (t === 'a' && typeof node.properties.href === 'string') {
        const href = normalizeHref(node.properties.href, opts.pagePath)
        node.properties.href = href
        if (/^https?:\/\//.test(href)) {
          node.properties.target = '_blank'
          node.properties.rel = ['noopener', 'noreferrer']
        }
        return
      }
      if (t === 'table' && parent && typeof index === 'number') {
        // Header rows with no text (`| | |`) are only there to satisfy GFM.
        const thead = node.children.find((c): c is Element => c.type === 'element' && c.tagName === 'thead')
        if (thead && !hastToString(thead).trim()) node.children = node.children.filter((c) => c !== thead)
        // Wrap for horizontal scrolling; the table's own children are still visited.
        parent.children[index] = el('div', { className: ['table-wrap'], tabIndex: 0, role: 'region', ariaLabel: 'Table' }, [node])
        return
      }
      if (t === 'pre' && parent && typeof index === 'number') {
        const code = node.children.find((c): c is Element => c.type === 'element' && c.tagName === 'code')
        const cls = (code?.properties.className as string[] | undefined) ?? []
        const lang = cls.find((c) => c.startsWith('language-'))?.slice(9) ?? 'text'
        parent.children[index] = el('figure', { className: ['code-block'], dataLang: lang }, [node])
        return SKIP
      }
    })
  }
}

function truncate(s: string, max = 158) {
  if (s.length <= max) return s
  const cut = s.slice(0, max)
  return cut.slice(0, cut.lastIndexOf(' ')).replace(/[,;:.\s]+$/, '') + '…'
}

export async function renderMarkdown(source: string, pagePath: string): Promise<RenderedDoc> {
  const out = { toc: [] as TocItem[], title: '', description: '' }
  const hl = await getHighlighter()
  const processor = unified()
    .use(remarkParse)
    .use(remarkGfm)
    .use(remarkDirective)
    .use(remarkCallouts)
    .use(remarkRehype, { allowDangerousHtml: true })
    .use(rehypeRaw)
    .use(rehypeDocs, { pagePath, out })
    .use(rehypeShikiFromHighlighter, hl as never, {
      theme: 'xdevs-dark',
      defaultLanguage: 'text',
      fallbackLanguage: 'text',
      addLanguageClass: true,
    })
  const tree = (await processor.run(processor.parse(source))) as HastRoot
  return { tree, title: out.title, description: truncate(out.description), toc: out.toc }
}
