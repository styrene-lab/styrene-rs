# nomadnet-pages - Baseline

### Requirement: Native NomadNet page hosts interoperate

The daemon must announce a functioning `nomadnetwork.node` destination and serve standard `/page/...` and `/file/...` RNS request paths.

#### Scenario: Python NomadNet fetches a Rust page
Given a Rust node announces a page host containing a static Micron page
When Python NomadNet requests the corresponding `/page/...` path
Then it receives the canonical page bytes through the native request protocol
And no Styrene-specific envelope is required

#### Scenario: Python NomadNet fetches a Rust file
Given a Rust page host exposes an allowed file
When Python NomadNet requests the corresponding `/file/...` path
Then the file transfers through the native packet or resource response
And the received content passes integrity verification

### Requirement: Native NomadNet browsing is staged and diagnostic

Browsing must use path discovery, identity resolution, link establishment, native request transfer, parsing, and rendering with authoritative outcomes for every stage.

#### Scenario: Rust fetches a Python NomadNet page
Given a Python NomadNet host has announced and is reachable
When the operator opens a valid page address
Then Rust resolves the host, establishes a link, and requests the native page path
And the returned Micron source is rendered without Styrene-specific transport

#### Scenario: Identity resolution fails
Given path discovery succeeds for a page destination
And identity resolution fails
When the browse operation terminates
Then Identity Resolution is the failed stage
And Transfer, Parse, and Render are not reported as attempted successes

### Requirement: Micron navigation preserves source semantics

Static Micron pages must retain canonical source bytes while supporting rendering, relative links, source view, history, reload, cache status, and parser warnings.

#### Scenario: Relative page link is activated
Given a rendered page contains a relative NomadNet page link
When the operator activates it
Then the address resolves relative to the current host and path
And Back returns to the prior page without duplicating history

#### Scenario: Micron construct is unsupported
Given a valid page contains a construct the renderer cannot display
When the page is parsed
Then canonical source remains available
And the unsupported construct is reported without fabricating rendered content

### Requirement: Dynamic pages and access policy are safe

Submitted fields, executable page environment, link identity, `.allowed` policy, password redaction, and bounded execution must follow NomadNet request semantics.

#### Scenario: Dynamic page receives submitted fields
Given a page contains supported interactive fields
When the operator submits a field-bearing link
Then the native page request carries the submitted field map
And the resulting page replaces the active view through the normal staged lifecycle

#### Scenario: Page requires an allowed identity
Given a page or file is restricted by host policy
When an unidentified or unauthorized link requests it
Then the request is denied without executing the page
And the client receives an authorization-specific outcome
