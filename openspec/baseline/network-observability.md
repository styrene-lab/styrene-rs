# network-observability - Baseline

### Requirement: Network views preserve protocol truth

Discovery observations, routes, links, interfaces, and non-routing associations must be distinct data and visual concepts.

#### Scenario: Peer is discovered without a route
Given a valid announce was accepted for a peer
And no current path-table entry or active link exists
When the network graph renders that peer
Then it displays a discovery observation
And does not display a direct route or active-link edge

#### Scenario: Routed peer is displayed
Given the daemon reports a route with next hop and hop count
When the Routes or Combined view renders
Then the edge identifies route semantics, freshness, and hop information
And remains visually distinct from discovery and link edges

#### Scenario: Network cardinality is high
Given a fixture contains at least 500 peers and sustained incremental events
When the operator searches, filters, selects, or navigates
Then interactions remain within the recorded responsiveness budget
And unchanged topology is not rebuilt for counter-only updates

### Requirement: Network observations expose provenance and freshness

Every displayed discovery observation, route, link, and interface state must identify its source, observation time, connection generation, and freshness.

#### Scenario: Observation becomes stale
Given a network observation exceeds its domain freshness threshold
When the Network page renders it
Then the observation is visibly stale
And is not presented as current solely because the entity remains cached

#### Scenario: Selected network entity is inspected
Given the operator selects a peer, route, link, or interface
When the contextual inspector opens
Then it identifies the observation source and age
And offers only actions supported by current capabilities and entity state
