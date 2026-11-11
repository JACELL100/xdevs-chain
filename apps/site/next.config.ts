import type { NextConfig } from 'next'

const nextConfig: NextConfig = {
  // Static HTML for Cloudflare Pages: `next build` writes the site to `out/`.
  // With a custom NEXT_DIST_DIR the export lands in that directory instead
  // (used only to run two local builds side by side; CI uses the default).
  output: 'export',
  distDir: process.env.NEXT_DIST_DIR || '.next',
  trailingSlash: false,
  images: { unoptimized: true },
  reactStrictMode: true,
  poweredByHeader: false,
}

export default nextConfig
