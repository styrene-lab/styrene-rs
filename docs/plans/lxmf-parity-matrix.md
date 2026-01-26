# LXMF Parity Matrix

Status legend: missing | partial | done

| Python Module | Rust Module | Status | Tests | Notes |
| --- | --- | --- | --- | --- |
| LXMF/LXMF.py | src/constants.rs + src/helpers.rs | missing | tests/constants_parity.rs | constants/helpers |
| LXMF/LXMessage.py | src/message/* | missing | tests/payload_parity.rs, tests/wire_parity.rs | payload + wire |
| LXMF/LXMPeer.py | src/peer/mod.rs | missing | tests/peer_parity.rs | peer tracking |
| LXMF/LXMRouter.py | src/router/mod.rs | missing | tests/router_parity.rs | router |
| LXMF/Handlers.py | src/handlers.rs | missing | tests/handlers_parity.rs | handlers |
| LXMF/LXStamper.py | src/stamper.rs | missing | tests/stamper_parity.rs | stamps |
| LXMF/Utilities/lxmd.py | src/bin/lxmd.rs | missing | tests/lxmd_cli.rs | daemon/cli |
