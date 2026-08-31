# RNode Firmware Provisioning Contract Corpus

These fixtures define firmware policy before product implementation. They do
not contain firmware, device secrets, stable USB serial numbers, BLE peripheral
identifiers, or physical acceptance evidence.

All target observations and digests are synthetic unless a field explicitly
identifies an immutable public upstream revision. Passing these fixtures proves
only policy and state-machine conformance. It does not enable a hardware support
claim.

Run the validator and mutation tests with:

```bash
python3 scripts/validate_rnode_firmware_corpus.py
python3 scripts/test_validate_rnode_firmware_corpus.py
```

Product implementations must consume these same cases. Add or change a corpus
case before changing capability, artifact-admission, destructive-operation, or
recovery behavior.
