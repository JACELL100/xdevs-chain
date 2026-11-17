// QR code as inline SVG (dark modules on a light tile, quiet zone included) from the local encoder in lib/qr.ts.
import { useMemo } from 'react'
import { encodeQr, qrPath } from '../lib/qr.ts'

export function QrCode({ value, size = 220, label }: { value: string; size?: number; label: string }) {
  const { d, n } = useMemo(() => {
    const m = encodeQr(value)
    return { d: qrPath(m, 4), n: m.size + 8 }
  }, [value])
  return (
    <svg className="w-qr" viewBox={`0 0 ${n} ${n}`} width={size} height={size} role="img" aria-label={label} shapeRendering="crispEdges">
      <rect width={n} height={n} rx={2.2} fill="#f3f5f8" />
      <path d={d} fill="#05060a" />
    </svg>
  )
}
