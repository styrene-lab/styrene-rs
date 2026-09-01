# Mobile Platform Workflows - Delta Spec

## ADDED Requirements

### Requirement: Public identity management is complete and custody-safe

The mobile product must expose public identity metadata, public destination
sharing, and encrypted backup and restore without exposing private key material
to presentation state or platform logs.

#### Scenario: User renames the identity
Given the mobile identity is available under protected custody
When the user saves a valid display name
Then the backend persists the normalized public metadata
And the identity hash and private custody backend remain unchanged

#### Scenario: User shares the public destination
Given the backend exposes the current public LXMF delivery destination
When the user requests Copy or Show QR
Then the platform workflow exposes only that public destination
And no private identity material enters the clipboard, QR payload, or accessibility tree

#### Scenario: User creates an encrypted backup
Given the active custody backend permits an encrypted export
When the user completes backup with valid protection input
Then the backend returns one authenticated opaque backup artifact to the file or share service
And diagnostics and presentation state contain no key bytes or protection input

#### Scenario: User restores an invalid backup
Given the user selects a corrupt, incompatible, or incorrectly protected backup
When the backend attempts restoration
Then it returns a typed non-destructive failure
And the active identity and durable messages remain unchanged

### Requirement: Destination ingress uses typed platform services

Clipboard paste and QR scanning must pass one bounded candidate destination to
backend validation and must not create product state from unvalidated input.

#### Scenario: User pastes a destination
Given the clipboard service returns text within the accepted bound
When the user chooses Paste in New Message
Then the application submits the candidate to backend destination validation
And it creates no conversation before validation succeeds

#### Scenario: Camera permission is denied
Given QR scanning requires camera permission and the operating system denies it
When the user chooses Scan QR
Then the application reports the typed denial and an available recovery action
And manual entry, paste, and unrelated messaging remain usable

#### Scenario: Scanner returns an oversized payload
Given the QR service decodes a payload beyond the accepted destination bound
When the result reaches the platform-service boundary
Then the service rejects it before backend mutation
And raw frame or payload content is not logged

#### Scenario: User captures one QR image
Given the packaged application can invoke the operating-system camera or image picker
When the user chooses one JPEG or PNG image containing one QR symbol
Then the Rust decoder returns one generation-tagged bounded candidate
And the application submits that candidate through the same backend validation path as manual entry and paste

#### Scenario: Captured image exceeds a resource bound
Given a selected image exceeds the compressed-byte, dimension, or decoded-pixel limit
When the image reaches the QR decoder boundary
Then decoding stops with a typed oversized failure
And no image buffer, decoded payload, contact, or conversation is retained

#### Scenario: Captured image contains no unambiguous candidate
Given a selected image contains no QR symbol or more than one decodable QR symbol
When the Rust decoder examines the image
Then it returns a typed no-code or ambiguous failure
And the existing destination field and unrelated workflows remain unchanged

#### Scenario: Scan completion belongs to an old generation
Given a camera or image-picker request remains open while the mobile session generation changes
When the old request completes
Then the application rejects the completion as stale
And the completion cannot replace a newer candidate or failure state

#### Scenario: User cancels image capture
Given a QR capture request is active
When the user cancels the operating-system camera or image picker
Then the platform service returns a typed cancelled outcome
And manual entry and paste remain available without an error claim

#### Scenario: Scan diagnostics remain payload-free
Given QR capture or decoding fails at any supported stage
When diagnostics, fixtures, and generic debug output are inspected
Then they contain only stable failure codes and bounded metadata
And they contain no encoded image bytes, grayscale frame, or decoded payload text

### Requirement: Protected capability state is visible and isolated

The mobile product must present typed availability, authorization, restriction,
and failure state for each protected capability without conflating permissions
or disabling unrelated workflows.

#### Scenario: User inspects permissions
Given camera, Bluetooth, notifications, and secure storage have different current states
When the user opens the relevant settings surface
Then each capability shows its own typed state and disabled reason
And no unsupported location-sharing permission is requested

#### Scenario: Platform supports opening application settings
Given a denied capability can be changed in system settings
When the user invokes its recovery action
Then a Rust platform service requests the supported settings destination
And product navigation does not claim the permission changed before resume requery

### Requirement: Propagation scheduling and disclosure match mobile lifecycle

Automatic propagation synchronization must use only bounded connection,
reconnection, foreground, or platform-granted background opportunities and must
disclose its actual scheduling and airtime behavior.

#### Scenario: No lifecycle opportunity occurs
Given automatic synchronization is enabled and the session state does not change
When a periodic wall-clock interval elapses
Then the mobile client does not start synchronization from that interval alone
And the last trigger source remains unchanged

#### Scenario: Foreground opportunity occurs
Given automatic synchronization is enabled, a ready node is selected, and no sync is in flight
When the platform reports an allowed foreground opportunity
Then the backend schedules one bounded synchronization under cooldown
And the UI reports the trigger source and resulting progress or failure

#### Scenario: User inspects propagation settings
Given the platform cannot guarantee background execution
When the propagation settings render
Then they disclose manual airtime cost and best-effort system scheduling
And they do not claim guaranteed automatic or background collection
