/**
 * Props for a scroll-reveal element (see MotionRuntime), with an optional stagger delay.
 * The delay travels as a data attribute so it never clobbers the element's own `style`.
 */
export function rv(delay = 0, kind: '' | 'fade' | 'scale' = ''): { 'data-rv': string; 'data-d'?: number } {
  return delay ? { 'data-rv': kind, 'data-d': delay } : { 'data-rv': kind }
}
