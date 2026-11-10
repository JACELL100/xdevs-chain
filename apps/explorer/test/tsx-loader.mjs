import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { transform } from 'esbuild'

export async function load(url, context, nextLoad) {
  if (!url.startsWith('file:') || !url.endsWith('.tsx')) return nextLoad(url, context)
  const file = fileURLToPath(url)
  const { code } = await transform(await readFile(file, 'utf8'), { loader: 'tsx', jsx: 'automatic', format: 'esm', sourcefile: file, target: 'es2022' })
  return { format: 'module', source: code, shortCircuit: true }
}
