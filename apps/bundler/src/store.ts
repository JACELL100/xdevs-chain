// Tiny crash-safe JSON state file: write to a temp file, fsync, rename over the old one. Debounced saves.
import { closeSync, existsSync, fsyncSync, mkdirSync, openSync, readFileSync, renameSync, writeSync } from 'node:fs'
import { dirname } from 'node:path'

export class JsonStore<T> {
  private timer: NodeJS.Timeout | undefined
  private pending: (() => T) | undefined
  readonly path: string
  private readonly debounceMs: number
  private readonly onError: (e: Error) => void

  constructor(path: string, debounceMs = 250, onError: (e: Error) => void = () => {}) {
    this.path = path
    this.debounceMs = debounceMs
    this.onError = onError
  }

  load(): T | undefined {
    if (!existsSync(this.path)) return undefined
    try {
      return JSON.parse(readFileSync(this.path, 'utf8'), reviveBigints) as T
    } catch (e) {
      this.onError(new Error(`state file ${this.path} unreadable (${(e as Error).message}); starting empty`))
      return undefined
    }
  }

  saveNow(state: T): void {
    mkdirSync(dirname(this.path), { recursive: true })
    const tmp = `${this.path}.tmp`
    const fd = openSync(tmp, 'w', 0o600)
    try {
      writeSync(fd, JSON.stringify(state, (_k, v) => (typeof v === 'bigint' ? `${v.toString()}n` : v)))
      fsyncSync(fd)
    } finally {
      closeSync(fd)
    }
    renameSync(tmp, this.path)
  }

  /** Saves `snapshot()` after `debounceMs` (coalescing bursts of changes). */
  schedule(snapshot: () => T): void {
    this.pending = snapshot
    if (this.timer) return
    this.timer = setTimeout(() => this.flush(), this.debounceMs)
    this.timer.unref()
  }

  flush(): void {
    if (this.timer) clearTimeout(this.timer)
    this.timer = undefined
    const p = this.pending
    this.pending = undefined
    if (!p) return
    try {
      this.saveNow(p())
    } catch (e) {
      this.onError(e as Error)
    }
  }
}

/** JSON reviver for `JsonStore` files: `"123n"` -> 123n. */
export const reviveBigints = (_k: string, v: unknown) => (typeof v === 'string' && /^\d+n$/.test(v) ? BigInt(v.slice(0, -1)) : v)
