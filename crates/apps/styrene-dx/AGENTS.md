# styrene-dx

**Status: Spike / experimental. Not in default workspace members. `publish = false`.**

Dioxus 0.7 cross-platform UI experiment (desktop + web). Explores whether a single codebase can serve both targets vs the current ratatui TUI.

## Structure

```
src/
  main.rs          — Dioxus app root, routes
  state.rs         — App state (identity, mesh status)
  components/      — UI components
  assets/          — Static assets (CSS, images)
```

## Build

```bash
cargo build -p styrene-dx          # desktop
dx serve                           # dev server (requires dioxus-cli)
```

Not included in `cargo test --workspace` or default members. Build explicitly with `-p styrene-dx`.

## Dependencies

- `dioxus 0.7` (desktop + web features)
- `styrene-rns` (data types only, no transport)
- `rand_core`, `hex`, `log`

## Notes

- This is a spike to evaluate Dioxus, not a shipping product
- The primary TUI is `styrene-tui` (ratatui)
- If this spike proves out, it would replace or supplement the TUI for desktop/web use cases
