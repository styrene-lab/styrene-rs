# Mobile RNode Bluetooth

## Scope

The Android and iOS hosts use Bluetooth Low Energy as the preferred RNode bearer. Android retains USB serial as an explicit fallback. USB attachment does not start or replace an active Bluetooth session.

The mobile hosts target the Nordic UART Service in canonical RNode Firmware 1.86. The source reference is commit [`d39339f8ecd5145b248c18bac7b6ea0f82faf85a`](https://github.com/markqvist/RNode_Firmware/commit/d39339f8ecd5145b248c18bac7b6ea0f82faf85a).

## Firmware Contract

The host uses these GATT identifiers:

| Role | UUID |
|---|---|
| Nordic UART service | `6E400001-B5A3-F393-E0A9-E50E24DCCA9E` |
| Host writes to RNode | `6E400002-B5A3-F393-E0A9-E50E24DCCA9E` |
| RNode notifies host | `6E400003-B5A3-F393-E0A9-E50E24DCCA9E` |

The GATT payload is the unchanged RNode KISS byte stream. A GATT write or notification is not a KISS frame boundary. Each host keeps one KISS decoder across arbitrary notification fragments.

The host uses write-with-response and serializes all writes. It limits each write to the platform-reported GATT limit. This policy supports the ESP32-S3 implementation, which requires write-with-response. It also supports the nRF52 implementation.

## Compatible Hardware

Canonical firmware 1.86 provides this BLE service on supported ESP32-S3 and nRF52840 boards. Examples include Heltec LoRa32 v3 and v4, LilyGO T3S3, T-Deck, T-Beam Supreme, RAK4631, T-Echo, and Heltec T114.

Older ESP32 boards can expose Bluetooth Classic SPP instead of BLE. iOS CoreBluetooth cannot connect to arbitrary SPP devices. Confirm that the selected board and firmware expose the Nordic UART Service before mobile testing.

Use firmware 1.82 or later. Firmware 1.86 is preferred because it includes the current pairing, reconnect, and buffering behavior.

## Connection Policy

The first connection requires an explicit selection in the mobile UI. An unknown advertisement is listed but never connected automatically.

After approval, the host stores the platform peripheral identifier. The host reconnects only to that approved peripheral. The displayed `RNode XXXX` name is not a stable or globally unique identity.

Android stores the bonded `BluetoothDevice` identifier. iOS stores the CoreBluetooth peripheral UUID. App approval remains until the operator selects **Forget**. If the operating-system bond is removed, the operating system can request pairing again for the approved peripheral.

Only one bearer owns the RNode session. The coordinator pauses access to the Rust outbound packet channel until a byte link is active. Android starts USB only after the operator selects **Use USB**.

## Pairing Procedure

1. Install firmware 1.82 or later on a compatible BLE board.
2. Enable Bluetooth in the persistent RNode configuration.
3. Start the Styrene mobile node.
4. Select **Scan** in the Network screen.
5. Put the RNode in pairing mode. Hold its hardware button for more than five seconds.
6. Select the advertised RNode in Styrene.
7. Enter the six-digit RNode PIN when the operating system requests it.
8. Wait for the first pairing connection to close. The app reconnects with the stored bond.
9. Confirm that the Network screen reports `RNode <version> online over Bluetooth`.

The firmware pairing window is 35 seconds. If pairing expires, start pairing mode and scan again.

## Radio Profile

The current mobile proof profile is named `US_915_DEVELOPMENT`. It configures `915 MHz`, `125 kHz`, `17 dBm`, `SF7`, and coding rate `5`.

This profile is not a universal default. The operator must confirm that the frequency and transmit power are legal at the test location. A production release must provide an explicit regional radio-profile selection before it enables transmission.

## Evidence Limits

Automated tests cover KISS fragmentation and bounds, byte-order configuration,
packet pumping, outbound retention, and host lifecycle state. Local validation
also builds both applications and cold-launches them in a simulator or emulator.
These checks do not prove BLE behavior.

A physical acceptance record must include:

- mobile device model and operating-system version
- RNode board, firmware version, and advertised name
- observed Nordic UART service and characteristic properties
- pairing and reconnect result
- negotiated write limit or MTU
- successful RNode detection and configuration response
- bidirectional packet counts and message correlation
- disconnect and reconnect behavior
- selected radio profile and test jurisdiction

Do not claim physical BLE acceptance from a successful build, simulator run, advertisement, or GATT connection alone.

An operator-observed iOS run reached approved-peripheral reconnect, RNode
detection, radio configuration, and packet transmission. The run does not meet
the complete acceptance-record requirements above. Physical iOS and Android BLE
acceptance therefore remain open.
