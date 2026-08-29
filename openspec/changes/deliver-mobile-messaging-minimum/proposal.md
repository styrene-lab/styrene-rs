# Deliver Mobile Messaging Minimum

## Intent

Deliver a useful Rust-owned Dioxus Reticulum and LXMF mobile product with the
smallest defensible interoperability claim and an evidence-backed product floor.
The mobile product must preserve one identity, connect to a configured public
TCP interface, discover canonical peers, exchange durable text messages, and use
a selected standard LXMF propagation node for offline delivery.

A versioned corpus of RNS-compatible messaging applications provides observed
workflow floors. Skywave provides the currently recorded connectivity and
discovery evidence. Pinned Python RNS and LXMF remain the protocol authorities,
and the Python LXMF round trip against Brutus provides the propagation behavior
target. Interaction-only references cannot establish protocol compatibility.

## Scope

This change adds persistent network and propagation-node configuration. It adds
truthful connection and discovery state, durable text conversations, explicit
delivery evidence, and standard propagation controls. A versioned
application-parity corpus complements the shared cross-platform state corpus.
One Dioxus application and its Rust-owned embedded runtime deliver the behavior
on iOS and Android.

No Swift or Kotlin mobile host is retained. This change excludes mobile
propagation hosting, automatic propagation-node selection, attachments, Paper
delivery, NomadNet parity, advanced route or link administration, fleet, tunnel,
and Lab workflows.

A platform must not claim RNode support until its Dioxus release candidate passes
the applicable physical-device gates.

## Success criteria

- A cold launch restores one identity and one configured TCP client, and reports
  connection and reconnect state without requiring an RNode.
- Canonical delivery announces produce a deduplicated, generation-safe people
  directory on iOS and Android.
- A user can compose, persist, send, receive, retry, and inspect text messages
  without conflating queue acceptance, propagation upload, and delivery.
- A selected standard LXMF propagation node persists across launch, supports
  manual and bounded automatic synchronization, and presents each retrieved
  message once.
- The same deterministic state and behavior corpus passes against the Dioxus iOS
  and Android targets, followed by public-Brutus and applicable physical-device gates.
- Every required mobile journey has a provenance-backed application-parity row.
  The row is matched or records an accepted intentional difference. Deferred,
  unsupported, and unevidenced rows remain explicit and block the affected claim.
