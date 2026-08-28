# Deliver Mobile Messaging Minimum

## Intent

Turn the existing iOS and Android reference hosts into a useful Reticulum and
LXMF mobile product with the smallest defensible interoperability claim. The
mobile product must preserve one identity, connect to a configured public TCP
interface, discover canonical peers, exchange durable text messages, and use a
selected standard LXMF propagation node for offline delivery.

Skywave provides the observed connectivity and discovery floor. The pinned
Python LXMF round trip against Brutus provides the propagation behavior target.

## Scope

This change adds persistent network and propagation-node configuration, truthful
connection and discovery state, durable text conversation workflows, explicit
delivery evidence, standard propagation upload and synchronization controls, and
one shared cross-platform acceptance corpus. It delivers the behavior through
the current SwiftUI and Compose reference hosts and the embedded Rust runtime.

This change does not require a renderer migration. A later shared Dioxus client
must satisfy the same behavioral scenarios. This change excludes mobile
propagation hosting, automatic propagation-node selection, attachments, Paper
delivery, NomadNet parity, advanced route or link administration, fleet, tunnel,
and Lab workflows.

RNode behavior remains governed by `stabilize-mobile-platform-hosts`. A platform
must not claim RNode support until its applicable physical-device gates pass.

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
- The same deterministic state and behavior corpus passes against both native
  hosts, followed by public-Brutus and applicable physical-device gates.
