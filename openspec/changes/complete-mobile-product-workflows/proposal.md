# Complete Mobile Product Workflows

## Intent

Close the mobile product gaps exposed by the reviewed Skywave build 9 capture
and the follow-up Styrene implementation audit. Existing backend contracts cover
much of the required behavior, but the Dioxus application still drops
authoritative state, leaves discovery and composition dead-ended, overstates
propagation readiness, and lacks completed platform and packaged-app workflows.

This change turns those findings into one cross-repository implementation wave
with explicit ownership and retained acceptance evidence.

## Scope

- `.local/styrene-rs` owns authoritative runtime, message, retry, route, bearer,
  propagation, identity-custody, backup/restore, and scheduling contracts.
- `.local/styrene-ui` owns Dioxus presentation state, reducers, compose and
  discovery workflows, identity and permission surfaces, clipboard and QR
  adapters, accessibility, and packaged iOS and Android execution.
- Cross-repository work owns immutable revision handoff, fixture synchronization,
  parity-corpus reconciliation, and physical acceptance.
- The existing Messages, People, Network, and More information architecture is
  retained. A useful operational summary may be added within that structure;
  Skywave's navigation is not copied wholesale.
- Calls, map and location sharing, multi-peer group conversations, propagation
  hosting, guaranteed background execution, and iCloud-specific identity sync
  remain excluded from this P0 completion wave. Their presence in a reference
  application does not make them an admitted Styrene requirement.
- Skywave build 9 remains a candidate observation until distribution provenance
  and immutable publication are resolved. Its captures inform gap discovery but
  do not establish protocol compatibility or satisfy Styrene packaged evidence.

## Success criteria

- A discovered peer or manually supplied valid LXMF destination can start one
  durable conversation without fabricated reachability or duplicate identity.
- Mobile runtime, message, retry, route, bearer, receipt, and propagation state
  remains typed and lossless from `styrene-rs` through rendered Dioxus output.
- Direct and Propagated submission availability reflects backend capability and
  current propagation-node readiness before submission.
- Identity, permissions, propagation scheduling, clipboard, QR, and encrypted
  recovery workflows expose truthful outcomes and safe disabled reasons.
- Both packaged targets pass the applicable compose, send, receipt, retry,
  restart, propagation, degraded-state, accessibility, and platform-service
  scenarios with exact backend and UI revisions.
- Application-parity and backend handoff corpora are updated only from evidence
  at their declared boundary; no reference capture promotes a Styrene claim.
