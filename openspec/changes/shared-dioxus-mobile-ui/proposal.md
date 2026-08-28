# Shared Dioxus Mobile UI

## Intent

Replace the duplicated SwiftUI and Compose product surfaces with one Dioxus
application while retaining thin native adapters for operating-system services.

## Scope

This change adds iOS and Android Dioxus packaging and a shared mobile shell. It
adds shared product workflows, platform-service traits, native adapters,
embedded sessions, accessibility coverage, and incremental native-host retirement.

This change does not move protocol behavior into UI code or remove Ratatui. It
excludes mobile Lab, unrestricted Admin workflows, and premature native-host
removal.

This change depends on `extract-styrene-ui-repository`.

## Success criteria

- iOS and Android render the primary mobile product from shared Dioxus component
  source.
- Both platforms consume the same typed backend session and presentation
  reducers.
- Swift and Kotlin code is limited to declared platform-service and packaging
  responsibilities.
- Bluetooth-first RNode behavior and explicit Android USB fallback retain their
  validated lifecycle and evidence semantics.
- Shared corpus, simulator, emulator, accessibility, and available physical
  device gates pass before duplicate native screens are removed.
