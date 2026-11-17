// node --test hooks for the component tests: .tsx files (and @xdevs/ui's TypeScript source with extensionless
// imports) are compiled with the SWC binary that Next.js already ships. Plain .ts files use Node's type stripping.
import { existsSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { fileURLToPath, pathToFileURL } from 'node:url'

const require = createRequire(import.meta.url)
let swc
const transform = async (src, filename) => {
  if (!swc) {
    swc = require('next/dist/build/swc/index.js')
    await swc.loadBindings()
  }
  const out = await swc.transform(src, {
    filename,
    sourceMaps: false,
    module: { type: 'es6' },
    jsc: { target: 'es2022', parser: { syntax: 'typescript', tsx: true }, transform: { react: { runtime: 'automatic' } } },
  })
  return out.code
}

export async function resolve(specifier, context, next) {
  // next/* subpaths are CommonJS files without an exports map (next/link -> next/link.js).
  if (/^next\/[a-z-]+$/.test(specifier)) return next(`${specifier}.js`, context)
  if ((specifier.startsWith('./') || specifier.startsWith('../')) && !/\.[cm]?[jt]sx?$|\.json$/.test(specifier) && context.parentURL?.startsWith('file:')) {
    for (const ext of ['.ts', '.tsx', '/index.ts', '/index.tsx']) {
      const url = new URL(specifier + ext, context.parentURL)
      if (existsSync(fileURLToPath(url))) return { url: url.href, shortCircuit: true }
    }
  }
  return next(specifier, context)
}

export async function load(url, context, next) {
  if (url.startsWith('file:') && (url.endsWith('.tsx') || (url.endsWith('.ts') && url.includes('/packages/ui/')))) {
    const file = fileURLToPath(url)
    return { format: 'module', source: await transform(readFileSync(file, 'utf8'), file), shortCircuit: true }
  }
  return next(url, context)
}
