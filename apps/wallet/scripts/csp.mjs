// Postbuild: writes out/_headers (Cloudflare Pages) with security headers and a strict, per-page Content Security
// Policy. Next.js inlines bootstrap scripts, so instead of 'unsafe-inline' every inline <script> of each exported
// page is allowed by its SHA-256 hash. connect-src lists exactly the configured endpoints (NEXT_PUBLIC_*).
import { createHash } from 'node:crypto'
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { join, relative, sep } from 'node:path'

const ENCLAVE = {
  rpc: 'https://rpc.xdevschain.tech',
  bundler: 'https://bundler.xdevschain.tech',
  api: 'https://explorer-api.xdevschain.tech',
}

export function inlineScriptHashes(html) {
  const out = new Set()
  const re = /<script(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/gi
  for (let m = re.exec(html); m; m = re.exec(html)) {
    if (!m[1]) continue
    out.add(`'sha256-${createHash('sha256').update(m[1], 'utf8').digest('base64')}'`)
  }
  return [...out]
}

function origin(u) {
  try {
    return new URL(u).origin
  } catch {
    return null
  }
}

export function connectSources(env = process.env) {
  const api = env.NEXT_PUBLIC_API_URL === 'off' || env.NEXT_PUBLIC_API_URL === 'none' ? null : env.NEXT_PUBLIC_API_URL || ENCLAVE.api
  const list = [env.NEXT_PUBLIC_RPC_URL || ENCLAVE.rpc, env.NEXT_PUBLIC_BUNDLER_URL || ENCLAVE.bundler, api].map((u) => (u ? origin(u) : null)).filter(Boolean)
  return [...new Set(["'self'", ...list])]
}

export function csp(hashes, connect) {
  return [
    "default-src 'self'",
    `script-src 'self' ${hashes.join(' ')}`.trim(),
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self'",
    `connect-src ${connect.join(' ')}`,
    "frame-ancestors 'none'",
    "base-uri 'none'",
    "form-action 'none'",
    "object-src 'none'",
    'upgrade-insecure-requests',
  ].join('; ')
}

function htmlFiles(dir) {
  const out = []
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) out.push(...htmlFiles(p))
    else if (name.endsWith('.html')) out.push(p)
  }
  return out
}

/** Route paths served by a static export file: index.html -> "/", send.html -> "/send" (+ "/send.html"). */
export function routesFor(rel) {
  const p = rel.split(sep).join('/')
  if (p === 'index.html') return ['/', '/index.html']
  if (p.endsWith('/index.html')) return [`/${p.slice(0, -'/index.html'.length)}`, `/${p}`]
  return [`/${p.slice(0, -'.html'.length)}`, `/${p}`]
}

export function buildHeaders(outDir, env = process.env) {
  const connect = connectSources(env)
  const common = [
    'X-Content-Type-Options: nosniff',
    'Referrer-Policy: no-referrer',
    'X-Frame-Options: DENY',
    'Cross-Origin-Opener-Policy: same-origin',
    'Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=(), usb=(), publickey-credentials-get=(self), publickey-credentials-create=(self)',
    'Strict-Transport-Security: max-age=31536000',
  ]
  let text = `/*\n${common.map((h) => `  ${h}`).join('\n')}\n  Content-Security-Policy: ${csp([], connect)}\n`
  for (const file of htmlFiles(outDir)) {
    const hashes = inlineScriptHashes(readFileSync(file, 'utf8'))
    for (const route of routesFor(relative(outDir, file))) {
      text += `\n${route}\n  ! Content-Security-Policy\n  Content-Security-Policy: ${csp(hashes, connect)}\n`
    }
  }
  return text
}

if (import.meta.url === `file://${process.argv[1].replace(/\/g, '/')}` || process.argv[1]?.endsWith('csp.mjs')) {
  const outDir = process.argv[2] ?? 'out'
  const text = buildHeaders(outDir)
  writeFileSync(join(outDir, '_headers'), text)
  console.log(`wrote ${join(outDir, '_headers')} (${text.length} bytes)`)
}
