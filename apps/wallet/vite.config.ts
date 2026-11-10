import { defineConfig } from 'vite'

// No React plugin (keeps dependencies minimal): esbuild compiles TSX with the automatic JSX runtime.
// @xdevs/sdk is a `file:` link to sdk/ts, which has its own node_modules/viem: `dedupe` makes the bundle use one copy.
export default defineConfig({
  esbuild: { jsx: 'automatic' },
  resolve: { preserveSymlinks: true, dedupe: ['viem'] },
  build: {
    target: 'es2022',
    sourcemap: false,
    chunkSizeWarningLimit: 700,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (/node_modules[\/](react|react-dom|scheduler)[\/]/.test(id)) return 'react'
          if (/node_modules[\/](viem|@noble|@scure|abitype|ox)[\/]/.test(id)) return 'viem'
        },
      },
    },
  },
  server: { port: 5173 },
})
