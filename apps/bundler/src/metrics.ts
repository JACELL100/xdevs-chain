// Minimal Prometheus text-format registry (counters and gauges with optional labels), no dependencies.
type Labels = Record<string, string>
type Series = { labels: Labels; value: number | bigint }
type Metric = { name: string; help: string; type: 'counter' | 'gauge'; series: Map<string, Series> }

const key = (labels: Labels) =>
  Object.keys(labels)
    .sort()
    .map((k) => `${k}=${labels[k]}`)
    .join(',')
const esc = (v: string) => v.replace(/\\/g, '\\\\').replace(/\n/g, '\\n').replace(/"/g, '\\"')

export class Metrics {
  private readonly metrics = new Map<string, Metric>()

  private metric(name: string, help: string, type: Metric['type']): Metric {
    let m = this.metrics.get(name)
    if (!m) {
      m = { name, help, type, series: new Map() }
      this.metrics.set(name, m)
    }
    return m
  }

  /** Declares a metric so it is exported (as 0) before its first update. */
  declare(name: string, help: string, type: Metric['type'], labels: Labels = {}): void {
    const m = this.metric(name, help, type)
    if (!m.series.has(key(labels))) m.series.set(key(labels), { labels, value: 0 })
  }

  inc(name: string, help: string, labels: Labels = {}, by: number | bigint = 1): void {
    const m = this.metric(name, help, 'counter')
    const k = key(labels)
    const s = m.series.get(k)
    if (!s) m.series.set(k, { labels, value: by })
    else s.value = typeof s.value === 'bigint' || typeof by === 'bigint' ? BigInt(s.value) + BigInt(by) : s.value + by
  }

  set(name: string, help: string, value: number | bigint, labels: Labels = {}): void {
    this.metric(name, help, 'gauge').series.set(key(labels), { labels, value })
  }

  get(name: string, labels: Labels = {}): number | bigint | undefined {
    return this.metrics.get(name)?.series.get(key(labels))?.value
  }

  render(): string {
    const out: string[] = []
    for (const m of this.metrics.values()) {
      out.push(`# HELP ${m.name} ${m.help}`, `# TYPE ${m.name} ${m.type}`)
      for (const s of m.series.values()) {
        const l = Object.entries(s.labels)
        const lbl = l.length ? `{${l.map(([k, v]) => `${k}="${esc(v)}"`).join(',')}}` : ''
        out.push(`${m.name}${lbl} ${s.value.toString()}`)
      }
    }
    return `${out.join('\n')}\n`
  }
}
