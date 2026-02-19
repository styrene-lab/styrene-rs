# ADR-0003: Aggressive Platform Split

## Status
Accepted

## Date
2026-02-19

## Context
The previous monolithic crate boundaries made it difficult to enforce architecture rules across protocol logic, transport/runtime orchestration, and operator tooling.

## Decision
- Introduce layered public crates under `crates/libs/*`:
  - `lxmf-core`, `lxmf-router`, `lxmf-runtime`
  - `rns-core`, `rns-transport`, `rns-rpc`
- Move binary entrypoints to `crates/apps/*`:
  - `lxmf-cli`, `reticulumd`, `rns-tools`
- Add boundary checks and CI jobs that enforce layering and API drift control.
- Move Python interop harness ownership out of this repository.

## Consequences
- Immediate hard break in repository structure and crate paths.
- Faster independent evolution of protocol libraries vs operator binaries.
- Stronger CI posture for API governance and dependency policy.
