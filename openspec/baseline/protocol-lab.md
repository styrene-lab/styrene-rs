# protocol-lab - Baseline

### Requirement: Lab scenarios reuse canonical harness behavior

Protocol Lab must execute scenarios through the pinned interoperability harness or its shared runner boundary, preserving topology, deadlines, process supervision, revision evidence, assertions, and cleanup.

#### Scenario: Pinned scenario succeeds
Given a declared scenario and required implementations are available
When every canonical milestone and assertion completes before the deadline
Then Lab displays the harness success outcome
And links retained evidence and exact source revisions

#### Scenario: Scenario fails
Given a required milestone, assertion, process, or dependency fails
When the harness finalizes the run
Then Lab displays the harness failure class and missing milestone
And does not reinterpret the run as successful

#### Scenario: Scenario is cancelled
Given Lab owns a running scenario
When the operator cancels or resets it
Then every owned process is supervised through termination
And selected diagnostics are retained before temporary state is removed

#### Scenario: Operate mode is active
Given the console is in Operate mode
When pages render ordinary controls
Then fault injection and undeclared scenario mutation controls are unavailable

### Requirement: Scenario execution is isolated from the desktop process

Live scenario process supervision and Python reference execution must occur behind a structured runner boundary rather than inside Dioxus component tasks.

#### Scenario: Scenario runner exits unexpectedly
Given Lab started a live scenario through the runner boundary
When the runner exits before producing a terminal report
Then Lab reports a runner failure
And the desktop event loop remains responsive

#### Scenario: Fixture scenario runs
Given a fixture-only scenario requires no external implementations
When Lab starts the scenario
Then it runs through the same scenario event and result contract
And does not start Python or external network processes
