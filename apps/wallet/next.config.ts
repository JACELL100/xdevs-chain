import type { NextConfig } from 'next'

const nextConfig: NextConfig = {
  // Static HTML for Cloudflare Pages: `next build` writes the app to `out/`.
  output: 'export',
  trailingSlash: false,
  images: { unoptimized: true },
  reactStrictMode: true,
  poweredByHeader: false,
  // @xdevs/ui ships TypeScript source; @xdevs/sdk is prebuilt (npm run build:sdk) but listing it is harmless.
  transpilePackages: ['@xdevs/ui', '@xdevs/sdk'],
}

export default nextConfig
