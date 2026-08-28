# Deliver Mobile Messaging Minimum

## Intent

Deliver a useful Rust-owned Dioxus Reticulum and LXMF mobile product with the
smallest defensible interoperability claim. The
mobile product must preserve one identity, connect to a configured public TCP
interface, discover canonical peers, exchange durable text messages, and use a
selected standard LXMF propagation node for offline delivery.

Skywave provides the observed connectivity and discovery floor. The pinned
Python LXMF round trip against Brutus provides the propagation behavior target.

## Scope

This change adds persistent network and propagation-node configuration, truthful
connection and discovery state, durable text conversation workflows, explicit
delivery evidence, standard propagation upload and synchronization controls, and
one shared cross-platform acceptance corpus. One Dioxus application and its
Rust-owned embedded runtime deliver the behavior on iOS and Android.

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
