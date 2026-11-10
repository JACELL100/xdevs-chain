// Lets `node --test` import .tsx components: transpiles them with esbuild (a Vite dependency) on load.
import { register } from 'node:module'
register('./tsx-loader.mjs', import.meta.url)
