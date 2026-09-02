# Embedded Time - Delta Spec

## ADDED Requirements

### Requirement: No-std protocol time is explicit and non-panicking

The protocol core must compile without `std`. Timestamp-dependent announce and ratchet operations
must use one embedding-controlled Unix-time source and return a typed unavailable-time error until
that source is initialized.

#### Scenario: No-std core is compiled
Given default and standard-library features are disabled
When the owning RNS core package is checked for its supported embedded target contract
Then the core compiles without unconditional `std` imports or a required async runtime
And transport-only dependencies remain excluded

#### Scenario: Embedded time is unavailable
Given a no-std embedding has not supplied Unix time
When it requests announce creation or time-dependent ratchet rotation
Then the operation returns the typed unavailable-time error
And it does not panic, underflow a boot offset, or emit an epoch timestamp

#### Scenario: Embedded time is supplied
Given a no-std embedding supplies a current Unix timestamp and refreshes it as time advances
When it creates announces and evaluates ratchet rotation
Then announce random blobs contain the canonical five-byte timestamp suffix
And ratchet age uses the same supplied time source
