# Schema-Driven Client Generation Strategy

Status: Draft, implementation target

This contract defines deterministic client generation inputs for cross-language SDK consumers.

## Goals

1. Generate stable client surfaces from versioned JSON schemas.
2. Keep Go/JavaScript/Python clients aligned with SDK v2.5 contracts.
3. Prevent silent drift between schema contracts and generated client stubs.

## Source of Truth

Client generation manifest:

- `docs/schemas/sdk/v2/clients/client-generation-manifest.json`

Smoke fixtures:

- `docs/schemas/sdk/v2/clients/smoke-requests.json`

Core schema inputs:

- `docs/schemas/sdk/v2/rpc/sdk_negotiate_v2.schema.json`
- `docs/schemas/sdk/v2/rpc/sdk_send_v2.schema.json`
- `docs/schemas/sdk/v2/rpc/sdk_poll_events_v2.schema.json`
- `docs/schemas/sdk/v2/rpc/sdk_snapshot_v2.schema.json`
- `docs/schemas/sdk/v2/error.schema.json`

## Target Client Languages

- Go
- JavaScript/TypeScript
- Python

Each target must include:

1. schema-derived request/response types,
2. machine-code error mapping,
3. transport-agnostic RPC envelope handling.

## Versioning and Backward Compatibility

1. Contract namespace (`v2`) is immutable for generated client major version.
2. Additive schema fields must be optional in generated clients.
3. Breaking schema changes require migration notes and regenerated baselines.

## Generation and Smoke Gate

Run:

```bash
cargo run -p xtask -- schema-client-check
```

The gate validates:

1. manifest completeness,
2. required schema presence,
3. smoke fixture coverage for Go/JS/Python.
