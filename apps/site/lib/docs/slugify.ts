/**
 * Heading ids, identical to VitePress (`@mdit-vue/shared` slugify), so every
 * deep link into the old docs (`/concepts/guardian-vault#guardian-freeze`,
 * `/roadmap#phase-3-—-proof-of-stake-and-public-devnet-next`, ...) still lands.
 */
const rControl = /[\u0000-\u001f]/g
const rSpecial = /[\s~`!@#$%^&*()\-_+=[\]{}|\\;:"'“”‘’<>,.?/]+/g
const rCombining = /[̀-ͯ]/g

export function slugify(str: string): string {
  return str
    .normalize('NFKD')
    .replace(rCombining, '')
    .replace(rControl, '')
    .replace(rSpecial, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/^(\d)/, '_$1')
    .toLowerCase()
}
