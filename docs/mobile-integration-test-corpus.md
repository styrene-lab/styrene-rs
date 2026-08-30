# Mobile Integration Test Corpus

## Purpose

The backend-owned mobile corpus connects the Rust mobile runtime contract to
Dioxus component, package, simulator, device, radio, and interoperability
evidence. Its source is
`tests/fixtures/mobile-integration-v1/corpus.json`. Generated results belong
under `target/mobile-integration/` and are never committed.

`styrene-rs` owns runtime behavior and the authoritative fixture contract.
`styrene-ui` owns Dioxus rendering, mobile packaging, platform-service tests, and
copies of versioned fixtures with backend revision provenance.

The independent application workflow corpus lives at
`tests/fixtures/mobile-application-parity-v1/corpus.json`. It records reference
provenance, retained execution evidence, static candidate evidence, and the 11
required P0 parity decisions. Static Sideband or Reticulum MeshChat inspection
cannot become a workflow floor until a provenance-locked application run retains
platform, OS, and non-summary artifacts. Application evidence does not replace
state fixtures, Python protocol interoperability, or packaged Styrene runs.

## Evidence Rules

A passing component fixture proves rendering and interaction behavior only. It
does not prove message delivery, route selection, bearer use, persistence,
security, or interoperability.

Every P0 case requires an authoritative assertion from Rust runtime state,
durable storage, a typed Rust platform service, accessibility semantics, or
pinned upstream interoperability evidence. Screenshots may supplement those
assertions but cannot replace them.

The corpus uses three maturity values:

- `executable`: all seams exist for the declared lane.
- `partial`: a lower layer proves part of the behavior, but package or platform
  evidence remains incomplete.
- `blocked`: the owning API or platform integration does not exist.

## Offline Validation

`just test-mobile-corpus` validates the integration, minimum-state,
application-parity, and backend P0 corpora, then runs the focused backend and
forced-termination evidence tests. It checks schema, repository references,
area and case coverage, evidence scope, launch-profile IDs, bounded deadlines,
cleanup ownership, artifact policy, reference classification, provenance shape,
and closed P0 parity accounting. It does not launch packaged apps or promote
static source and binary inspection into executed application evidence.

## Rust Mobile Runtime

`cargo test -p styrened --test mobile_node` covers embedded boot, direct TCP,
peer discovery, bidirectional LXMF, conversation state, persistence, RNode
bearer policy, and shutdown. Because it opens loopback listeners, it remains in
the explicit network test gate.

## Dioxus Package Tests

The same versioned state corpus must run through the shared Dioxus components on
iOS and Android target classes. Package acceptance requires deterministic launch
profiles, stable accessibility identifiers, bounded logs, artifact identity,
and cleanup evidence.

No maintained Swift or Kotlin host, adapter, test target, or runner is an
acceptance dependency. Platform tooling may generate packaging scaffolding, but
that output is disposable and untracked.

## Simulator Integration

Cross-platform scenarios use Dioxus iOS and Android packages through direct TCP
or an isolated local hub. Each app receives separate identity and storage paths.
The runner records the corpus case, correlation ID, exact UI and backend
revisions, package hashes, platform runtime, milestones, semantic snapshots,
bounded redacted logs, and cleanup outcome.

The isolated hub is controlled with:

```sh
just mobile-hub-start
just mobile-hub-status
just mobile-hub-logs
just mobile-hub-stop
```

The hub binds mesh transport on port 4242 and keeps test state under
`target/mobile-integration/hub/`. It must be stopped after runner-owned tests.

## Device And Radio Evidence

Simulator success does not satisfy physical device, Bluetooth, USB, secure
storage, notification, background, or RNode claims. Physical evidence identifies
the exact Dioxus application revision, backend revision, package hash, device
class, OS, bearer, RNode firmware, radio profile, correlation, packet counts,
deadlines, interruption behavior, reconnect result, and terminal outcome.

Raw device logs, credentials, package contents, generated scaffolding, and local
paths remain outside source control.

## Live Interoperability

Pinned Python RNS and LXMF runs remain separate from internal Rust, fixture,
simulator, and physical-device evidence. A passing internal round trip does not
prove upstream wire compatibility.
