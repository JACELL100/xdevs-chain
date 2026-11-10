import { defineConfig } from 'vite'

// No React plugin: esbuild compiles JSX (automatic runtime). Fast refresh is not needed for this small app and the
// plugin would pull in Babel.
export default defineConfig({
  esbuild: { jsx: 'automatic' },
  // @xdevs/sdk is a symlink to sdk/ts, which has its own node_modules/viem: bundle one copy only.
  resolve: { dedupe: ['viem'] },
  server: { port: 5173, fs: { allow: ['.', '../../sdk/ts'] } },
  build: { target: 'es2022', sourcemap: true, chunkSizeWarningLimit: 400 },
})
