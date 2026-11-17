import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
// @ts-expect-error plain .mjs helper without types
import { applyRewrites, parseRedirects } from '../scripts/serve.mjs'
import { DYNAMIC_SEGMENTS } from '../src/lib/routes.ts'

const rules = parseRedirects(readFileSync(new URL('../public/_redirects', import.meta.url), 'utf8'))

test('every dynamic route is rewritten (200) to its exported placeholder page', () => {
  for (const seg of DYNAMIC_SEGMENTS) {
    assert.equal(applyRewrites(`/${seg}/0xabc`, rules), `/${seg}/_`, seg)
    assert.ok(rules.some((r: { from: string; status: number }) => r.from === `/${seg}/*` && r.status === 200), `${seg} rule`)
  }
  assert.equal(applyRewrites('/name/alice.xdevs', rules), '/name/_')
  assert.equal(applyRewrites('/validators', rules), '/validators', 'static pages are not rewritten')
  assert.equal(applyRewrites('/_next/static/chunks/a.js', rules), '/_next/static/chunks/a.js')
})

test('placeholder pages exist for each dynamic route', () => {
  for (const seg of DYNAMIC_SEGMENTS) {
    const dir = { block: 'n', tx: 'hash', address: 'addr', token: 'addr', name: 'name' }[seg]
    const page = readFileSync(new URL(`../app/${seg}/[${dir}]/page.tsx`, import.meta.url), 'utf8')
    assert.match(page, /generateStaticParams\(\) \{\s*return \[\{ \w+: '_' \}\]/, seg)
  }
})
