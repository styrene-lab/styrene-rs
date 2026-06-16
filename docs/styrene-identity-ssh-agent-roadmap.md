# Styrene Identity SSH Agent Roadmap

Status: design / implementation roadmap  
Owner: Styrene identity + Nex profile integration

## Context

Nex profiles need to materialize SSH client configuration such as:

```toml
[ssh]
canonical_domains = ["vanderlyn.local"]

[[ssh.hosts]]
pattern = "github.com"
user = "git"
identity_files = ["~/.ssh/security-key", "~/.ssh/personal-github"]
identities_only = true

[[ssh.hosts]]
pattern = "git.styrene.io"
user = "git"
identity_label = "styrene-git"
identities_only = true
```

Literal `identity_files` can be rendered directly into `~/.ssh/config.d/nex.conf`.
`identity_label` is different: it refers to a key derived from the Styrene root identity.
OpenSSH cannot authenticate with only a public key string; it needs either a private key file
or an SSH agent that can sign challenges.

The long-term design should therefore be:

```text
OpenSSH
  IdentityAgent ~/.styrene/ssh-agent.sock
      ↓
Styrene SSH agent
      ↓
~/.config/styrene/identity.key
      ↓
HKDF derive_ssh_user_key(label) → sign challenge in memory → zeroize
```

## Current implementation evidence

`styrene-identity` already has the important primitives:

- `KeyDeriver::derive_ssh_user_key(label)` derives per-label SSH user Ed25519 seeds.
- `KeyDeriver::derive_agent_key(agent_name)` derives per-agent signing keys in a separate HKDF family.
- `format::ssh_pubkey(seed, comment)` exports OpenSSH public keys.
- `format::ssh_pubkey_fingerprint(seed)` emits OpenSSH-compatible fingerprints.
- `ssh_agent.rs` already implements `StyreneAgent` behind the `ssh-agent` feature.
- `StyreneAgent` serves:
  - SSH user keys: `styrene-ssh-user-{label}`
  - git signing key: `styrene-git-signing`
  - agent signing keys: `styrene-agent:{name}`
  - optional SSH host key: `styrene-ssh-host`
- `StyreneAgent::sign()` derives only the requested private seed, signs, then zeroizes the seed.

This means the missing work is not the cryptographic core. The missing work is daemonization,
socket lifecycle, profile/Nex integration, and operator UX.

## Design principles

1. **Do not export derived private SSH keys by default.**
   Private key files under `~/.ssh/styrene/<label>` are easy to use but weaken the identity model:
   stealing the exported file can grant SSH access without the root identity/passphrase.

2. **Use an SSH agent socket as the default private-key boundary.**
   OpenSSH gets public keys and signatures; root-derived private seeds stay in process memory only.

3. **Keep profile materialization separate from identity serving.**
   Nex profile application should render SSH host routing config. Styrene identity should serve keys.
   The bridge is `identity_label` + `IdentityAgent`.

4. **Key labels are public routing names, not secrets.**
   Labels such as `github`, `work`, or `styrene-git` appear in profile config and public key comments.
   They must be validated but do not need secrecy.

5. **Agent keys and SSH user keys remain domain-separated.**
   `derive_agent_key("github")` and `derive_ssh_user_key("github")` intentionally produce different keys.

## Target operator UX

### Start or install the agent

```sh
styrene identity agent start \
  --identity ~/.config/styrene/identity.key \
  --socket ~/.styrene/ssh-agent.sock \
  --ssh-label github \
  --ssh-label styrene-git \
  --git-signing
```

Long-running workstation install:

```sh
styrene identity agent install --user
styrene identity agent status
styrene identity agent stop
```

On macOS this should install a LaunchAgent. On Linux this should install a systemd user unit.

### Inspect public keys

```sh
styrene identity agent list-keys
```

Example output:

```text
styrene-ssh-user-github      SHA256:...  ssh-ed25519 AAAA...
styrene-ssh-user-styrene-git SHA256:...  ssh-ed25519 AAAA...
styrene-git-signing          SHA256:...  ssh-ed25519 AAAA...
```

### Nex profile materialization

A host entry with `identity_label` should render as:

```sshconfig
Host git.styrene.io
    User git
    IdentityAgent ~/.styrene/ssh-agent.sock
    IdentitiesOnly yes
    # StyreneIdentity label: styrene-git
    # Public key: run `nex identity ssh styrene-git`
```

If OpenSSH requires narrower key selection than agent-wide identity listing can provide,
Nex may also render a public identity hint once a supported OpenSSH mechanism is chosen.
Until then, the agent should only serve labels explicitly configured for this machine.

## Component split

### `styrene-identity` crate

Already present:

- HKDF derivation families
- public key formatting
- `StyreneAgent` session implementation

Needed:

- Stable public API for constructing an SSH agent from config.
- Label validation that rejects empty labels and control characters.
- Optional public-key inventory helper returning label/comment/fingerprint/public-key records.
- Tests for invalid labels and duplicate labels.

### `styrene` CLI / daemon side

Needed:

- CLI commands for running the agent in foreground/background.
- Unix socket creation with safe permissions.
- macOS LaunchAgent generation.
- Linux systemd user unit generation.
- Status reporting: socket path, configured labels, identity hash, key fingerprints.
- Passphrase handling policy.

Passphrase policy options:

1. Prompt on foreground start.
2. Use OS keychain integration where available.
3. Refuse unattended startup unless an explicit secure provider is configured.

### Nex profile integration

Needed in Nex, not `styrene-rs`:

- Add top-level profile `ssh` schema.
- Render `~/.ssh/config.d/nex.conf` and idempotently include it from `~/.ssh/config`.
- Support literal `identity_files` immediately.
- Support `identity_label` by rendering `IdentityAgent` and comments, and by warning if the Styrene agent is not running.
- Do not export private keys as part of `nex profile apply`.

## Security model

### Secrets at rest

- Root identity remains encrypted in `~/.config/styrene/identity.key`.
- Derived SSH private seeds are never written to disk by default.
- Any optional export mode must be explicit and documented as a downgrade.

### Secrets in memory

- The agent may need root-equivalent material while unlocked.
- Derived per-key seeds must be scoped to one signing operation and zeroized immediately.
- Existing `StyreneAgent::sign()` already follows the derive-sign-zeroize pattern.

### Socket security

- Socket directory should be `0700`.
- Socket should be owned by the current user.
- Refuse to bind through symlinks or world-writable parent directories.
- Replace stale sockets only after confirming they are sockets owned by the current user.

### Config injection

Nex SSH config rendering must reject newline/control characters in:

- host patterns
- users
- hostnames
- identity paths
- labels
- option keys and values

Nex should forbid command-execution SSH options in profile v1 unless explicitly designed:

- `ProxyCommand`
- `LocalCommand`
- `PermitLocalCommand`
- `Match exec`

`ProxyJump` is safer and should be preferred.

## Open questions

1. How should OpenSSH be constrained to choose a specific Styrene label for a host?
   - Simplest v1: only serve labels configured for this machine/session.
   - Better v2: agent-side host constraints or per-host agent sockets.

2. Where should the unlocked root secret live?
   - In the agent process memory only.
   - Potentially in OS keychain-backed signer implementations later.

3. Should the agent support confirmation prompts per signature?
   - Useful for high-value labels.
   - Requires UI/TTY integration and a non-interactive policy for headless nodes.

4. Should host keys be served from the same agent?
   - `StyreneAgent::with_host_key()` exists.
   - Server-side OpenSSH host-key integration is a separate design from client auth.

5. What is the stable socket path?
   - Proposed: `~/.styrene/ssh-agent.sock`.
   - Needs XDG/runtime-dir consideration on Linux.

## Implementation phases

### Phase 1 — Documented bridge and inventory

- Add key inventory helper in `styrene-identity`.
- Add CLI command to list agent-served public keys/fingerprints.
- Keep private-key export out of scope.

### Phase 2 — Foreground SSH agent

- Provide `styrene identity agent start --foreground`.
- Bind Unix socket safely.
- Serve configured SSH labels using existing `StyreneAgent`.
- Add integration tests using `ssh-agent-lib` protocol requests.

### Phase 3 — Workstation service install

- Add macOS LaunchAgent support.
- Add Linux systemd user support.
- Add status/stop commands.
- Define passphrase/keychain behavior.

### Phase 4 — Nex profile bridge

- Nex renders `IdentityAgent ~/.styrene/ssh-agent.sock` for `identity_label` hosts.
- Nex warns when configured labels are not served by the running agent.
- Nex supports dry-run output showing label → fingerprint.

### Phase 5 — Advanced constraints

- Per-label confirmation policy.
- Per-host or constrained keys.
- Optional explicit private-key export mode, if still needed, with strong warnings.

## Non-goals for the first implementation

- Replacing the platform `ssh-agent` globally.
- Exporting root-derived private keys to `~/.ssh` by default.
- Rewriting all user SSH config.
- Server-side SSH host key provisioning.
- Remote fleet profile application over SSH; this roadmap is only local SSH agent integration.
