# app-shell - Baseline

### Requirement: Pages consume typed domain state

Primary pages must consume typed stores and view models rather than raw IPC maps or shared application-level signals.

#### Scenario: Snapshot and event update the same entity
Given a store has loaded a snapshot
When a newer incremental event for an entity arrives
Then the store applies it deterministically
And selectors expose one current entity state

#### Scenario: Page has no data
Given a primary page has completed loading with no records
When it renders
Then it shows a domain-specific empty state
And does not present the state as loading or failed

#### Scenario: Page data is degraded
Given current data is partial, stale, or unsupported
When a primary page renders
Then it identifies the degraded source or missing capability
And preserves available read-only information

### Requirement: Primary workflows have stable routes and persistent context

The console must provide Command, Network, Messages, Fleet, Propagation, Content, Lab, and System routes with persistent runtime, identity, alert, activity, and selection context.

#### Scenario: Operator navigates between workflows
Given the console has an active backend session
And an entity is selected in a primary route
When the operator navigates to another primary route
Then runtime and identity context remain visible
And the selected entity remains available to the contextual inspector where applicable

#### Scenario: Backend session is not ready
Given no backend session is ready
When the application shell renders
Then navigation remains available for connection and fixture workflows
And mutation controls that require a ready backend are disabled
