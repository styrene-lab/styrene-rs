# Extract Styrene UI Repository Design

## Repository Roles

`styrene-rs` owns protocol, runtime, daemon, IPC, transports, shared client and
session crates, backend fixtures, and the Ratatui client.

`styrene-ui` owns Dioxus presentation, renderer-neutral presentation stores,
desktop and mobile packaging, assets, native platform adapters, and UI tests.

The planned canonical repository is `styrene-lab/styrene-ui`. Repository creation
must stop if that target or its governance cannot be confirmed.

## Extraction Method

Extraction runs from a temporary clone or another isolated source. It must not
rewrite the consolidation checkout or the reserved long-running Styrene clone.
The extraction retains history for `crates/apps/styrene-dx` and records the
source `styrene-rs` revision in migration documentation.

After the new repository passes its acceptance gates, `styrene-rs` removes the
maintained Dioxus copy in a separate reviewable commit. A short pointer may
remain. There is no period where two repositories accept independent product
changes to the same Dioxus source.

## Initial Workspace

The extracted repository begins with these conceptual boundaries:

```text
crates/
  styrene-ui-app/       shared Dioxus routes and components
  styrene-ui-state/     presentation stores, reducers, and selectors
  styrene-ui-platform/  platform-service traits
apps/
  desktop/              desktop launcher and packaging
native/
  ios/                  reserved for the later mobile change
  android/              reserved for the later mobile change
```

The initial extraction can preserve the current internal layout before a
reviewable refactor reaches this structure. History preservation and a working
desktop build take priority over an atomic directory redesign.

## Dependency Policy

The first independent build pins `styrene-rs` Git dependencies to one full
commit identifier. Released crate versions may replace Git dependencies after
the shared contracts have a compatible release policy. Local path overrides are
allowed only in untracked developer configuration.

`styrene-ui` consumes the shared client and session crates. It must not import
server wire modules or use private daemon internals. Protocol Lab uses the
published runner boundary or a separately installed runner executable. Dioxus
tasks never supervise Python or protocol test processes directly.

## Compatibility

Each GUI revision declares its minimum and tested `styrene-rs` contract version
or revision. CI tests supported Live negotiation failures and Embedded startup.
Breaking backend changes land with a migration path or a coordinated GUI pin.

## Rollout And Recovery

The in-tree Dioxus application remains authoritative during extraction. The new
repository must pass formatting, lint, unit, component, runtime-profile, and
applicable Lab tests. If validation fails, the source remains in `styrene-rs`.
No partial authority switch occurs.
