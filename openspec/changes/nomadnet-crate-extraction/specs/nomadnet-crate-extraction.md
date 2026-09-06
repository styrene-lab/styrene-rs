# NomadNet crate extraction

## ADDED Requirements

### Requirement: Independent domain library
The NomadNet library SHALL have no IPC, daemon, UI or session-runtime dependency.
The daemon SHALL adapt public IPC contracts without changing their wire encoding.

#### Scenario: Content behavior without a daemon
Given the NomadNet crate and a Micron document
When its standalone tests execute
Then rendering and form encoding require no daemon, transport or IPC session

### Requirement: Content extraction preserves behavior
Projection SHALL preserve text, links, field order and warnings, omit password
values, and keep submission debug output redacted. Native request encoding SHALL
preserve field selection, validation bounds and MessagePack bytes. Binary response
decoding SHALL reject non-binary values and trailing bytes.

#### Scenario: Form content crosses the adapter
Given text, password and repeated checkbox fields
When the daemon projects the page and encodes a submission through the library
Then existing IPC field projection and native request bytes are unchanged
And password defaults are absent from the field projection

### Requirement: Coordinator extraction preserves ownership
The library SHALL ultimately own sessions, history, cache, downloads and bounded
link cleanup. Daemon adapters SHALL retain authorization, discovery and transport
integration. Route repair SHALL remain an RNS transport responsibility.

#### Scenario: Shared link survives one browser close
Given two browser sessions retain the same library-owned link
When one session closes
Then the other session retains its link
And a borrowed external link is not torn down

#### Scenario: Extraction does not claim cache repair
Given a behavior-preserving coordinator move
When the moved tests pass
Then cache expiry and failed-route recovery remain explicitly separate changes
