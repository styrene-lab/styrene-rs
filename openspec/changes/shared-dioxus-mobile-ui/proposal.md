# Shared Dioxus Mobile UI

## Intent

Deliver one Rust-owned Dioxus application without maintained Swift or Kotlin
hosts, adapters, or product surfaces.

## Scope

This change adds iOS and Android Dioxus packaging and a shared mobile shell. It
adds shared product workflows, Rust platform-service traits and implementations,
embedded sessions, and accessibility coverage.

This change does not move protocol behavior into UI code or remove Ratatui. It
excludes mobile Lab and unrestricted Admin workflows.

This change depends on `extract-styrene-ui-repository`.

## Success criteria

- iOS and Android render the primary mobile product from shared Dioxus component
  source.
- Both platforms consume the same typed backend session and presentation
  reducers.
- The versioned Dioxus mobile UX corpus governs framework, WebView, adaptive
  layout, platform interaction, accessibility, and evidence decisions.
- Maintained mobile product and platform-service source is Rust; generated
  packaging output is not committed.
- Bluetooth-first RNode behavior and explicit Android USB fallback retain their
  validated lifecycle and evidence semantics.
- Shared corpus, simulator, emulator, accessibility, and available physical
  device gates pass on the Dioxus application.
