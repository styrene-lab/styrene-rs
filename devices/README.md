# Device Registry

A lightweight, human-maintained inventory of physical devices used for Styrene development and validation.

Each device has one Markdown record under this directory. Records distinguish direct observation, operator report, manufacturer claims, and inference. Unknown fields remain unknown; do not generalize one unit's evidence to an entire model family.

## Devices

| Unit ID | Manufacturer | Model | Role | Evidence state | Record |
|---|---|---|---|---|---|
| `operator-rg35xxsp-a` | ANBERNIC | RG35XXSP | First constrained-communicator target | Physical identity and stock-system baseline observed | [operator-rg35xxsp-a.md](operator-rg35xxsp-a.md) |
| `styrene-builder-a` | Raspberry Pi | Raspberry Pi 4 Model B | Native ARM64 builder; first declarative flasher reference | Hardware validated | [styrene-builder-a.md](styrene-builder-a.md) |

## Record conventions

- **Observed:** directly visible in a photograph, command output, or physical test.
- **Operator-reported:** reported by the operator but not independently captured.
- **Manufacturer-declared:** taken from manufacturer documentation or labeling.
- **Inferred:** plausible interpretation that still needs confirmation.
- Record DHCP addresses as observations, never stable identity.
- Never store passwords, private keys, Wi-Fi credentials, mesh identities, full card images, or unreviewed serial/MAC identifiers here.
- Preserve artifact hashes and concise evidence pointers; keep large/raw evidence outside Git until reviewed.
