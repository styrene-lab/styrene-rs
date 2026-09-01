# Extract Styrene UI Repository Design

## Reassessment

The repository extraction and authority switch are complete. Provenance, history,
workspace separation, immutable backend pins, generated-output exclusions, Fixture
isolation, and Lab process bounds are present in `styrene-ui`.

The remaining tasks are not extraction implementation. They cover repository
governance, the unresolved desktop client/session boundary, the historical
rollback condition, and retained final desktop validation runs. The original
requirement that native mobile directories
remain empty was superseded when the independently versioned Rust mobile hosts
landed; only the generated-output exclusion remains authoritative.

## Repository Roles

`styrene-rs` owns protocol, runtime, daemon, IPC, transports, shared client and
session contracts, backend fixtures, and the Ratatui client.

`styrene-ui` owns Dioxus presentation, renderer-neutral presentation stores,
desktop and mobile packaging, assets, native platform adapters, and UI tests.

The canonical Dioxus repository is `styrene-lab/styrene-ui`. `styrene-rs` no
longer accepts Dioxus application changes. Remaining work that changes a backend
contract and its Dioxus consumer is coordinated across both repositories.

## Extraction Method

Extraction runs from a temporary clone or another isolated source. It must not
rewrite the consolidation checkout or the reserved long-running Styrene clone.
The extraction retains history for `crates/apps/styrene-dx` and records the
source `styrene-rs` revision in migration documentation.

The authority switch removes the maintained Dioxus copy from `styrene-rs` in a
separate reviewable commit and leaves repository and compatibility pointers.
There is no period where two repositories accept independent product changes to
the same Dioxus source.

## Initial Workspace

The extracted repository begins with these conceptual boundaries:

```text
crates/
  styrene-ui-app/       shared Dioxus routes and components
  styrene-ui-state/     presentation stores, reducers, and selectors
  styrene-ui-platform/  platform-service traits
apps/
  desktop/              desktop launcher and packaging
  mobile/               Rust Dioxus mobile launcher and packaging
```

The initial extraction can preserve the current internal layout before a
reviewable refactor reaches this structure. History preservation and a working
desktop build take priority over an atomic directory redesign.

## Dependency Policy

The first independent build pins `styrene-rs` Git dependencies to one full
commit identifier. Released crate versions may replace Git dependencies after
the shared contracts have a compatible release policy. Local path overrides are
allowed only in untracked developer configuration.

`styrene-ui` consumes public shared client, session, and IPC contracts from
`styrene-rs`. It must not import
server wire modules or use private daemon internals. Protocol Lab uses the
published runner boundary or a separately installed runner executable. Dioxus
tasks never supervise Python or protocol test processes directly.

## Compatibility

Each GUI revision declares its minimum and tested `styrene-rs` contract version
or revision. CI tests supported Live negotiation failures and Embedded startup.
Breaking backend changes land with a migration path or a coordinated GUI pin.

## Rollout And Recovery

The Dioxus authority switch is complete: `styrene-ui` is authoritative and the
maintained `styrene-dx` source is removed from `styrene-rs`. Validation of the
post-removal `styrene-rs` TUI, workspace, documentation, and release boundary is
still required before task 5.4 can close. A failed cross-repository change is
repaired or reverted in its owning repository; it does not restore a second
editable Dioxus copy to `styrene-rs`.
