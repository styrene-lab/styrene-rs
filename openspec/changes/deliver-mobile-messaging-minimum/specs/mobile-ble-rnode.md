# Mobile BLE RNode - Delta Spec

## ADDED Requirements

### Requirement: BLE connection requires explicit peripheral approval

The mobile application must connect only to an explicitly selected and approved
platform peripheral identifier. An advertised name is display metadata and must
not become persistent identity.

#### Scenario: Unknown RNode advertises
Given BLE discovery reports an unknown peripheral with an RNode display name.
When the discovery result enters the mobile session.
Then the application lists the peripheral without connecting.
And backend Bluetooth RNode state does not become connecting or connected.

#### Scenario: User approves a discovered RNode
Given discovery reports a peripheral that exposes the expected Nordic UART Service.
When the user explicitly selects that peripheral.
Then the application stores its platform peripheral identifier as approved.
And only that approved identifier becomes eligible for connection and reconnect.

#### Scenario: User forgets the approved RNode
Given an approved peripheral identifier is stored.
When the user selects Forget.
Then the application closes its active attempt and removes application approval.
And later advertisements do not reconnect until another explicit approval.

### Requirement: BLE uses the canonical Nordic UART byte contract

The platform adapter must require service `6E400001-B5A3-F393-E0A9-E50E24DCCA9E`,
host-write characteristic `6E400002-B5A3-F393-E0A9-E50E24DCCA9E`, and
host-notify characteristic `6E400003-B5A3-F393-E0A9-E50E24DCCA9E`. It must use
write-with-response, subscribe to notifications, and preserve the unchanged RNode
KISS byte stream across arbitrary GATT boundaries.

#### Scenario: Required NUS property is absent
Given the approved peripheral omits a required UUID or characteristic property.
When service discovery completes.
Then the platform adapter closes the attempt with a typed incompatible-device failure.
And backend Bluetooth RNode state does not become connected.

#### Scenario: Notifications fragment KISS traffic
Given the approved NUS peripheral sends one KISS stream across arbitrary notifications.
When the ordered bytes enter the shared RNode engine.
Then complete frames decode in byte order without treating notifications as frame boundaries.
And incomplete data remains bounded until a later notification completes or invalidates it.

#### Scenario: Host writes exceed one GATT operation
Given the platform reports a safe write size smaller than an encoded KISS frame.
When the shared RNode engine emits that frame.
Then the host sends serialized response writes no larger than the reported limit.
And the next write begins only after the prior response completes.

### Requirement: The backend owns BLE RNode readiness and bearer truth

The mobile byte session must identify its active bearer. Bluetooth becomes connected
only after the shared RNode engine validates exact radio configuration readback.
Bluetooth and Android USB state changes must remain independent.

#### Scenario: Approved BLE attempt starts
Given an approved Bluetooth peripheral and no active RNode bearer.
When the host starts a BLE ordered-byte attempt.
Then backend Bluetooth RNode state becomes connecting.
And Android USB state does not change.

#### Scenario: BLE GATT connects without RNode readback
Given an approved BLE link has valid NUS characteristics.
When RNode configuration readback is absent or mismatched.
Then backend Bluetooth RNode state does not become connected.
And payload transmission remains gated.

#### Scenario: BLE readback completes
Given an approved BLE attempt is connecting.
When the shared RNode engine validates every configured radio value.
Then backend Bluetooth RNode state becomes connected.
And outbound RNS packets become eligible for KISS framing.

#### Scenario: USB fallback is requested during active BLE
Given approved Bluetooth is connecting, connected, or reconnecting.
When an Android user requests USB fallback.
Then the backend rejects the fallback as Bluetooth active.
And the active Bluetooth attempt remains the only RNode bearer owner.

### Requirement: BLE attempt lifecycle is bounded and generation safe

Discovery, connection, notification, write, and disconnect callbacks must carry an
attempt generation. Late callbacks must not replace the current attempt. Closing an
attempt must be idempotent and leave a bounded reconnect opportunity.

#### Scenario: Stale callback arrives after reconnect
Given a newer BLE attempt generation is active.
When an older generation reports connection, bytes, write completion, or disconnect.
Then the mobile session ignores the stale callback.
And current bearer state and byte ownership remain unchanged.

#### Scenario: Active BLE connection is interrupted
Given the BLE RNode bearer is connected.
When the platform reports a connection interruption.
Then the session closes the attempt and reports Bluetooth disconnected or reconnecting.
And it preserves unaffected TCP messaging state.

#### Scenario: Application returns to foreground
Given an approved peripheral was interrupted while the application was suspended.
When the application receives a current foreground opportunity.
Then it starts at most one bounded reconnect attempt for that approved identifier.
And it does not claim guaranteed background execution.

### Requirement: BLE handoff retains outbound data until terminal write evidence

Removing an outbound packet from the backend channel must not silently lose it when
the BLE attempt fails before all response writes complete. Retention and replay must
be bounded and must not duplicate an acknowledged packet.

#### Scenario: BLE write fails before completion
Given an outbound packet has entered the active BLE handoff.
When a response write fails before the complete KISS frame is accepted.
Then the packet remains eligible for bounded replay after reconnect.
And backend state does not report remote reception or delivery.

#### Scenario: Retained packet succeeds after reconnect
Given one outbound packet was retained after an interrupted BLE write.
When the approved peripheral reconnects and accepts the complete frame.
Then the session removes that retained packet once.
And repeated reconnect processing does not send another retained copy.

### Requirement: BLE support claims require physical bidirectional evidence

A build, permission grant, advertisement, GATT connection, or outbound API
acceptance must not establish BLE RNode support. Each platform claim requires the
complete physical acceptance record.

#### Scenario: GATT connection succeeds without packet correlation
Given a release candidate connects to the expected NUS characteristics.
When no bidirectional packet and message correlation completes.
Then BLE RNode support remains unverified.
And the evidence reports only the completed connection observations.

#### Scenario: Physical BLE acceptance completes
Given the release candidate uses an identified mobile device, RNode board, firmware, and legal test profile.
When bidirectional correlation, interruption, retained replay, and approved-device reconnect complete.
Then the evidence records UUID properties, safe write size, packet counts, revisions, OS, radio profile, jurisdiction, and outcomes.
And support applies only to the exercised platform, board, firmware class, and scenarios.
