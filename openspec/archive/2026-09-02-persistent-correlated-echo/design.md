# Design

The existing `AutoReplyService` remains the response policy owner and gains echo semantics; persisted settings remain in `DaemonConfig`, while `ConfigService` performs synchronized mutation and disk persistence. The canonical service inbound worker invokes policy only after the existing durable acceptance and trust gate for both packet and resource paths. The legacy daemon remains an event projection adapter, not an inbound writer.

Echo responses use the normal `MessagingService` outbound lifecycle with structured LXMF fields. The inbound `source` is interpreted only as an LXMF delivery destination and must decode to exactly 16 bytes; it is never rehashed. A `styrene_echo.response = true` marker prevents loops and `request_id` correlates the response to the accepted inbound message ID.

Direct fallback occurs inside the existing persisted attempt after direct dispatch fails. `RouterCoordinator` atomically changes only actual method, representation, and fallback reason, preserving requested method, correlation, attempt count, and deadline. Messaging then sends destination-stripped canonical wire through `send_raw`; packet size eligibility is decided before fallback.
