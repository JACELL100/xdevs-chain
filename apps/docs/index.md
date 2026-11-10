---
layout: home
title: xDEVS Chain
titleTemplate: The chain that has your back

hero:
  name: xDEVS Chain
  text: The chain that has your back
  tagline: An EVM-equivalent Proof-of-Stake Layer 1 that builds user protection into the chain itself, against lost keys, scams, drained wallets and look-alike addresses. Ethereum tools work unchanged.
  image:
    src: /logo.svg
    alt: xDEVS Chain
  actions:
    - theme: brand
      text: Get started
      link: /guide/introduction
    - theme: alt
      text: Run a local chain
      link: /guide/quickstart
    - theme: alt
      text: GitHub
      link: https://github.com/JACELL100/xdevs-chain

features:
  - icon: 🔑
    title: Passkey smart accounts
    details: Sign in with Face ID or a fingerprint. No seed phrase. Guardians you choose can recover the account if you lose your device.
    link: /concepts/smart-accounts
    linkText: How accounts work
  - icon: 🛡️
    title: Guardian Vault
    details: Large transfers, approvals and unknown contract calls wait out a cancel window. A thief cannot switch the protection off quickly either.
    link: /concepts/guardian-vault
    linkText: Read about the vault
  - icon: ⏳
    title: Approvals that expire
    details: XDRC-20 tokens and a Permit2-style contract put an expiry on every allowance, with one-call "revoke all". No more forgotten unlimited approvals.
    link: /concepts/approvals
    linkText: Expiring approvals
  - icon: 🏷️
    title: Look-alike-proof .xdevs names
    details: While paypal.xdevs exists, nobody can register paypa1.xdevs. Non-ASCII homographs are rejected outright.
    link: /concepts/xns
    linkText: XNS names
  - icon: ⚙️
    title: EVM-equivalent
    details: Runs on revm (Osaka rules) and alloy. Solidity, MetaMask, Foundry, Hardhat, viem and ethers work as on Ethereum.
    link: /developers/deploy-contracts
    linkText: Deploy a contract
  - icon: ⚡
    title: BFT finality, no reorgs
    details: Tendermint-style consensus (Malachite). A block is final the moment it is committed. Runs today on local multi-validator networks; public network next.
    link: /operators/local-network
    linkText: Run 4 validators
---

<div class="home-section">

::: warning Status: Phases 1–2 working locally, public testnet not live yet
The node runs a single-node dev chain or a multi-validator BFT network (Malachite consensus, block sync, transaction gossip, WebSocket subscriptions) on your own machines, and works with MetaMask and Foundry. The user-safety contracts, the TypeScript SDK and the faucet are built and tested. Proof-of-Stake wiring and the public devnet (Phase 3) come next. **Testnet tokens (tXDS) have no monetary value** and will not convert to mainnet XDS.
:::

## How it compares

Most chains compete on speed and fees. The problems that actually cost ordinary users money are usually left to wallets and apps. xDEVS Chain puts them into the chain.

| Problem users face | Ethereum | Solana | xDEVS Chain |
|---|---|---|---|
| Lost seed phrase = lost funds | Optional smart wallets (ERC-4337) | Wallet-level only | Built-in smart accounts: passkey login, guardian recovery |
| Stolen key drains the wallet | Nothing at protocol level | Nothing at protocol level | Guardian Vault cancel window, plus an opt-in protocol rule ([XIP-0001](/concepts/eoa-lock)) for existing MetaMask addresses |
| Forgotten unlimited approvals | Unlimited and permanent by default | Delegations persist until revoked | Every allowance expires; one-call "revoke all" |
| Address poisoning, look-alike names | Wallet warnings only | Wallet warnings only | `.xdevs` names reject look-alikes; recipient checks planned in the RPC |
| Time to finality | About 13 minutes | Seconds | Final on commit (BFT, no reorgs); 1–2 s block time target *(public network planned)* |
| Developer compatibility | The standard | Different VM | EVM-equivalent: same tools, same Solidity |

All protection features are **opt-in** and **never reverse a finalized transaction**. There is no admin key that can freeze or seize anyone's funds. [Full comparison and honest trade-offs →](/guide/why)

## What works today and what is next

| Feature | Status |
|---|---|
| EVM execution, MetaMask/Foundry-compatible JSON-RPC, local dev chain | <span class="status ok">Built</span> |
| Multi-validator BFT consensus (Malachite), block sync, tx gossip, `eth_subscribe`, Docker localnet with chaos tests | <span class="status ok">Built (Phase 2)</span> |
| Smart accounts (passkeys, social recovery, session keys), Guardian Vault, guardian freeze | <span class="status ok">Contracts built</span> |
| Expiring approvals, `.xdevs` names with look-alike protection, staking & governance contracts | <span class="status ok">Contracts built</span> |
| EOA lock (XIP-0001) enforced by the node | <span class="status ok">Built</span> |
| Staking-driven validator set, system contracts in genesis, public devnet, faucet & explorer online | <span class="status next">Next (Phase 3)</span> |
| Transaction simulation and recipient-check RPCs, reference wallet | <span class="status planned">Planned (Phase 4)</span> |
| Encrypted mempool, parallel execution, local fee markets | <span class="status planned">Planned (Phase 5)</span> |
| Light-client bridges | <span class="status planned">Planned (Phase 7)</span> |

"Contracts built" means written in Solidity and covered by unit, fuzz and invariant tests in Foundry. They are not yet deployed on any public network and have not been externally audited. See the [roadmap](/roadmap).

## Start building

<div class="home-cta">
  <a class="primary" href="./guide/quickstart">Run the dev chain</a>
  <a href="./developers/json-rpc">JSON-RPC reference</a>
  <a href="./developers/sdk">TypeScript SDK</a>
  <a href="https://github.com/JACELL100/xdevs-chain">Source on GitHub</a>
</div>

</div>
