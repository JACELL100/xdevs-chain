# Bridges (light clients)

:::warning[Prototype]
The xDEVS ↔ Ethereum Sepolia bridge is a **prototype**. It is tested but not audited and **not deployed** on any network. Do not use it with anything of value. The design and its open risks are in the [research document](https://github.com/JACELL100/xdevs-chain/blob/main/docs/research/light-client-bridges.md).
:::

## No multisig bridges

Most bridge hacks share one cause: a small group of signers (a "multisig" or "committee") is trusted to say what happened on the other chain. Steal or bribe enough of those keys and the attacker can mint anything.

xDEVS Chain bridges have **no signer set of their own**. Each side runs a **light client** of the other chain in a smart contract. The light client checks the other chain's own consensus proofs, the same proofs that chain's nodes check:

| Direction | What the contract verifies | Where |
|---|---|---|
| Ethereum → xDEVS | Ethereum's **sync committee** (512 validators) signed the header with a BLS aggregate signature, and that header finalized the block | `EthereumLightClient` on xDEVS Chain |
| xDEVS → Ethereum | More than 2/3 of the xDEVS **validator voting power** signed the block (a commit certificate, Ed25519) | `XdevsLightClient` on Sepolia |

Once a block is verified, the contract trusts its `receiptsRoot`. A bridge transfer is then a **receipt proof**: a Merkle-Patricia proof that the bridge contract on the other chain emitted the lock or burn event in that block. Bridging needs the other chain's consensus to be honest, and nothing else.

## How a transfer works

1. **Lock** an ERC-20 in the `SepoliaTokenVault` on Sepolia. The vault emits `Locked(nonce, token, sender, recipient, amount)`.
2. Wait until Ethereum finalizes the block (about 13 minutes).
3. A **relayer** submits the finality update, the block header and the receipt proof to xDEVS Chain. Anyone can run the relayer (`tools/bridge-relayer`). It cannot forge anything, because the contracts check every proof.
4. `XdevsBridge` mints the **canonical** bridged token to the recipient. Canonical tokens come from the `CanonicalTokenFactory`, so there is exactly one "TST from Sepolia" and wallets can verify it.
5. To go back, **burn** the bridged token with `XdevsBridge.burn`. xDEVS finality is instant. The relayer submits the block's commit certificate, header and receipt proof to Sepolia, and the vault releases the original tokens.

Each message has a nonce that can be used once, so a proof cannot be replayed.

## Validator-set changes

The xDEVS validator set can change at each epoch boundary once staking-driven sets are active. The Sepolia light client does not trust anyone to announce the new set. It verifies a storage proof of the Staking contract's `validatorSetHash` in the certified boundary block, then derives the new voting powers the same way the nodes do.

The Ethereum sync committee rotates every 8,192 slots (about 27 hours). The new committee is proven from a finalized update of the current period.

## Costs (measured on real data)

| Step | Gas |
|---|---|
| Ethereum finality update on xDEVS (BLS, 3–19 non-signers) | ~500k–660k |
| Load a new sync committee on xDEVS (every ~27 h) | ~3.8M |
| xDEVS commit certificate on Sepolia (Ed25519 in Solidity) | ~450k per signature; 1.37M for the testnet's 3-of-4 quorum |
| Store a header (either side) | ~180k (cold) |
| Mint / release with a receipt proof | ~150k–230k |

Ethereum has no Ed25519 precompile, so certificate checks on Sepolia are the expensive part. A BLS-aggregated or zero-knowledge certificate would make them constant-cost. On xDEVS Chain, the `ED25519VERIFY` precompile (`0x0A00…0001`, about 3,000 gas) exists for contracts that verify Ed25519, behind a genesis activation block. It is off by default.

## Safety switches

- **Freeze on equivocation.** If two valid proofs ever disagree about the same block height, the remote chain's finality has failed. The light client then freezes for good, and every bridge action that depends on it stops.
- **Pause only.** A guardian (the timelocked Governance contract) can pause and unpause the endpoints. It cannot mint, release, change the light client or upgrade the code.
- **No upgrade path.** Fixing a bug means deploying new contracts, with a governance timelock, and migrating.
