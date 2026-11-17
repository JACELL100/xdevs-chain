import type { NextConfig } from 'next'

const nextConfig: NextConfig = {
  // Static HTML for Cloudflare Pages: `next build` writes the explorer to `out/`. Dynamic routes are exported once
  // with the placeholder param "_" and served for every value by the rewrites in public/_redirects.
  output: 'export',
  trailingSlash: false,
  images: { unoptimized: true },
  reactStrictMode: true,
  poweredByHeader: false,
  // @xdevs/ui ships TypeScript source; @xdevs/sdk is prebuilt (npm run build:sdk).
  transpilePackages: ['@xdevs/ui', '@xdevs/sdk'],
}

export default nextConfig
