# Reticulum 1.5.2 Empty Carrier Parity Tasks

## 1. Evidence

- [x] 1.1 Pin the Reticulum 1.5.2 authority and generate bounded empty-carrier evidence.
- [x] 1.2 Add the authority and checksummed vector to the shared RNS fixture index without changing earlier records.

## 2. Implementation

- [x] 2.1 Ignore empty UDP datagrams before IFAC and packet admission.
- [x] 2.2 Ignore empty decoded HDLC frames before IFAC and packet admission.
- [x] 2.3 Preserve non-empty malformed-frame accounting and delivery after ignored input.

## 3. Validation

- [x] 3.1 Run focused default and transport tests.
- [x] 3.2 Run warning-denied Clippy, fixture provenance, formatting, and diff checks.
- [x] 3.3 Validate this OpenSpec change.
