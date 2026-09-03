# Tasks

- [x] 1.1 Add a connect-announce worker that announces after the transport connects at start and after every reconnect, with mock-transport tests
- [x] 1.2 Wire the worker into the mobile node and abort it with the other workers
- [x] 1.3 Log an unverified inbound with its reason and request the sender's path, on both the packet and resource paths
- [ ] 1.4 Verify on the phone: connect to the echo peer after a peer restart and receive an echo without a manual announce
