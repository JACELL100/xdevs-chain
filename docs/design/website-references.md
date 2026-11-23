# Website references

Measured on 2026-11-23 with Playwright (Chromium, 1440x900 desktop and 390x844 phone)
from apple.com/iphone, metamask.io, ethereum.org/en and linear.app. Screenshots are kept
outside the repo. These numbers drive the product site in `apps/web`.

## Type scale (computed styles)

| Site | Hero h1 desktop / phone | Section h2 desktop / phone | Lead / body | Tracking |
| --- | --- | --- | --- | --- |
| Apple | 80 / 48 px, wt 600, lh 1.05 | 48-64 / 32-40 px (headline sections) | 28 px lead (19 px phone), 17 px body | -1.2 px on hero, slightly negative everywhere |
| MetaMask | 180 / 74 px condensed, lh 0.82 | 64 / 40 px, lh 0.8-0.94 | 14-16 px | -2% on display |
| ethereum.org | 64 / 48 px, wt 900 | 60 / 48 px, lh 1.2 | 24 px lead (18 px phone), lh 1.6 | normal |
| Linear | 64 / 38 px, wt 510, lh 1.0 | 48 / 24 px, lh 1.0 | 15 px body, 13 px UI labels | -2.2% display |

Take-away: hero 72-88 px desktop / 44-48 px phone, tight leading (1.0-1.05), negative
tracking (-0.02 to -0.035em) and medium weight (500-600), never 900. Section titles
48-64 px / 32-36 px. Lead copy 20-24 px / 18 px; body 16-17 px; labels 12-13 px mono or
caps. Two-tone headlines (bright first clause, dimmed continuation) as on Linear.

## Widths and spacing

- Content max width: Apple 980-1260 px (grid with 90 px side margins at 1440), Linear
  1280 px with 80 px margins, ethereum.org ~1400 px. Text columns stay 560-740 px.
- Section padding: Apple 144 px top/bottom desktop, 112 px phone; Linear 128 px;
  ethereum.org 48-96 px. Use 128-160 px desktop, 88-96 px phone.
- Nav bar height: Apple 44-48 px, Linear 64-73 px, ethereum.org 76 px. Sticky, translucent
  with backdrop blur on dark sites (Linear).

## Imagery and devices

- Apple: product photography floats on a flat field inside large rounded tiles (28 px
  radius), horizontally scrolling "lineup" cards, gallery with pager dots and play button.
- MetaMask: a real phone UI crop at ~600 px tall, bleeding off the hero edge, set on a
  bold diagonal colour block. Product UI is the hero image, not an illustration.
- Linear: the real app UI as a large framed screenshot directly under the hero, with a
  soft light gradient beneath it; logo wall under it in muted grey.
- ethereum.org: two-column feature rows (copy left, illustration right) with floating
  callout chips overlapping the image.

## Navigation and footer

- Short top-level nav (5-7 items), one primary pill button on the right ("Sign up",
  "Get MetaMask"). Mobile collapses to a hamburger/full-screen sheet.
- Footers are dense link grids (4-6 columns, 12-14 px), with legal row and socials.

## Motion

- Apple: sticky scrollytelling (a pinned media frame that changes as text chapters scroll),
  scroll-scrubbed video, fade-and-rise reveals of headline then copy, auto-playing galleries
  with an explicit pause control.
- Linear: subtle fade/blur-in of hero words, screenshots rising into place with a light
  sweep, grid/dot backgrounds that animate slowly; everything is transform/opacity.
- MetaMask: large kinetic type, parallax device over colour block, horizontal marquees.
- ethereum.org: mostly static; carousels with arrows and dots.

Rules adopted: reveal on enter (opacity + 24-40 px rise, 0.6-0.9 s, ease-out), one pinned
scrollytelling section for the safety chapters with a device frame that swaps screens,
parallax only on decorative layers, video opt-in with visible controls, and all motion off
under `prefers-reduced-motion`.
