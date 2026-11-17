// Human-readable one-line descriptions of calls (vault queue, history). Pure; labels / token metadata are injected.
import { type Address, type Hex, decodeFunctionData, isAddressEqual } from 'viem'
import { accountAbi, erc20Abi, nftAbi, permit2Abi, xdrc20Abi, xnsAbi } from './abi.ts'
import { type Call, UNLIMITED_APPROVAL, XNS_REGISTRY } from './account.ts'
import { fmtAmount, fmtDuration, shortAddr } from './format.ts'

export type Describer = {
  label?: (a: Address) => string | null
  token?: (a: Address) => { symbol: string; decimals: number } | null
  symbol?: string
}

const anyAbi = [...accountAbi, ...erc20Abi, ...xdrc20Abi, ...nftAbi, ...permit2Abi, ...xnsAbi]

export function describeCall(self: Address, c: Call, d: Describer = {}): string {
  const name = (a: Address) => d.label?.(a) ?? shortAddr(a)
  const sym = d.symbol ?? 'tXDS'
  if (c.data === '0x' || c.data.length < 10) return `Send ${fmtAmount(c.value)} ${sym} to ${name(c.target)}`
  try {
    const f = decodeFunctionData({ abi: anyAbi, data: c.data as Hex })
    const args = (f.args ?? []) as readonly unknown[]
    if (isAddressEqual(c.target, self)) {
      switch (f.functionName) {
        case 'setGuardians': {
          const g = args[0] as Address[]
          return `Change guardians to ${g.length ? `${args[1]} of ${g.length}` : 'none'}`
        }
        case 'setVaultPolicy': {
          const p = args[0] as { enabled: boolean; delay: bigint; nativeThreshold: bigint }
          return p.enabled ? `Change vault policy (instant up to ${fmtAmount(p.nativeThreshold)} ${sym}, ${fmtDuration(Number(p.delay))} delay)` : 'Turn the Guardian Vault OFF'
        }
        case 'setWhitelisted':
          return `${args[1] ? 'Trust' : 'Stop trusting'} ${name(args[0] as Address)}`
        case 'disableEoaLock':
          return 'Turn the EOA lock OFF'
        case 'removeSigner':
          return 'Remove a signer'
        case 'setTokenLimit':
          return `Change the limit for token ${name(args[0] as Address)}`
        default:
          return `Account setting: ${f.functionName}`
      }
    }
    const meta = d.token?.(c.target) ?? null
    const amt = (v: bigint) => (meta ? `${fmtAmount(v, meta.decimals)} ${meta.symbol}` : `${v} units of ${name(c.target)}`)
    switch (f.functionName) {
      case 'transfer':
        return `Send ${amt(args[1] as bigint)} to ${name(args[0] as Address)}`
      case 'approve':
        if (args.length === 4) return `Permit2: allow ${name(args[1] as Address)} to spend ${(args[2] as bigint) >= UNLIMITED_APPROVAL ? 'ALL' : String(args[2])} of ${name(args[0] as Address)}`
        return (args[1] as bigint) >= UNLIMITED_APPROVAL ? `Allow ${name(args[0] as Address)} to spend ALL your ${meta?.symbol ?? 'tokens'}` : `Allow ${name(args[0] as Address)} to spend ${amt(args[1] as bigint)}`
      case 'setApprovalForAll':
        return `${args[1] ? 'Let' : 'Revoke'} ${name(args[0] as Address)} ${args[1] ? 'move ALL your NFTs' : 'as NFT operator'}`
      case 'register':
        if (isAddressEqual(c.target, XNS_REGISTRY)) return `Register ${String(args[0])}.xdevs`
        break
      case 'setName':
        return `Set your name to ${String(args[0])}`
      case 'revokeAll':
        return `Revoke every approval on ${name(c.target)}`
    }
    return `${f.functionName} on ${name(c.target)}${c.value ? ` + ${fmtAmount(c.value)} ${sym}` : ''}`
  } catch {
    return `Call ${c.data.slice(0, 10)} on ${name(c.target)}${c.value ? ` + ${fmtAmount(c.value)} ${sym}` : ''}`
  }
}
