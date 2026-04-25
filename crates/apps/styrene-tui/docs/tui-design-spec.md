---
id: styrene-tui-design
title: "Styrene TUI: Visual Design Specification"
status: draft
date: 2026-04-18
---

# Styrene TUI: Visual Design Specification

## 1. Design Philosophy

The Styrene TUI is an operator's primary interface to a Reticulum mesh network. It must do three things well:

1. **Show the mesh** — who's online, what's changed, what needs attention
2. **Enable communication** — read, write, and browse content naturally
3. **Feel alive** — the mesh is a living network; the TUI should breathe with it

### Anti-Patterns to Avoid

- **Feature screens.** The Python TUI has 13 screens because it has 13 features. Users don't think in features — they think in tasks.
- **Hidden navigation.** Nine global keybindings nobody discovers. The grave key for settings.
- **Mode confusion.** Three surfaces for the same concept (Nodes, Contacts, MeshDeviceDetail). Users can't build a mental model.
- **Terminal cosplay.** Textual-style widget density that fights the medium. A terminal is good at text, lists, and tables — not form builders.

### Design Principles

- **Three workspaces, not thirteen screens.** Home, Peers, Messages. Everything else is a command or a panel.
- **One peer concept.** A peer is a peer. Contacts are just peers with names. The tree view is an option, not a workspace.
- **Content is king.** The main pane should always show content worth reading — a conversation, a page, a status panel. Never empty, never just a table.
- **The mesh breathes.** Subtle visual effects convey network state. Borders pulse with activity. Signal strength is a waveform, not a number.

---

## 2. Layout Architecture

### 2.1 The Shell

```
┌──────────────────────────────────────────────────────────────────────┐
│  ■ styrene    Home · Peers · Messages              3↑  ●12  ◐2  ○1 │  ← TOP BAR
├──────────┬───────────────────────────────────────────────────────────┤
│          │                                                           │
│  SIDEBAR │                    MAIN PANE                              │
│          │                                                           │
│  context │  (content changes based on workspace + selection)         │
│  list    │                                                           │
│          │                                                           │
│          │                                                           │
│          │                                                           │
│          │                                                           │
│          │                                                           │
│          │                                                           │
│          │                                                           │
│          │                                                           │
├──────────┴───────────────────────────────────────────────────────────┤
│ > _                                                          [mesh] │  ← INPUT BAR
└──────────────────────────────────────────────────────────────────────┘
```

Four persistent zones, always present:

| Zone | Height | Purpose |
|------|--------|---------|
| **Top Bar** | 1 line | Workspace tabs, mesh health badges, unread count |
| **Sidebar** | Fill | Context-sensitive list (peers, conversations, tree) |
| **Main Pane** | Fill | Primary content (detail, chat thread, page, feed) |
| **Input Bar** | 1-3 lines | Command input, search, chat compose, status |

### 2.2 Proportions

- **Sidebar width:** 28 columns (fixed). Enough for peer names + status symbols. Collapsible with `[` / `]` or auto-hides below 80 columns.
- **Main pane:** Remaining width. Minimum 50 columns.
- **Input bar:** 1 line default. Expands to 3 when composing multi-line messages (Enter inserts newline in compose mode; Ctrl+Enter sends).
- **Total minimum:** 80x24 (standard terminal). Optimized for 120x40+.

### 2.3 Responsive Behavior

| Terminal Width | Layout |
|---------------|--------|
| < 60 cols | Sidebar hidden; main pane full-width; `Tab` toggles sidebar overlay |
| 60-79 cols | Sidebar 22 cols; main pane rest |
| 80-119 cols | Sidebar 28 cols; main pane rest |
| 120+ cols | Sidebar 28 cols; main pane rest; extra space for content padding |

---

## 3. Workspaces

### 3.1 Home

**Purpose:** "What's happening on my mesh right now?"

```
┌──────────┬───────────────────────────────────────────────────────────┐
│  PEERS   │  ACTIVITY FEED                                           │
│          │                                                           │
│ ● Alice  │  14:32  ● Bob announced (3 hops, TCP)                    │
│ ● Bob    │  14:31  ↑ Message delivered to Alice                     │
│ ◐ Carol  │  14:28  ← Alice: "meeting at 3?"                        │
│ ○ Dave   │  14:25  ● Carol went stale (last seen 12m ago)           │
│          │  14:20  ⬡ New link established: Bob ↔ Eve (4.2ms RTT)   │
│          │  14:15  ↓ Propagation sync: 3 messages queued            │
│          │                                                           │
│          │  ─── NODE STATUS ────────────────────────                 │
│          │  Identity: a3f8c21d  Uptime: 4h 12m                      │
│          │  Links: 3 active   Interfaces: 2 (TCP, UDP)              │
│          │  Propagation: enabled (12 stored)                         │
│          │  ▁▂▃▅▆▇█▇▆▅▃▂▁  signal: 3 peers in range               │
└──────────┴───────────────────────────────────────────────────────────┘
```

**Sidebar:** Online peers sorted by status (● active, ◐ stale, ○ lost). Shows unread badge if messages waiting. Selecting a peer jumps to Peers workspace with that peer focused.

**Main pane (top):** Reverse-chronological activity feed. Mesh events, message delivery, announces, link changes. Each line is one event with timestamp, icon, and summary.

**Main pane (bottom):** Node status card — identity hash, uptime, active links, interfaces, propagation state. Signal waveform (CIE L* interpolated) showing mesh activity intensity.

**Visual effects:**
- Activity feed entries fade from accent → muted as they age (HSL lightness decay over 60s)
- Signal waveform: continuous sine, "pluck" animation on packet receipt (amplitude spike, 1s decay)
- New peer announce: sidebar row briefly glows accent (300ms hsl_shift_fg, QuadOut)

### 3.2 Peers

**Purpose:** "Interact with someone on the mesh."

```
┌──────────┬───────────────────────────────────────────────────────────┐
│  PEERS   │  ■ Alice  a3f8c21d                              ● ACTIVE │
│          │  ─────────────────────────────────────────────────────────│
│ ▸ Alice  │  Status │ Chat │ Pages │ Terminal │ Commands              │
│   Bob    │  ─────────────────────────────────────────────────────────│
│   Carol  │                                                           │
│   Dave   │  Device:    Styrene Node v0.10.70                         │
│   Eve    │  Uptime:    2d 14h 32m                                    │
│          │  Links:     2 active (RTT: 4.2ms, 12.8ms)                │
│  ── tree │  Hops:      3 via TCP→hub.vanderlyn.local                │
│  Hub-A   │  Signal:    ▁▂▃▅▇▅▃▂▁ (good)                            │
│   ├ Bob  │  Propagation: client (last sync: 5m ago)                  │
│   ├ Eve  │                                                           │
│   └ Fn   │  Capabilities: chat, rpc, pages, terminal                 │
│  Hub-B   │  First seen: 2026-04-12  Announces: 847                   │
│   └ Carol│  Identity:  ed25519:a3f8...c21d                           │
└──────────┴───────────────────────────────────────────────────────────┘
```

**Sidebar:** All discovered peers. Two modes toggled with `t`:
- **List mode** (default): Flat list sorted by status then name. Search with `/`.
- **Tree mode**: Mesh topology tree (hub → child nodes). Shows routing structure.

Peers with aliases show the alias. Peers with unread messages show a count badge.

**Main pane:** Selected peer's detail, with tab bar:

| Tab | Content |
|-----|---------|
| **Status** | Device info, uptime, links, hops, signal, capabilities, identity |
| **Chat** | Conversation thread with this peer. Input bar becomes compose. |
| **Pages** | Micron page browser (if peer hosts pages). Renders inline. |
| **Terminal** | Remote shell session (if peer supports terminal). ANSI rendering. |
| **Commands** | Structured RPC (status query, exec, reboot, config update). |

**Tab switching:** Number keys `1-5` or arrow keys on the tab bar.

**Visual effects:**
- Selecting a new peer: main pane content fades in via `coalesce_from()` (200ms, SineOut)
- Tab switch: subtle sweep_in from the direction of the tab (left tab → sweep from left, 150ms)
- Link quality indicator: heat-colored border on the status card (border lerps toward accent with signal strength)
- Terminal tab: raw ANSI passthrough — tachyonfx effects disabled in this zone to avoid interference

### 3.3 Messages

**Purpose:** "Read and write messages."

```
┌──────────┬───────────────────────────────────────────────────────────┐
│  CONVOS  │                                                           │
│          │  Alice                                        14:28 today │
│ ▸ Alice 2│  ─────────────────────────────────────────────────────────│
│   Bob    │                                                           │
│   Carol 1│       meeting at 3?                              14:28   │
│          │                                                           │
│          │                                             sure, west    │
│          │                                             conference    │
│          │                                             room? ✓ 14:30 │
│          │                                                           │
│          │       sounds good, see you there                 14:31   │
│          │                                                           │
│          │                                                           │
│          │                                                           │
├──────────┴───────────────────────────────────────────────────────────┤
│ > _                                                                  │
└──────────────────────────────────────────────────────────────────────┘
```

**Sidebar:** Conversation list sorted by last activity. Shows peer name (or alias), unread count badge, and relative timestamp. Search with `/` filters conversations.

**Main pane:** Active conversation thread. Messages rendered as bubbles:
- Inbound messages: left-aligned, accent-colored name header
- Outbound messages: right-aligned, muted border
- Delivery status: ✓ sent, ✓✓ delivered, ✗ failed (inline after timestamp)
- System events (link established, peer stale): centered, dim, italic

**Input bar:** Chat compose mode activates automatically when Messages workspace is focused and a conversation is selected. `Enter` sends. `Shift+Enter` or `Alt+Enter` inserts newline. `Esc` deselects conversation (returns focus to sidebar).

**Visual effects:**
- New inbound message: brief accent glow on the message line (200ms hsl_shift_fg, QuadOut)
- Delivery confirmation: ✓ symbol fades from bright to muted (500ms)
- Unread badge in sidebar: subtle pulse (2000ms ping_pong, 0.08 lightness oscillation)

---

## 4. Input Bar

The input bar is always present and serves multiple roles based on context:

| Context | Behavior | Prefix |
|---------|----------|--------|
| **Default** | Status display (mesh summary, last event) | none |
| **Command mode** | Execute commands (settings, announce, etc.) | `:` |
| **Search mode** | Filter sidebar items | `/` |
| **Compose mode** | Write a chat message (in Messages or Peers→Chat) | `>` |

### 4.1 Commands

Commands replace global keybindings for non-navigation actions:

```
:settings              Open settings panel (right overlay)
:announce              Send mesh announce
:provision             Open provisioning wizard
:connect <host:port>   Add TCP interface
:block <peer>          Block a peer
:unblock <peer>        Unblock a peer
:alias <peer> <name>   Set peer alias
:export                Export mesh state
:help                  Show command reference
```

Command mode activates when `:` is typed in default state. Tab completion for command names and peer hashes/aliases.

### 4.2 Search

`/` activates search. Types filter the active sidebar:
- In Home: filters peer list by name/hash
- In Peers: filters peer list or tree
- In Messages: filters conversation list by peer name or message content

`Esc` exits search. `Enter` selects the first match.

---

## 5. Settings Panel

Settings is a right-side overlay panel, not a full-screen takeover. Triggered by `:settings` command.

```
┌──────────┬────────────────────────────┬──────────────────────────────┐
│  SIDEBAR │      MAIN PANE             │  SETTINGS                    │
│          │      (dimmed)              │                              │
│          │                            │  ■ Identity                   │
│          │                            │  Name: Wilson                 │
│          │                            │  Icon: ⬡                     │
│          │                            │                              │
│          │                            │  ■ Network                    │
│          │                            │  Role: full_node              │
│          │                            │  Propagation: enabled         │
│          │                            │                              │
│          │                            │  ■ Appearance                 │
│          │                            │  Theme: alpharius             │
│          │                            │                              │
│          │                            │  ■ Advanced ▸                 │
│          │                            │                              │
├──────────┴────────────────────────────┴──────────────────────────────┤
│ :settings                                                            │
└──────────────────────────────────────────────────────────────────────┘
```

Settings panel width: 32 columns. Main pane dims (darken effect, 40% lightness reduction). `Esc` or `:settings` again closes the panel.

**Settings sections:**
- **Identity** — display name, icon, short name
- **Network** — node role, propagation, announce interval
- **Appearance** — theme selection
- **Advanced** — RBAC, relay, RNS config overrides, interface management

Advanced section expands inline (no separate screen). Each setting is a single line: label + value. `Enter` on a setting opens inline edit. `Esc` cancels edit.

**Visual effects:**
- Panel slides in from right (`slide_in(Direction::Right, 200ms, SineOut)`)
- Main pane dims (`darken(0.4, 200ms, SineOut)`)
- Panel slides out on close (reverse)

---

## 6. Visual Language

### 6.1 Color Palette (Alpharius Theme)

```
BACKGROUND
  bg:           Rgb(2, 4, 8)        Near-black with blue tint
  card:         Rgb(4, 10, 18)      Subtle lift for panels
  surface:      Rgb(8, 16, 28)      Elevated surfaces (settings panel)

BORDER
  border:       Rgb(48, 112, 140)   Cool mid-blue
  border-dim:   Rgb(36, 80, 104)    Quieter blue (inactive)
  border-hot:   Rgb(42, 180, 200)   Accent (active/focused)

TEXT
  fg:           Rgb(196, 216, 228)  Bright cool white
  muted:        Rgb(108, 136, 152)  Mid-blue gray
  dim:          Rgb(72, 100, 124)   Dark blue gray

ACCENT
  accent:       Rgb(42, 180, 200)   Teal — primary brand, active states
  accent-muted: Rgb(26, 136, 152)   Quieter teal
  accent-bright:Rgb(110, 202, 216)  Vivid teal for highlights

SIGNAL (semantic)
  success:      Rgb(26, 184, 120)   Green — online, delivered
  error:        Rgb(224, 72, 72)    Red — failed, lost
  warning:      Rgb(200, 100, 24)   Orange — stale, degraded
  caution:      Rgb(120, 184, 32)   Lime — attention needed
```

### 6.2 Status Iconography

| Symbol | Meaning | Color |
|--------|---------|-------|
| `●` | Active (seen < 5min) | success |
| `◐` | Stale (5-30min) | warning |
| `○` | Lost (> 30min) | dim |
| `✓` | Sent | muted |
| `✓✓` | Delivered | success |
| `✗` | Failed | error |
| `⬡` | Mesh/Styrene node | accent |
| `↑` | Outbound | accent-muted |
| `↓` | Inbound | accent |
| `←` | Received message | accent |
| `→` | Sent message | muted |

### 6.3 Signal Waveform

The signal waveform is a continuous visualization of mesh activity:

```
▁▂▃▅▆▇█▇▆▅▃▂▁
```

Rendered using block characters with CIE L* perceptual gamma correction:
- Low activity: dark teal (near bg)
- Medium activity: full teal (accent)
- High activity: bright teal → amber transition
- Packet receipt: amplitude "pluck" (spike to max, decay over 1s via sine curve)

The waveform occupies 1 line in the footer of the node status card. Width adapts to available space (min 12, max 48 characters).

---

## 7. Effects System

### 7.1 Design Rules

1. **Effects reinforce state, never obscure it.** A glow means "something happened." A pulse means "attention needed." Never decorative-only.
2. **Subtlety over spectacle.** HSL lightness shifts of 0.05-0.15. Never full-color replacement.
3. **Zone isolation.** Each zone (sidebar, main, input) has its own effect manager. Effects never bleed across zones.
4. **CellFilter::Text always.** Effects apply to text content only, never borders or structural characters.

### 7.2 Effect Catalog

| Effect | Trigger | Zone | Timing | Details |
|--------|---------|------|--------|---------|
| **Peer announce glow** | New peer appears in sidebar | Sidebar | 300ms, QuadOut | hsl_shift_fg +0.15L on the new row |
| **Message flash** | Inbound message arrives | Main | 200ms, QuadOut | hsl_shift_fg +0.12L on message line |
| **Delivery confirm** | ✓ → ✓✓ status change | Main | 500ms, SineOut | fade_fg from accent-bright to muted |
| **Unread pulse** | Conversation has unread | Sidebar | 2000ms, ping_pong | hsl_shift_fg ±0.08L on unread badge |
| **Signal pluck** | Packet received | Main (status) | 1000ms, SineOut | Waveform amplitude spike + decay |
| **Settings slide-in** | `:settings` command | Main + overlay | 200ms, SineOut | slide_in + darken(0.4) |
| **Settings slide-out** | Close settings | Main + overlay | 200ms, SineIn | slide_out + lighten(0.4) |
| **Tab sweep** | Tab switch in Peers | Main | 150ms, SineOut | sweep_in from tab direction |
| **Peer select coalesce** | Select new peer in sidebar | Main | 200ms, SineOut | coalesce_from on new content |
| **Splash convergence** | Startup | Full screen | 1.7s, 22fps | CRT glitch → character convergence |
| **Splash glow** | Post-convergence | Full screen | 300ms+400ms | brightness spike +0.20L then fade |
| **Splash breathing** | After glow | Full screen | 2500ms, ping_pong | hsl_shift_fg ±0.06L, never_complete |
| **Splash dissolve** | Any key / connection | Full screen | 300ms, QuadIn | dissolve() → main UI |
| **Border heat** | Mesh activity level | All borders | Continuous | Lerp border color → accent based on activity rate |

### 7.3 Splash Sequence

```
T=0.0s   Black screen
T=0.1s   Hex mesh sigil appears with CRT noise glyphs (▓▒░█▄▀⬡)
T=0.1-1.8s  Characters converge center-outward (deterministic RNG, 45ms/frame)
T=1.8s   Full sigil revealed
T=1.8s   Post-convergence glow (+0.20L, 300ms QuadOut)
T=2.1s   Glow fades (-0.20L, 400ms QuadIn)
T=2.5s   Breathing begins (±0.06L, 2500ms sine, infinite)
T=2.5s+  "Connecting to daemon..." status text appears below sigil
T=?      Connection established → dissolve(300ms) → Home workspace
```

---

## 8. Keyboard Map

### 8.1 Global (Always Active)

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Next / previous workspace |
| `1` `2` `3` | Jump to Home / Peers / Messages |
| `:` | Enter command mode |
| `/` | Enter search mode |
| `Esc` | Back / cancel / deselect |
| `Ctrl+C` | Quit (double-press from any state) |
| `?` | Toggle help overlay |

### 8.2 Sidebar Navigation

| Key | Action |
|-----|--------|
| `j` / `↓` | Next item |
| `k` / `↑` | Previous item |
| `Enter` | Select / open |
| `g` | Jump to top |
| `G` | Jump to bottom |
| `t` | Toggle list/tree mode (Peers workspace) |
| `[` / `]` | Collapse / expand sidebar |

### 8.3 Main Pane

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down |
| `k` / `↑` | Scroll up |
| `1-5` | Switch tabs (Peers workspace) |
| `i` | Focus input / compose |
| `r` | Refresh current view |

### 8.4 Input Bar

| Key | Action |
|-----|--------|
| `Enter` | Send message / execute command / select search result |
| `Shift+Enter` | Insert newline (compose mode) |
| `Esc` | Exit mode / deselect |
| `Ctrl+R` | Reverse search (history) |
| `Ctrl+K` / `Ctrl+U` | Kill line (readline) |
| `Ctrl+Y` | Yank (paste from kill ring) |
| `↑` / `↓` | History navigation (when input empty) |

---

## 9. Content Rendering

### 9.1 Micron Markup

NomadNet pages use Micron markup. The TUI must render these inline in the main pane:

- **Headers:** Bold, accent-colored
- **Body text:** Default fg, wrapped to pane width
- **Links:** Underlined accent, navigable with `Enter`
- **Code blocks:** `surface` bg, monospace (already monospace in terminal)
- **Dividers:** `border-dim` horizontal rule

The existing `styrene-micron` crate handles parsing. Rendering maps micron nodes to ratatui `Line`/`Span` sequences.

### 9.2 Chat Messages

Messages render as aligned bubbles with semantic coloring:

```
  Alice                                              14:28
  meeting at 3?

                                          sure, west conference
                                          room?              ✓ 14:30

  sounds good, see you there                         14:31
```

- Inbound: left-aligned, peer name in accent-bold above first message in a group
- Outbound: right-aligned, no name header (it's you)
- Timestamps: dim, right-aligned for inbound, inline after content for outbound
- Delivery status: after timestamp, colored by state (muted=sent, success=delivered, error=failed)
- Grouping: consecutive messages from same sender within 5 minutes collapse into one block (no repeated name/timestamp)

### 9.3 Terminal Output

ANSI escape sequence passthrough for remote terminal sessions. The main pane becomes a raw terminal emulator:

- tachyonfx effects disabled in terminal zone
- Scrollback buffer: 10,000 lines
- Mouse events forwarded to remote PTY
- `Ctrl+\` exits terminal mode (returns to Peers workspace)

---

## 10. Implementation Strategy

### 10.1 Build Order

| Phase | Deliverable | Wired To |
|-------|------------|----------|
| **0** | Shell (top bar + sidebar + main + input) | — |
| **1** | Home workspace (peer list + activity feed + node status) | `query_status`, `query_devices`, `subscribe_devices` |
| **2** | Messages workspace (conversation list + chat thread + compose) | `query_conversations`, `query_messages`, `send_chat`, `mark_read`, `subscribe_messages` |
| **3** | Peers workspace (peer detail + status tab) | `query_devices`, `device_status`, `query_path_info` |
| **4** | Peers→Chat tab (reuse Messages conversation renderer) | same as Messages |
| **5** | Peers→Pages tab (micron rendering) | page browsing (future IPC method) |
| **6** | Peers→Terminal tab | `terminal_open`, `terminal_input`, `terminal_resize`, `terminal_close` |
| **7** | Peers→Commands tab | `exec`, `reboot_device`, `self_update` |
| **8** | Settings panel overlay | `query_config`, `save_config`, `set_auto_reply` |
| **9** | Splash screen with convergence animation | — |
| **10** | Effects pass (glow, pulse, sweep, heat borders) | — |
| **11** | Command mode (`:settings`, `:announce`, etc.) | various |

### 10.2 Crate Dependencies

```toml
[dependencies]
ratatui = "0.30"
crossterm = { version = "0.28", features = ["event-stream"] }
tachyonfx = { version = "0.25", features = ["sendable"] }
tui-textarea = "0.7"
styrene-ipc = { path = "../../libs/styrene-ipc" }
styrene-micron = { path = "../../libs/styrene-micron" }
tokio = { version = "1", features = ["full"] }
```

### 10.3 Module Structure

```
styrene-tui/src/
  main.rs               Entry, terminal setup, event loop, panic hook
  app.rs                App struct, workspace routing, state ownership
  daemon.rs             IPC bridge, TuiEvent, async event pump
  workspaces/
    home.rs             Home workspace (activity feed, node status)
    peers.rs            Peers workspace (detail tabs, tree mode)
    messages.rs         Messages workspace (conversation thread)
  panels/
    settings.rs         Settings overlay panel
    help.rs             Help overlay
  widgets/
    activity_feed.rs    Timestamped event list with fade
    peer_list.rs        Sidebar peer list (list + tree modes)
    conversation.rs     Chat thread renderer (bubbles, grouping)
    node_status.rs      Status card with signal waveform
    tab_bar.rs          Horizontal tab selector
    micron.rs           Micron markup renderer
    terminal.rs         ANSI terminal emulator
    signal.rs           Signal waveform (CIE L*)
  theme.rs              Theme trait + Alpharius implementation
  effects.rs            tachyonfx EffectManager per zone
  input.rs              Input bar (command/search/compose modes)
  editor.rs             Text editor with readline keybinds
```
