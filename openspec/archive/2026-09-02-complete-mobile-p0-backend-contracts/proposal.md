# Complete Mobile P0 Backend Contracts

## Intent

Complete and verify the backend-owned P0 mobile contracts that are unsafe,
missing, or incomplete at revision
`7dbe68e0e7a2e5e657e4c6c55b304a6a009ab992`. Keep this work independent from
the frontend branch, which can continue integrating the existing stable draft,
send, retry, unread, lifecycle, receipt, contact, and standard-propagation APIs.

## Scope

- Add one versioned backend P0 implementation corpus that references the
  existing mobile integration cases and application-parity journeys instead of
  creating another product or protocol authority.
- Correct legacy propagation polling so remote deletion follows durable local
  acceptance and notification previews are Unicode-safe.
- Make production identity custody fail closed, expose typed custody state, and
  persist editable public identity metadata.
- Distinguish a ready offline runtime from stopped transport, preserve session
  generation in capability and interface observations, and expose typed boot,
  degradation, and recovery reasons.
- Add durable conversation creation from canonical discovery, resolve contact
  aliases into conversation summaries, and preserve empty-conversation truth.
- Correlate message attempts with route, interface, and bearer evidence without
  inferring evidence that was not observed.
- Add bounded, chronological, payload-free mobile diagnostics and explicit
  storage recovery state with forced-termination tests.
- Exclude frontend reducers and components, native platform adapters, packaged
  iOS and Android execution, external application observations, protocol parity
  claims, attachments, pages, background scheduling, and notification delivery.

## Success criteria

- Every backend P0 corpus row names its existing integration case and parity
  journey, current contract state, implementation surfaces, required tests,
  forbidden outcomes, frontend handoff, and exclusions.
- Corpus validation rejects unknown P0 cases, unknown parity journeys, duplicate
  ownership, incomplete assertions, and promotion of host or packaged evidence.
- Rejected or non-durable propagation messages are never acknowledged, and
  arbitrary valid UTF-8 content cannot panic preview generation.
- Production secure-storage selection cannot silently use plaintext, and the
  public custody projection contains no secret material.
- Offline readiness, capability generation, interface failure, conversation
  identity, route evidence, diagnostics, and storage recovery are represented
  by authoritative typed backend state.
- Focused backend tests, restart and fault tests, formatting, warning-denied
  Clippy, and OpenSpec validation pass before this change is complete.
