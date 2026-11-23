/**
 * Roadmap phases, condensed from apps/site/content/roadmap.md (the docs page has the detail).
 * No dates: the project is built by a small team and does not promise them.
 */

export type PhaseStatus = 'done' | 'live' | 'built' | 'next'
export type Phase = {
  n: number
  title: string
  status: PhaseStatus
  badge: string
  summary: string
  points: string[]
}

export const PHASES: Phase[] = [
  {
    n: 0,
    title: 'Foundations',
    status: 'done',
    badge: 'Done',
    summary: 'Repository, Apache-2.0 license, contribution and security policies, architecture decision records.',
    points: ['ADRs 0001 to 0008', 'Security policy and private reporting'],
  },
  {
    n: 1,
    title: 'A single node that MetaMask understands',
    status: 'done',
    badge: 'Done',
    summary: 'Ethereum-exact blocks, transactions and state roots on revm, with an Ethereum JSON-RPC.',
    points: ['Legacy, EIP-1559 and EIP-7702 transactions', 'MetaMask sends tXDS; forge and cast work', 'Restarts keep the same state root'],
  },
  {
    n: 2,
    title: 'A multi-validator BFT network',
    status: 'done',
    badge: 'Essentially done',
    summary: 'Malachite consensus with execute-before-vote, commit certificates, block sync and transaction gossip.',
    points: ['One of four validators down: the chain keeps going', 'Two down: it halts safely and resumes, no forks', 'Chaos scenarios on every pull request'],
  },
  {
    n: 3,
    title: 'Proof of Stake and the public testnet',
    status: 'live',
    badge: 'Live on Enclave',
    summary: 'Enclave went live on 17 Nov 2026. The validator set has been driven by the Staking contract since block 97,200.',
    points: ['Public RPC, explorer, wallet, faucet, bundler, status page', 'Block rewards, downtime and double-sign slashing', 'Proposer rotation since block 225,000'],
  },
  {
    n: 4,
    title: 'The user-safety layer',
    status: 'live',
    badge: 'Live on Enclave',
    summary: 'Passkey accounts, Guardian Vault, freeze and recovery, expiring approvals, .xdevs names, the EOA lock and the safety RPCs.',
    points: ['Reference wallet at wallet.xdevschain.tech', 'Simulation and recipient checks served by the RPC', 'Next: an alert bot for vault events'],
  },
  {
    n: 5,
    title: 'Performance and MEV protection',
    status: 'built',
    badge: 'Built',
    summary: 'Block-STM parallel execution and local fee markets run on Enclave. The encrypted mempool is built and switched off.',
    points: ['Parallel execution on every Enclave validator', 'Per-contract fees since block 97,200', 'Encrypted mempool waits for a distributed key setup and review'],
  },
  {
    n: 6,
    title: 'Public testnet and community',
    status: 'next',
    badge: 'Next',
    summary: 'A long-lived testnet with independent validators, snapshot sync, a bug bounty and real governance on testnet.',
    points: ['Independent community validators', 'Incentivised testnet with points, not tokens', 'Hackathons and a bug bounty'],
  },
  {
    n: 7,
    title: 'Interop and mainnet preparation',
    status: 'next',
    badge: 'Prototype built',
    summary: 'Light-client bridges only, never multisig bridges. Then independent audits and a genesis ceremony for mainnet.',
    points: ['Sepolia light-client bridge: prototype, not deployed', 'Independent security audits', 'Mainnet: coming next, no date'],
  },
]
