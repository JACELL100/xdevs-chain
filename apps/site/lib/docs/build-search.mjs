// Postbuild: builds the Pagefind full-text index for the docs into <site>/pagefind/.
// Runs automatically after `npm run build` (npm "postbuild"). The static site is
// in `out/`, or in $NEXT_DIST_DIR when a custom distDir is used (Next writes the
// export there; see next.config.ts).
import fs from 'node:fs'
import path from 'node:path'
import * as pagefind from 'pagefind'

const site = path.resolve(process.env.NEXT_DIST_DIR || 'out')
if (!fs.existsSync(path.join(site, '404.html'))) {
  console.error(`build-search: no exported site in ${site}`)
  process.exit(1)
}

const { index, errors } = await pagefind.createIndex({ forceLanguage: 'en' })
if (!index) throw new Error(`pagefind: ${errors.join('; ')}`)

const added = await index.addDirectory({ path: site })
if (added.errors.length) throw new Error(`pagefind: ${added.errors.join('; ')}`)

const written = await index.writeFiles({ outputPath: path.join(site, 'pagefind') })
if (written.errors.length) throw new Error(`pagefind: ${written.errors.join('; ')}`)

console.log(`build-search: indexed ${added.page_count} pages into ${path.relative(process.cwd(), written.outputPath)}`)
await pagefind.close()
