# ADR-0003: Execute-before-vote (state root in the same block)

- Status: Accepted
- Plan reference: §6

## Context
Ethereum tooling expects a header's `stateRoot` to be the state *after* that block.

## Decision
The proposer executes the block, and validators re-execute it before prevoting. The header carries the post-execution state root of the same block.

## Alternatives
- Pipelined or asynchronous execution, where consensus agrees on ordering and execution lags by one block. It gives higher throughput but breaks Ethereum header semantics and complicates tooling.

## Consequences
- Ethereum tooling works unchanged.
- Execution time is on the consensus critical path, which limits block time.

## Revisit
Only when benchmarks show execution is the bottleneck; then consider pipelined execution.
