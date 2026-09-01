# styrene-rnode-firmware

This crate owns transport-neutral RNode firmware policy and operation records.
It must not open devices, invoke tools, implement BLE or serial transports, or
depend on renderer crates.

Committed corpuses under `tests/fixtures/rnode-firmware-provisioning-v1` define
capability, artifact, and workflow behavior. Change corpus cases before changing
policy behavior.

Target selection is exact and deny-by-default. A completed write is not success.
Success requires authoritative model, firmware version, and application-hash
verification.

Run:

```bash
cargo test -p styrene-rnode-firmware
cargo clippy -p styrene-rnode-firmware --all-targets -- -D warnings
```
