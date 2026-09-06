# Extract NomadNet services

Create an internal workspace `styrene-nomadnet` library for native content and
browsing behavior. First extract pure Micron projection, native form encoding and
binary-response decoding with daemon adapters and existing behavioral evidence.
Then move the coordinator behind domain-owned transport/discovery contracts.

Keep RNS route recovery in styrene-rns, Micron parsing in styrene-micron, and
identity selection, authorization, IPC dispatch and runtime wiring in styrened.
No repository split, publication, plugin ABI, wire change or runtime deployment.
Cache expiry and recovery fixes follow the behavior-preserving extraction.
