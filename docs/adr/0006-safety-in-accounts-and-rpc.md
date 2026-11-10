# ADR-0006: Safety features live in accounts and RPC, never in finality

- Status: Accepted
- Plan reference: §6

## Context
xDEVS Chain offers user-safety features such as simulation, guardian delays and expiring approvals. These must not weaken finality.

## Decision
There are no protocol-level reversals: a finalized transaction is final. Safety features are built into accounts the user opts into (for example, Guardian Vault delays are enforced by the account's own code) and into the RPC layer (simulation and risk flags).

## Alternatives
- Protocol-level reversal or freeze mechanisms: rejected because they undermine finality, invite governance capture and break Ethereum equivalence.

## Consequences
- Finality stays absolute and Ethereum-compatible.
- Users who do not opt in get no account-level protection.

## Revisit
Not planned. Changing this would change the chain's core guarantees.
