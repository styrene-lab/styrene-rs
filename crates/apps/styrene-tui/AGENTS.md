# styrene-tui

Terminal UI for the Styrene mesh daemon. Three-workspace Ratatui application that connects to a running `styrened` instance over Unix domain sockets (msgpack wire protocol) and degrades to demo mode when no daemon is available.

## What It Does

Displays mesh network state, peer activity, and LXMF conversations in a vim-style TUI with three workspaces, a persistent sidebar, and a modal input bar. Connects to `styrened` via IPC, subscribes to device/message/link event streams, and polls for status snapshots.

## Workspaces

| Workspace | Sidebar | Main Pane |
|-----------|---------|-----------|
| **Home** | Peer list (status-sorted) | Activity feed (reverse-chrono) + node status card + signal waveform |
| **Peers** | Peer list | Selected peer detail with 5 tabs: Status, Chat, Pages, Terminal, Commands |
| **Messages** | Conversation list (by last activity) | Chat thread with bubble rendering |

Switch workspaces with `Tab`/`Shift+Tab` or `1`/`2`/`3`.

## Input Modes

| Mode | Trigger | Prefix | Purpose |
|------|---------|--------|---------|
| Normal | default | none | Status display, keybind navigation |
| Command | `:` | `:` | Execute commands (`:quit`, `:connect`, `:disconnect`, `:help`) |
| Search | `/` | `/` | Filter sidebar items by query |
| Compose | `i` | `>` | Write chat message to selected peer |

`Esc` returns to Normal from any mode.

## Daemon Connection Modes

Determined by the onboarding wizard on first run, or loaded from saved preferences:

| Mode | Behavior |
|------|----------|
| `Embedded` | Not yet implemented; falls back to socket connect |
| `Background` | Spawns `styrened` as child process, retries connection for 6s |
| `ConnectExisting` | Connects to existing daemon socket (`$STYRENED_SOCKET` or `$XDG_RUNTIME_DIR/styrened/control.sock`) |

If all connection attempts fail, the TUI runs in demo mode. Press `r` for a demo announce, `l` for a demo link.

## Module Map

```
src/
  main.rs              Entrypoint, terminal setup, panic hook, 60fps event loop,
                       key dispatch (handle_key, handle_compose_key,
                       handle_command_key, handle_search_key), command parser
  app.rs               App struct (all state), workspace/focus/input-mode enums,
                       draw methods (top_bar, sidebar, main pane, input bar),
                       tick logic, demo data injection
  daemon.rs            DaemonHandle (Unix socket RPC), TuiEvent enum,
                       connect(), event_reader task, spawn_poll_task(),
                       apply_event() state applicator, wire payload parsers
  mesh_state.rs        PeerRecord, LinkRecord, ActivityLog (ring buffer),
                       PeerStatus/LinkStatus enums, epoch_secs() helper
  micron_widget.rs     Micron markup renderer (NomadNet page format)
  onboarding/
    mod.rs             TuiPrefs, DaemonMode detection
    detect.rs          Environment scanner (RNS config, NomadNet, Sideband)
    reticulum.rs       Reticulum config parsing
    screens.rs         Wizard screen rendering
    setup.rs           DaemonMode enum, SetupResult
    wizard.rs          WizardState, WizardAction, key handling
  tui/
    mod.rs             Re-exports all TUI submodules
    theme.rs           Theme trait + Alpharius dark theme
    effects.rs         tachyonfx effect manager (startup glow, zone-isolated)
    conversation.rs    ConversationView — segment list + scroll state
    conv_widget.rs     StatefulWidget for rendering conversation segments
    segments.rs        Segment enum (system, sent, received, protocol events),
                       DeliveryStatus, ProtocolEventKind
    editor.rs          Single-line text editor with readline keybinds
    signal.rs          RTT waveform renderer (CIE L* block chars)
    splash.rs          Startup splash with CRT convergence animation
    spinner.rs         Deterministic RNG for splash effects
    footer.rs          (legacy, largely unused — badges moved to top bar)
    topology.rs        TopologyState for future tree-mode sidebar
    widgets.rs         Shared widget utilities
```

## Key Types

| Type | Location | Role |
|------|----------|------|
| `App` | `app.rs` | All application state. Owns peers, links, conversations, effects, editor, theme. |
| `Workspace` | `app.rs` | `Home` / `Peers` / `Messages` |
| `InputMode` | `app.rs` | `Normal` / `Command { buffer }` / `Search { query }` / `Compose` |
| `Focus` | `app.rs` | `Sidebar` / `Main` / `Input` |
| `PeerTab` | `app.rs` | `Status` / `Chat` / `Pages` / `Terminal` / `Commands` |
| `TuiEvent` | `daemon.rs` | Events from daemon: Identity, Status, PeerAnnounce, Message, MessageStatus, LinkUpdate, Disconnected |
| `DaemonHandle` | `daemon.rs` | Owns the Unix socket connection. Methods: `identity()`, `status()`, `devices()`, `ping()`, subscribe_*() |
| `PeerRecord` | `mesh_state.rs` | Hash, name, first/last seen, hop count, status, link IDs |
| `LinkRecord` | `mesh_state.rs` | ID, peer hash, RTT, wave animation state |
| `ActivityLog` | `mesh_state.rs` | Ring buffer of `ActivityEntry` (capacity 64) |
| `ConversationView` | `tui/conversation.rs` | Segment list + scroll state for a single chat thread |
| `Segment` | `tui/segments.rs` | Sum type: system messages, sent/received messages, protocol events |

## Widget Inventory

| Widget | File | Description |
|--------|------|-------------|
| Top bar | `app.rs::draw_top_bar` | Brand, workspace tabs, badge counters |
| Sidebar | `app.rs::draw_sidebar` | Scrollable peer/conversation list with status icons |
| Activity feed | `app.rs::draw_home` | Timestamped protocol events with age display |
| Node status | `app.rs::draw_home` | Identity, daemon, mesh, propagation status lines |
| Signal waveform | `tui/signal.rs` | RTT sine waves in block characters |
| Conversation | `tui/conv_widget.rs` | Chat bubble renderer (left/right aligned, delivery status) |
| Editor | `tui/editor.rs` | Single-line input with Ctrl+U/K/W keybinds |
| Splash | `tui/splash.rs` | CRT convergence + glow + breathing animation |
| Peer status | `app.rs::draw_peer_status` | Device info, links, hops for selected peer |
| Peer tabs | `app.rs::draw_peers_workspace` | 5-tab bar (Status/Chat/Pages/Terminal/Commands) |
| Input bar | `app.rs::draw_input_bar` | Mode-aware: status / `:command` / `/search` / compose |
| Micron | `micron_widget.rs` | NomadNet page markup renderer |

## Test Commands

```bash
# Type check
cargo check -p styrene-tui

# Build
cargo build -p styrene-tui

# Run (connects to daemon or falls back to demo mode)
cargo run -p styrene-tui

# Full workspace validation
just validate
```

In demo mode (no daemon), press `r` to inject a fake peer announce and `l` to inject a fake link + inbound message.

## Known Stubs and TODOs

| Location | What |
|----------|------|
| `main.rs` embedded daemon | `DaemonMode::Embedded` falls back to socket connect; no in-process daemon bootstrap yet |
| `app.rs` Peers workspace | Pages, Terminal, Commands tabs render "coming soon" placeholder text |
| `app.rs` tree mode sidebar | `TopologyState` exists but tree-mode toggle (`t` key) is not wired |
| `main.rs` `:connect` command | Logs intent but does not actually establish a new daemon connection |
| `main.rs` search mode | Captures query text but does not filter sidebar items |
| `daemon.rs` event reader | 60s timeout keepalive cannot send ping due to shared lock; just continues |
| `tui/footer.rs` | Legacy module, badges migrated to top bar; file remains but is unused |
| Design spec Phase 5-11 | Pages tab, terminal tab, commands tab, settings overlay, full command suite, tree sidebar |
| Onboarding wizard | Functional but identity provisioning and interface setup are minimal |

## Current Status

Early operational. The shell, all three workspaces, splash screen, daemon IPC bridge, conversation rendering, effects system, onboarding wizard, and basic command mode are implemented. The TUI connects to a live `styrened`, displays real peers/links/messages, and allows chat composition. Peer detail tabs beyond Status and Chat are stubs. The command palette covers quit, connect (stub), disconnect, and help.

## Dependencies

Core: `ratatui` 0.30, `crossterm` 0.29, `tachyonfx` 0.25, `ratatui-textarea` 0.8, `tui-tree-widget` 0.24, `ratatui-toaster` 0.1.

Protocol: `styrene-rns` (identity/crypto types), `styrene-micron` (page markup).

Daemon: `styrene-ipc` (IPC types), `styrene-ipc-server` (wire protocol framing), `styrened` (default socket path).

Async: `tokio` (event loop, IPC tasks).
