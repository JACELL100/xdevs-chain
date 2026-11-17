// Postbuild CSP generator (scripts/csp.mjs) on a synthetic static export.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, it } from 'node:test'
// @ts-expect-error plain ESM script without types
import { MAX_HEADER, buildHeaders, connectSources, inlineScriptHashes, pagePolicy, routesFor } from '../scripts/csp.mjs'

const h = (s: string) => `'sha256-${createHash('sha256').update(s, 'utf8').digest('base64')}'`

describe('CSP generator', () => {
  it('hashes inline scripts only (not src= scripts), deduplicated', () => {
    const html = '<script src="/_next/a.js"></script><script>self.__next_f=[]</script><script>self.__next_f=[]</script><script id="x">b()</script>'
    assert.deepEqual(inlineScriptHashes(html), [h('self.__next_f=[]'), h('b()')])
  })
  it('connect-src: Enclave defaults, env overrides, API "off"', () => {
    assert.deepEqual(connectSources({}), ["'self'", 'https://rpc.xdevschain.tech', 'https://bundler.xdevschain.tech', 'https://explorer-api.xdevschain.tech'])
    assert.deepEqual(connectSources({ NEXT_PUBLIC_RPC_URL: 'http://127.0.0.1:8545/x', NEXT_PUBLIC_BUNDLER_URL: 'http://127.0.0.1:4337', NEXT_PUBLIC_API_URL: 'off' }), [
      "'self'",
      'http://127.0.0.1:8545',
      'http://127.0.0.1:4337',
    ])
  })
  it('routes and the long-policy fallback', () => {
    assert.deepEqual(routesFor('index.html'), ['/', '/index.html'])
    assert.deepEqual(routesFor('send.html'), ['/send', '/send.html'])
    const many = Array.from({ length: 60 }, (_, i) => `<script>x${i}()</script>`).join('')
    const p = pagePolicy(many, ["'self'"])
    assert.equal(p.strict, false)
    assert.match(p.policy, /'unsafe-inline'/)
    assert.ok(pagePolicy('<script>a()</script>', ["'self'"]).policy.length <= MAX_HEADER)
  })
  it('writes a catch-all rule plus one detached rule per page', () => {
    const dir = mkdtempSync(join(tmpdir(), 'csp-'))
    try {
      writeFileSync(join(dir, 'index.html'), '<script>home()</script>')
      writeFileSync(join(dir, '404.html'), '<script>nf()</script>')
      mkdirSync(join(dir, 'nested'))
      writeFileSync(join(dir, 'nested', 'index.html'), '<p>no scripts</p>')
      const { text, pages, loose } = buildHeaders(dir, {})
      assert.equal(pages, 3)
      assert.equal(loose, 0)
      const NL = '\n'
      const catchAll = text.split(NL + NL)[0]
      assert.ok(catchAll.includes('/*' + NL) && catchAll.includes(`script-src 'self' ${h('nf()')};`), 'default policy carries the 404 hashes')
      const rule = (route: string, scripts: string) => [route, '  ! Content-Security-Policy', `  Content-Security-Policy: default-src 'self'; script-src 'self'${scripts};`].join(NL)
      assert.ok(text.includes(NL + rule('/', ` ${h('home()')}`)))
      assert.ok(text.includes(NL + rule('/nested', '')))
      assert.ok(!text.includes("script-src 'self' 'unsafe-inline'"), 'no page needed the fallback')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
