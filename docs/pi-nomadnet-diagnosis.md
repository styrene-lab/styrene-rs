# Pi NomadNet direct-path diagnosis

On 2026-09-06 a Mac client connected only to the Pi at `192.168.0.74:4242`
received its LXMF announcement but could not browse native destination
`1d759695fd70ca5f66bf436c8f01b548`. The Pi serves the coordination-owned
`pi-nomadnet-v1` static corpus. Its installed daemon remains unchanged.

## Announcement admission

A diagnostic-only client build recorded the native announcement as validated but
held by the interface limiter for 360 seconds. The default limiter estimated
frequency from the first two tightly spaced announcements. A normal service pair
therefore triggered the burst hold (60 seconds) and release penalty (300 seconds).
The failed regression reproduced `Hold(360s)` on the second announcement.

The limiter now collects its bounded six-sample window before classifying a burst.
It admits at most the first five unknown announcements during sampling; the sixth
is subject to the existing frequency threshold. The calculation counts intervals
rather than timestamps and treats a full simultaneous window as a burst instead
of zero frequency. Thresholds, queue bounds, penalties and release ordering remain.
The existing limiter tests use a two-sample test configuration to exercise hold,
eviction and release promptly; new default-policy tests cover initial service
announcements, a full burst, simultaneous timestamps and sustained quiet traffic.
Hold decisions are visible through best-effort transport diagnostics.

With this fix the same client learned the native destination and fetched the
index, formatting and second pages in 80, 80 and 118 milliseconds respectively.
These are individual observations, not a performance distribution. All returned
source matches the deployed fixtures. This does not explain every public-host
failure or eliminate intentional throttling during large announcement bursts.

## Native file response

The deployed server and current server both encode native file responses as
`[filename, binary]`. The downloader decoded only bare binary, yielding
`native file response was malformed` after successful request transport.

A dedicated decoder now accepts the exact pair and retains legacy bare-binary
compatibility. Header/length validation is bounded and rejects malformed UTF-8,
truncation, unexpected containers and extra values. The supplied filename never
chooses a local save path. The split integration fixture now supplies the actual
server response shape rather than a page-style binary response.

The direct Pi download subsequently completed with integrity verification and
SHA-256 `ea75cd8e670946304cf70d6ea6472b7f17b7e83fe29530f1a94e63123c8d12fe`.

## Validation

- RNS transport/interop lane: 376 tests passed.
- NomadNet domain: 46 tests passed.
- Daemon split integration: 5 tests passed, including the file pair.
- Warning-denied Clippy passed for RNS transport and NomadNet/daemon libraries/tests.
- Formatting passed.

Raw live baseline/fixed evidence belongs to the coordination checkout under
`lab/runs/pi-nomadnet-diagnosis/`. No protocol encoding, Pi daemon binary, radio
configuration, persistent identity or public hub deployment was changed.
