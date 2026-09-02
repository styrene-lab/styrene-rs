# parity-evidence Delta

## ADDED Requirements

### Requirement: Reticulum 1.5.2 maintenance evidence has immutable provenance

The shared RNS fixture index must identify the canonical repository, full Reticulum 1.5.2 revision, release, generator, source symbols, typed expected outcome, artifact path, and SHA-256 for empty-carrier evidence.

#### Scenario: Empty-carrier evidence is validated

Given the committed `rns-1.5.2-empty-carrier-input` vector
When fixture validation runs offline
Then its authority is commit `ea98db4f53dcf0defc0e71a16e60d28b1229c4e6`
And its checksum and source symbols match the retained artifact
And all earlier authority records and vector metadata remain available
