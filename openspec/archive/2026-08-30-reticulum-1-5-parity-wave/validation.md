# Reticulum 1.5 Parity Wave Validation

Validated on 2026-08-30 against Reticulum authorities:

- `rns-1.4.2`: `b48b96e61676504e0a4e527b33b9a0b4495c6872`
- `rns-1.5.1`: `149e4151095adf098b8f53eab0c03b37169e8559`

## Offline Evidence

The following commands completed successfully:

- `just validate`
- `cargo test -p styrene-rns --features transport,interop-tests`
- `cargo test -p styrene-interop-runner --test rns_fixtures --test rns_handoff_manifests`
- `cargo clippy -p styrene-rns -p styrene-interop-runner --all-targets --features styrene-rns/transport,styrene-rns/interop-tests -- -D warnings`
- `cargo check --workspace --all-targets --exclude styrene-dx`
- `python3 scripts/test_validate_fixture_provenance.py`
- `python3 scripts/validate_fixture_provenance.py`
- `python3 scripts/test_validate_product_capabilities.py`
- `python3 scripts/test_parity_claim_labels.py`
- `python3 scripts/validate_product_capabilities.py`
- OpenSpec validation for this wave and the Beechat, FreeTAK, and Leviculum consumer waves

The shared loader verified both authority records and every indexed artifact checksum. Legacy
1.4.2 fixture tests and the three consumer registrations resolve through the same v2 loader.
The offline policy test rejects network or Python execution, hardware defaults, mutable revisions,
competing RNS authority schemas, missing future artifact checksum requirements, live registration,
and support-claim promotion.

## Live Handoff

No live Python, routed network, serial hardware, or physical-device scenario was executed. The
routed request/resource, mixed-interface MTU, and discovery scenarios are bounded handoff-only
records in `tests/interop/handoffs/reticulum-1.5.1-live.json`.

Registration, execution, and retained live evidence remain owned by
`reticulum-lxmf-nomadnet-parity` Tasks 4.7, 5.7, 8.8, and 12.6. The handoff scenarios are not in
`PinnedScenarioId`, `PINNED_SCENARIOS`, the product parity-gate registry, or the live workflow. No
live pass result or support claim is asserted by this wave.
