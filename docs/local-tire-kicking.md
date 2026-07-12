# Local Tire-Kicking

The canonical application is the single `styrene` binary. It embeds the runtime; installing or launching `styrened` separately is not required.

## Fast path: isolated ghost session

From the repository root:

```bash
./scripts/tire-kick.sh
```

This builds an optimized binary and runs:

```bash
styrene --ghost
```

A ghost session:

- generates an ephemeral identity;
- uses an isolated private temporary directory;
- starts the Styrene runtime in-process;
- never reads or modifies the normal Styrene identity/configuration;
- displays `[GHOST]` in the top bar;
- deletes its session state on normal exit.

Quit with `q`, or press `Ctrl+C` twice within one second.

## Persistent local installation

Build and install the canonical binary into Cargo's binary directory:

```bash
cargo install --path crates/apps/styrene --features tui --locked --force
styrene
```

The first persistent launch presents onboarding and writes the normal platform configuration. Existing state is not needed before launch.

## Portable persistent session

Keep all state under a chosen directory:

```bash
./scripts/tire-kick.sh portable "$PWD/.local/styrene-portable"
```

Equivalent installed invocation:

```bash
styrene --portable "$PWD/.local/styrene-portable"
```

The TUI displays `[PORTABLE]`. Remove the directory to reset it.

## Direct development launch

```bash
cargo run --release -p styrene --features tui -- --ghost
```

The double `--` separates Cargo options from Styrene options.

## Initial checks

Inside the TUI:

1. Confirm the top bar says `[GHOST]`, `[PORTABLE]`, or `[STANDARD]` as expected.
2. Confirm the activity feed reports `embedded runtime ready` followed by `daemon connected`.
3. Move among Home, Peers, and Messages with `Tab` or `1`/`2`/`3`.
4. Open command input with `:` and search with `/`.
5. Exit with `q`; a ghost session should leave no active daemon process or reusable identity.

A lone node will show no peers until an interface reaches another Styrene/Reticulum node. That is expected and is not a startup failure.

## Useful verification commands

```bash
cargo test -p styrene-tui --lib -- --test-threads=1
cargo check -p styrene --features tui
cargo build --release -p styrene --features tui
```

## Known scope boundary

This is the first local product boot and UI assessment path. Multi-node topology and external bearer setup are separate tests; start with ghost mode to assess startup, onboarding bypass, terminal rendering, embedded runtime lifecycle, and shutdown hygiene.
