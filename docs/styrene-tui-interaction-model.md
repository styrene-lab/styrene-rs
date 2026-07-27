---
id: styrene-tui-interaction-model
title: "Styrene TUI Interaction Model"
status: decided
tags: [tui, ux, keybindings, interaction]
open_questions: []
dependencies: []
related: []
---

# Styrene TUI Interaction Model

## Overview

Design a low-memory, discoverable interaction system for Styrene's terminal UI. The model must use a small invariant key vocabulary, contextual actions, explicit focus, and equivalent mouse paths rather than Vim-style command memorization. Existing implementation has workspaces, focus enum, editor modes, scattered key handlers, mouse capture, and a static footer, but lacks a coherent selection→activation→action loop.

## Research

### Current interaction implementation audit

Current implementation evidence: main input routing is split among handle_key_event, handle_compose_key, handle_search_key, and handle_mouse_event in crates/apps/styrene-tui/src/lib.rs. Tab currently calls next_workspace globally; Enter mutates sidebar selections using workspace-specific branches; mouse supports wheel and hard-coded workspace-tab columns only. App has Workspace, Focus, InputMode, sidebar_selection, selected_peer, selected_conversation, PeerTab, and action-capable daemon commands. Footer help in app.rs is static text ("tab switch workspace  ↑↓ scroll  / search  d demo  q quit") and is therefore inaccurate/context-insensitive. Ratatui Rects are not retained as a hit map, so rendered peer rows/content controls cannot be clicked reliably. This confirms the missing abstraction is a semantic action registry plus per-frame interaction map—not more ad hoc key branches.

## Decisions

### Small invariant interaction vocabulary

**Status:** accepted

**Rationale:** Adopt a universal baseline across every workspace: arrows move, Tab/Shift-Tab move focus, Enter activates, Esc backs out, Space toggles/selects, typing enters text only in a text-capable focus, ?/F1 opens help, and Ctrl+P opens the action palette. No action may require memorizing a workspace-specific letter binding.

### Context footer and action palette are authoritative

**Status:** accepted

**Rationale:** The footer must derive from the active focus and selection and show no more than five currently valid actions. The searchable action palette lists all currently available actions, shortcuts, and disabled reasons. Static hard-coded help strings are removed.

### Commands are semantic and input methods are adapters

**Status:** accepted

**Rationale:** Define one command/action registry for navigation and operations. Keyboard, mouse hit-testing, palette selection, footer clicks, and future accessibility surfaces dispatch the same semantic Action values. Rendering code must not own business actions.

### Progressive disclosure over mode proliferation

**Status:** accepted

**Rationale:** Normal navigation has no hidden letter-command mode. Text editing is visibly indicated and bounded to editor/search fields; modal confirmation is reserved for destructive operations. Power-user aliases may exist but are never the only path and never appear as the primary teaching surface.

### Workspace switching uses Ctrl+Left and Ctrl+Right

**Status:** accepted

**Rationale:** Operator confirmed explicit workspace switching through Ctrl+Left/Ctrl+Right. Tab and Shift-Tab are reserved exclusively for moving focus among visible regions.

### Consequential operations require explicit confirmation

**Status:** accepted

**Rationale:** Operator approved confirmation boundaries. Remote command execution, tunnel closure, peer removal or blocking, identity replacement, and persistent-data destruction require an explicit confirmation surface. Routine navigation, selection, messaging, and non-destructive inspection do not.

### Keyboard completeness is mandatory; mouse is equivalent additive input

**Status:** accepted

**Rationale:** Resolved the terminal-mouse assumption conservatively: mouse reporting may be absent or disabled, so every operation must remain fully keyboard-operable. When available, mouse hit regions dispatch the same semantic actions rather than separate behavior.

## Open Questions

None. The operator approved the workspace-switching and confirmation policies; keyboard completeness removes reliance on terminal mouse support. `Ctrl+P` is the action palette, while `?` and `F1` open contextual help. No numeric workspace shortcuts are required by the baseline vocabulary.
