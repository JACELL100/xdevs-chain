'use client'

/**
 * Ambient background: a quiet field of validator nodes. Once a second a
 * proposer lights up and its vote gossips two hops out along hairline links.
 * Low contrast on purpose. Paused off-screen and when the tab is hidden;
 * a single still frame under prefers-reduced-motion.
 */
import { useEffect, useRef } from 'react'
import { reducedMotion, useInView } from './motion'

type Node = { x: number; y: number; ox: number; oy: number; ph: number; heat: number; links: number[] }
type Pulse = { a: number; b: number; t0: number; dur: number; hop: number }

function rng(seed: number) {
  return () => {
    seed = (seed * 1664525 + 1013904223) % 4294967296
    return seed / 4294967296
  }
}

function createField(canvas: HTMLCanvasElement) {
  let ctx: CanvasRenderingContext2D | null = null
  let nodes: Node[] = []
  let pulses: Pulse[] = []
  let w = 0
  let h = 0
  let lastBlock = 0

  function layout() {
    const r = canvas.getBoundingClientRect()
    w = r.width
    h = r.height
    const dpr = Math.min(window.devicePixelRatio || 1, 2)
    canvas.width = Math.round(w * dpr)
    canvas.height = Math.round(h * dpr)
    ctx = canvas.getContext('2d')
    ctx?.setTransform(dpr, 0, 0, dpr, 0, 0)

    const rand = rng(7117)
    const count = w < 640 ? 22 : w < 1100 ? 34 : 46
    const cols = Math.max(1, Math.round(Math.sqrt((count * w) / Math.max(1, h))))
    const rows = Math.ceil(count / cols)
    nodes = []
    for (let i = 0; i < count; i++) {
      const c = i % cols
      const ri = Math.floor(i / cols)
      const x = ((c + 0.5 + (rand() - 0.5) * 0.8) / cols) * w
      const y = ((ri + 0.5 + (rand() - 0.5) * 0.8) / rows) * h
      nodes.push({ x, y, ox: x, oy: y, ph: rand() * Math.PI * 2, heat: 0, links: [] })
    }
    nodes.forEach((n, i) => {
      const near = nodes
        .map((m, j) => ({ j, d: (m.ox - n.ox) ** 2 + (m.oy - n.oy) ** 2 }))
        .filter((o) => o.j !== i)
        .sort((a, b) => a.d - b.d)
        .slice(0, 3)
      for (const { j } of near) {
        if (!n.links.includes(j)) n.links.push(j)
        if (!nodes[j].links.includes(i)) nodes[j].links.push(i)
      }
    })
    pulses = []
  }

  function propose(t: number) {
    const p = Math.floor(Math.random() * nodes.length)
    nodes[p].heat = 1
    for (const j of nodes[p].links) pulses.push({ a: p, b: j, t0: t, dur: 520 + Math.random() * 260, hop: 0 })
  }

  function frame(t: number) {
    if (!ctx || !nodes.length) return
    if (t - lastBlock > 1000) {
      lastBlock = t
      propose(t)
    }
    ctx.clearRect(0, 0, w, h)
    for (const n of nodes) {
      n.x = n.ox + Math.sin(t / 5200 + n.ph) * 6
      n.y = n.oy + Math.cos(t / 6100 + n.ph) * 5
      n.heat *= 0.965
    }
    ctx.lineWidth = 1
    ctx.strokeStyle = 'rgba(200, 210, 225, 0.05)'
    ctx.beginPath()
    nodes.forEach((n, i) => {
      for (const j of n.links) {
        if (j < i) continue
        ctx!.moveTo(n.x, n.y)
        ctx!.lineTo(nodes[j].x, nodes[j].y)
      }
    })
    ctx.stroke()

    const next: Pulse[] = []
    for (const p of pulses) {
      const k = (t - p.t0) / p.dur
      const a = nodes[p.a]
      const b = nodes[p.b]
      if (k >= 1) {
        b.heat = Math.max(b.heat, p.hop === 0 ? 0.7 : 0.4)
        if (p.hop === 0)
          for (const j of b.links) if (j !== p.a && Math.random() < 0.55) next.push({ a: p.b, b: j, t0: t, dur: 620, hop: 1 })
        continue
      }
      const e = k < 0.5 ? 2 * k * k : 1 - (-2 * k + 2) ** 2 / 2
      const x = a.x + (b.x - a.x) * e
      const y = a.y + (b.y - a.y) * e
      const alpha = (p.hop === 0 ? 0.7 : 0.38) * Math.sin(Math.PI * k)
      const grad = ctx.createLinearGradient(a.x, a.y, x, y)
      grad.addColorStop(0, 'rgba(79, 227, 207, 0)')
      grad.addColorStop(1, `rgba(79, 227, 207, ${alpha * 0.45})`)
      ctx.strokeStyle = grad
      ctx.beginPath()
      ctx.moveTo(a.x, a.y)
      ctx.lineTo(x, y)
      ctx.stroke()
      ctx.fillStyle = `rgba(150, 245, 230, ${alpha})`
      ctx.beginPath()
      ctx.arc(x, y, 1.4, 0, Math.PI * 2)
      ctx.fill()
      next.push(p)
    }
    pulses = next

    for (const n of nodes) {
      ctx.fillStyle = 'rgba(210, 218, 230, 0.26)'
      ctx.beginPath()
      ctx.arc(n.x, n.y, 1.3, 0, Math.PI * 2)
      ctx.fill()
      if (n.heat > 0.02) {
        ctx.fillStyle = `rgba(79, 227, 207, ${n.heat * 0.16})`
        ctx.beginPath()
        ctx.arc(n.x, n.y, 4 + n.heat * 6, 0, Math.PI * 2)
        ctx.fill()
        ctx.fillStyle = `rgba(160, 245, 232, ${n.heat * 0.9})`
        ctx.beginPath()
        ctx.arc(n.x, n.y, 1.6, 0, Math.PI * 2)
        ctx.fill()
      }
    }
  }

  return { layout, frame }
}

export default function HeroField() {
  const ref = useRef<HTMLCanvasElement>(null)
  const inView = useInView(ref)
  const field = useRef<ReturnType<typeof createField> | null>(null)

  useEffect(() => {
    const c = ref.current!
    const f = createField(c)
    field.current = f
    f.layout()
    f.frame(performance.now())
    const ro = new ResizeObserver(() => {
      f.layout()
      f.frame(performance.now())
    })
    ro.observe(c)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    if (!inView || reducedMotion()) return
    let raf = 0
    let running = false
    const loop = (t: number) => {
      field.current?.frame(t)
      raf = requestAnimationFrame(loop)
    }
    const start = () => {
      if (running || document.hidden) return
      running = true
      raf = requestAnimationFrame(loop)
    }
    const stop = () => {
      running = false
      cancelAnimationFrame(raf)
    }
    const onVis = () => (document.hidden ? stop() : start())
    start()
    document.addEventListener('visibilitychange', onVis)
    return () => {
      stop()
      document.removeEventListener('visibilitychange', onVis)
    }
  }, [inView])

  return <canvas ref={ref} className="hero-field" aria-hidden="true" />
}
