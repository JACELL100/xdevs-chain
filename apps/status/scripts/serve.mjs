// Local preview of the static export with Cloudflare Pages semantics: `public/_redirects` rewrites (status 200),
// pretty URLs (/blocks -> blocks.html), and 404.html for unknown paths. `npx serve out` does not apply _redirects,
// so deep links like /tx/0x… would 404 there.
//
//   node scripts/serve.mjs [port] [dir]      (defaults: 4173, out)

import { createServer } from 'node:http'
import { readFileSync, statSync } from 'node:fs'
import { extname, join, normalize, resolve } from 'node:path'

const port = Number(process.argv[2] ?? 4173)
const root = resolve(process.argv[3] ?? 'out')

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.txt': 'text/plain; charset=utf-8',
  '.json': 'application/json',
  '.woff2': 'font/woff2',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
}

/** `_redirects` lines: `<from> <to> [status]`; `*` at the end of `from` is a splat. Only 200 rewrites matter here. */
export function parseRedirects(text) {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('#'))
    .map((l) => {
      const [from, to, status = '302'] = l.split(/\s+/)
      return { from, to, status: Number(status) }
    })
}

export function applyRewrites(pathname, rules) {
  for (const r of rules) {
    if (r.status !== 200) continue
    if (r.from.endsWith('*') ? pathname.startsWith(r.from.slice(0, -1)) : pathname === r.from) return r.to.replace(':splat', pathname.slice(r.from.length - 1))
  }
  return pathname
}

const isFile = (p) => {
  try {
    return statSync(p).isFile()
  } catch {
    return false
  }
}

/** Cloudflare Pages asset lookup: exact file, then `.html`, then `/index.html`. */
export function resolveAsset(dir, pathname) {
  const clean = normalize(decodeURIComponent(pathname)).replace(/^([/\\])+/, '')
  const base = join(dir, clean)
  if (!base.startsWith(dir)) return null
  for (const c of [base, `${base}.html`, join(base, 'index.html')]) if (isFile(c)) return c
  return null
}

let rules = []
try {
  rules = parseRedirects(readFileSync(join(root, '_redirects'), 'utf8'))
} catch {
  /* no _redirects */
}

const server = createServer((req, res) => {
  const url = new URL(req.url ?? '/', 'http://localhost')
  let file = resolveAsset(root, url.pathname)
  if (!file) file = resolveAsset(root, applyRewrites(url.pathname, rules))
  const status = file ? 200 : 404
  if (!file) file = join(root, '404.html')
  try {
    const body = readFileSync(file)
    res.writeHead(status, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-store' })
    res.end(body)
  } catch {
    res.writeHead(404).end('not found')
  }
})

if (import.meta.main) {
  server.listen(port, () => console.log(`serving ${root} with _redirects on http://localhost:${port}`))
}
export { server }
