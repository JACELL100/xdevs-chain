import type { NextConfig } from 'next'

const nextConfig: NextConfig = {
  // Static HTML for Cloudflare Pages: `next build` writes the site to `out/`.
  output: 'export',
  trailingSlash: false,
  images: { unoptimized: true },
  reactStrictMode: true,
  poweredByHeader: false,
}

export default nextConfig
