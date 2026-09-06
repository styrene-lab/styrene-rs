# External Styrene Identity dependency

Styrene Identity is maintained at https://github.com/styrene-lab/styrene-identity.
The workspace dependency pins commit `7ce44fd8dac29299b88623ca0252e5f5cebcacfc`.
`styrened` retains its optional mobile identity/custody features; `styrene-e2e`
uses the same dependency for its identity contracts. No second workspace copy
is retained.

The Identity repository owns derivation vectors, signer features, backup formats,
package checks and their standalone CI. This workspace owns integration:
identity continuity, mobile recovery, profiles, runtime and protocol behavior.
The monorepo test/release lists no longer claim ownership of Identity unit tests.
The mobile P0 corpus's local owner paths name the runtime adapter; platform
custody implementations now live in the external repository's src directory.
Historical OpenSpec paths and generator provenance describe their original revision.

The pin follows the behavior-preserving extraction from backend 579ee533 and a
manifest fix for SSH dependencies accidentally scoped to Android. Private-key
material, derivation domains and persisted identity formats were not changed.
Git dependency use is deliberate; no crates.io release is required for this gate.

Cutover validation is recorded in the coordination repository's
`docs/identity-extraction-plan.md`. Do not infer platform runtime acceptance from
successful dependency resolution or compile checks. Nix vendoring, Linux/Android,
Apple device and UI revision-pair handoff remain explicit integration lanes.
