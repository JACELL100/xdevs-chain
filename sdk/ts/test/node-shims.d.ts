// Minimal ambient types for the Node built-ins used by the tests, so the package does not need
// @types/node (disk budget). Only what the tests use is declared.

declare module 'node:test' {
  type Fn = () => void | Promise<void>
  export function describe(name: string, fn: Fn): void
  export function it(name: string, fn: Fn): void
  export function test(name: string, fn: Fn): void
}

declare module 'node:assert/strict' {
  type ErrorMatcher = RegExp | ((err: unknown) => boolean) | (new (...args: any[]) => unknown) | object
  interface Assert {
    (value: unknown, message?: string): asserts value
    ok(value: unknown, message?: string): asserts value
    equal<T>(actual: unknown, expected: T, message?: string): asserts actual is T
    notEqual(actual: unknown, expected: unknown, message?: string): void
    deepEqual<T>(actual: unknown, expected: T, message?: string): asserts actual is T
    match(value: string, regexp: RegExp, message?: string): void
    throws(fn: () => unknown, error?: ErrorMatcher, message?: string): void
    rejects(fn: Promise<unknown> | (() => Promise<unknown>), error?: ErrorMatcher, message?: string): Promise<void>
  }
  const assert: Assert
  export default assert
}

declare module 'node:fs' {
  export function readFileSync(path: string | URL, encoding: 'utf8'): string
}
