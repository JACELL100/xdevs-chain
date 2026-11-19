// Structured JSON-lines logger (one object per line on stdout/stderr, journald-friendly). Never pass secrets.
export type LogLevel = 'debug' | 'info' | 'warn' | 'error' | 'silent'
export type LogFields = Record<string, unknown>
export type Logger = {
  debug(msg: string, fields?: LogFields): void
  info(msg: string, fields?: LogFields): void
  warn(msg: string, fields?: LogFields): void
  error(msg: string, fields?: LogFields): void
}

const ORDER: Record<LogLevel, number> = { debug: 10, info: 20, warn: 30, error: 40, silent: 100 }

const replacer = (_k: string, v: unknown) => (typeof v === 'bigint' ? v.toString() : v)

export function createLogger(
  level: LogLevel = 'info',
  sink: (line: string, level: LogLevel) => void = (line, l) => (l === 'error' || l === 'warn' ? process.stderr : process.stdout).write(`${line}\n`),
): Logger {
  const min = ORDER[level]
  const emit = (l: LogLevel, msg: string, fields?: LogFields) => {
    if (ORDER[l] < min) return
    sink(JSON.stringify({ ts: new Date().toISOString(), level: l, msg, ...fields }, replacer), l)
  }
  return {
    debug: (m, f) => emit('debug', m, f),
    info: (m, f) => emit('info', m, f),
    warn: (m, f) => emit('warn', m, f),
    error: (m, f) => emit('error', m, f),
  }
}

export const silentLogger: Logger = { debug() {}, info() {}, warn() {}, error() {} }

/** Adapts the legacy `log: (msg) => void` option. */
export function loggerFromFn(fn: (msg: string) => void): Logger {
  const f = (m: string, x?: LogFields) => fn(x ? `${m} ${JSON.stringify(x, replacer)}` : m)
  return { debug() {}, info: f, warn: f, error: f }
}
