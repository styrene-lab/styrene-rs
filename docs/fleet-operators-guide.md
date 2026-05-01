# Fleet Operators Guide

> Manage remote nodes over the Styrene mesh: provision, configure, monitor, and maintain.

## Overview

Fleet operations span two CLIs that work together:

| Tool | Role | Runs on |
|------|------|---------|
| **nex** | Cold-start lifecycle: identity, profiles, provisioning | Operator workstation |
| **styrene** | Runtime mesh operations: fleet commands, tunnels, messaging | Operator workstation (CLI) or node (daemon) |

The operator workflow:

```
 Operator workstation                          Remote node
 ──────────────────                          ───────────
 nex identity init          ─── enrollment ──→  styrened (daemon)
 nex profile sign           ─── mesh LXMF ──→  nex profile verify
 styrene fleet apply        ─── mesh LXMF ──→  nex profile apply
 styrene fleet exec         ─── mesh LXMF ──→  command execution
 styrene fleet status       ─── mesh LXMF ──→  status report
```

## Prerequisites

### Operator workstation

```bash
# Install nex
curl -fsSL https://nex.styrene.io/install.sh | sh

# Create your identity (one-time)
nex identity init

# Verify it
nex identity show
```

### Remote nodes

Each managed node runs `styrened` (the mesh daemon) and has `nex` installed. Nodes discover each other via mesh announces and communicate over LXMF.

```bash
# On the node (or baked into the image)
styrened --config /etc/styrene/node.toml --db /data/styrene.db
```

### Connectivity

The operator's workstation connects to a mesh hub (or directly to nodes) via the styrene daemon's IPC socket:

```bash
# Local daemon
styrene status

# Remote daemon via SSH tunnel
ssh -L /tmp/styrene-remote.sock:/run/styrene/daemon.sock user@remote-host
styrene --socket /tmp/styrene-remote.sock status
```

---

## Identity

All fleet operations are authenticated by Styrene identity — a deterministic key hierarchy where one root secret derives all cryptographic keys.

### Create an identity

```bash
nex identity init
```

Generates `~/.config/styrene/identity.key` (argon2id + ChaCha20Poly1305 encrypted). You'll be prompted for a passphrase.

### View your identity

```bash
nex identity show
```

Output:

```
  identity hash   a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4
  signing key     ssh-ed25519 AAAA... styrene
  SSH host key    ssh-ed25519 AAAA... styrene-host
  age recipient   age1...
```

The **identity hash** (SHA-256 of your Ed25519 signing pubkey, truncated to 16 bytes) is your canonical identifier across the mesh.

### Derived keys

| Purpose | Command | Format |
|---------|---------|--------|
| SSH pubkey | `nex identity ssh` | OpenSSH (stdout, pipeable) |
| Git signing | `nex identity git` | Configures git with SSH signing |
| WireGuard | `nex identity wg` | Private + public key pair |
| age encryption | `nex identity age` | age identity + recipient |

### Multiple identities

```bash
nex identity init --path ~/.config/styrene/work.key
nex identity show --path ~/.config/styrene/work.key
nex identity list    # scan for all identities on this machine
```

### Enroll with Signum hub

```bash
nex identity link https://signum.styrene.io
nex identity link https://signum.styrene.io --code INVITE-CODE
```

---

## Profiles

A profile is a TOML declaration of a machine's desired state: packages, shell config, git settings, system preferences.

### Profile format

```toml
[meta]
name = "edge-node"
description = "Standard edge node configuration"
extends = "team/base-profile"              # inherit from parent
compose = ["fragments/monitoring", "fragments/security"]  # merge fragments

[packages]
nix = ["htop", "bat", "ripgrep", "jq"]
brews = ["wget"]
casks = ["tailscale"]

[shell]
aliases = { ll = "eza -la", cat = "bat" }
env = { EDITOR = "vim" }
paths = ["$HOME/.local/bin"]

[git]
name = "Ops Team"
email = "ops@example.com"
default_branch = "main"
pull_rebase = true

[security]
firewall = true
ssh_hardening = true
```

### Chain resolution

Profiles support inheritance and composition:

- **extends**: Single parent profile. Child fields override parent.
- **compose**: List of fragment profiles merged in order (from same repo).

Resolution order: base parent -> composed fragments -> profile's own fields.

### Sign a profile

```bash
nex profile sign edge-node.toml
```

Produces `edge-node.signed.toml` with Ed25519 signature embedded in `[meta]`:

```toml
[meta]
name = "edge-node"
signed_by = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4"
signed_at = "2026-04-30T14:23:45Z"
pubkey = "cafe...beef"
signature = "deadbeef...cafe"
signed_source = "edge-node.toml"
```

Detached signature (for profiles you don't want to modify):

```bash
nex profile sign edge-node.toml --detached
# produces edge-node.sig
```

### Verify a profile

```bash
nex profile verify edge-node.signed.toml
```

Public-key only — no passphrase needed. Validates:
- Ed25519 signature over canonical TOML
- Pubkey matches the claimed `signed_by` identity hash
- Source ref binding (prevents rebinding attacks)

### Apply a profile locally

```bash
nex profile apply edge-node.signed.toml
nex profile apply github-user/my-profile    # from GitHub
nex profile apply . --dry-run               # preview changes
```

---

## Fleet Operations

### Discover your fleet

```bash
# All known peers
styrene peers

# Styrene nodes only
styrene peers --styrene-only

# Search by name or hash
styrene peers "edge"
```

### Query node status

```bash
# All nodes
styrene fleet status

# Specific node (remote RPC query)
styrene fleet status a1b2c3d4e5f6
styrene fleet status a1b2c3d4e5f6 --timeout 15
```

### Push a profile to a remote node

```bash
# Sign locally, push over mesh, verify + apply on remote
nex profile sign edge-node.toml
styrene fleet apply a1b2c3d4e5f6 edge-node.signed.toml
```

The remote node independently verifies the signature before applying. This is the primary configuration management pathway.

```bash
# Skip verification (use with caution)
styrene fleet apply a1b2c3d4e5f6 edge-node.signed.toml --no-verify

# Longer timeout for slow rebuilds
styrene fleet apply a1b2c3d4e5f6 edge-node.signed.toml --timeout 300
```

### Execute commands remotely

```bash
styrene fleet exec a1b2c3d4e5f6 uname -a
styrene fleet exec a1b2c3d4e5f6 systemctl status styrened
styrene fleet exec a1b2c3d4e5f6 nex list
styrene fleet exec a1b2c3d4e5f6 df -h --timeout 15
```

Output: stdout, stderr, and exit code from the remote node.

### Reboot a node

```bash
styrene fleet reboot a1b2c3d4e5f6
styrene fleet reboot a1b2c3d4e5f6 --delay 30    # 30s grace period
```

---

## Provisioning

### Build a bootable installer

```bash
# Interactive — prompts for profile, hostname, arch, disk
nex forge

# Non-interactive
nex forge my-org/edge-profile \
  --hostname edge-01 \
  --arch aarch64 \
  --disk /dev/sdb
```

Produces a bootable NixOS USB with the profile baked in. Boot the target machine from USB, then:

```bash
# On the target machine (booted from USB)
nex polymerize
```

Interactive installer that partitions the disk, installs NixOS, applies the baked-in profile, and configures the mesh daemon.

### Build a container image

```bash
nex build-image my-org/container-profile --name my-app --tag v1.0
```

---

## Tunnels

WireGuard tunnels are negotiated over LXMF between mesh peers.

```bash
# List active tunnels
styrene tunnel list

# Check tunnel status with a specific peer
styrene tunnel status a1b2c3d4e5f6

# Tear down a tunnel
styrene tunnel teardown a1b2c3d4e5f6
```

Tunnel establishment is automatic via the LXMF negotiation protocol (TUNNEL_OFFER / TUNNEL_ACCEPT). The daemon handles key exchange, WireGuard configuration, and keepalives.

---

## Messaging

Direct LXMF messaging between mesh peers:

```bash
# Send a message
styrene send a1b2c3d4e5f6 "deployment complete on edge-01"

# View message history
styrene messages a1b2c3d4e5f6
styrene messages a1b2c3d4e5f6 --limit 50
```

---

## Daemon Configuration

### View current config

```bash
styrene config
```

### Daemon identity

```bash
styrene identity
```

Shows the daemon's mesh identity hash, LXMF destination, and display name.

### Trigger announce

```bash
styrene announce
```

Forces an immediate mesh announce (normally periodic).

---

## Common Workflows

### Day 1: Provision a new node

```bash
# 1. Create the profile
vim edge-node.toml

# 2. Sign it
nex profile sign edge-node.toml

# 3. Build a bootable USB
nex forge my-org/edge-profile --hostname edge-01

# 4. Boot target from USB, run installer
nex polymerize

# 5. Verify the node joined the mesh
styrene peers --styrene-only
```

### Day 2: Update fleet configuration

```bash
# 1. Edit the profile
vim edge-node.toml

# 2. Re-sign
nex profile sign edge-node.toml

# 3. Push to all edge nodes
for node in $(styrene peers --styrene-only -q); do
  styrene fleet apply "$node" edge-node.signed.toml
done
```

### Incident response

```bash
# Check all node statuses
styrene fleet status

# Investigate a specific node
styrene fleet exec a1b2c3d4e5f6 journalctl -u styrened --since "1 hour ago"
styrene fleet exec a1b2c3d4e5f6 df -h
styrene fleet exec a1b2c3d4e5f6 free -h

# Emergency reboot
styrene fleet reboot a1b2c3d4e5f6
```

### Rotate identity

```bash
# Generate new identity
nex identity init --path ~/.config/styrene/identity-v2.key

# Re-sign all profiles with new identity
nex profile sign edge-node.toml

# Re-enroll with Signum
nex identity link https://signum.styrene.io --path ~/.config/styrene/identity-v2.key
```

---

## Security Model

### Authentication

All fleet operations are authenticated by Ed25519 identity. The mesh daemon verifies the caller's identity hash against its RBAC policy before executing commands.

### Authorization (RBAC)

| Capability | Operations |
|------------|-----------|
| `Status` | fleet status, remote inbox, remote messages |
| `Exec` | fleet exec, terminal sessions |
| `Reboot` | fleet reboot |
| `UpdateConfig` | fleet apply |

### Profile signing

Profiles are signed with the operator's Ed25519 key. The remote node:
1. Receives the profile over LXMF (encrypted in transit)
2. Verifies the Ed25519 signature against the embedded pubkey
3. Validates the pubkey matches the claimed signer identity hash
4. Only then applies the configuration

### Defense in depth

- **Transport encryption**: LXMF messages are encrypted end-to-end
- **Signature verification**: Profiles are verified on the remote node (not just the operator)
- **RBAC**: Capabilities are checked on both the local daemon (IPC) and remote daemon (RPC)
- **Source binding**: Profile signatures are bound to the original source ref, preventing rebinding

---

## Command Reference

### nex

```
nex init [--from <repo>]
nex install [--nix|--cask|--brew] <packages...>
nex remove [--cask|--brew] <packages...>
nex adopt
nex list
nex search <query>
nex update
nex switch
nex rollback
nex try <package>
nex diff
nex migrate
nex doctor
nex gc
nex self-update
nex relocate [--to <path>]

nex identity init [--path <file>]
nex identity show [--path <file>]
nex identity list
nex identity ssh [<label>] [--list] [--add <label>]
nex identity git [--show]
nex identity wg
nex identity age
nex identity link <url> [--code <invite>] [--path <file>]

nex profile apply <source>
nex profile sign <source> [--detached]
nex profile verify <source>

nex forge [<profile>] [--hostname <name>] [--disk <device>] [--output <dir>] [--arch <arch>]
nex polymerize [--bundle <path>]
nex build-image <profile> [--name <name>] [--tag <tag>]
nex develop <flake>
nex dev <project>
```

### styrene

```
styrene [--socket <path>] status
styrene [--socket <path>] peers [<query>] [--styrene-only]
styrene [--socket <path>] send <destination> <content> [--title <title>]
styrene [--socket <path>] messages <peer> [--limit <n>]
styrene [--socket <path>] identity
styrene [--socket <path>] announce
styrene [--socket <path>] config

styrene [--socket <path>] fleet status [<node>] [--timeout <secs>]
styrene [--socket <path>] fleet exec <node> <cmd> [<args>...] [--timeout <secs>]
styrene [--socket <path>] fleet reboot <node> [--delay <secs>]
styrene [--socket <path>] fleet apply <node> <profile> [--no-verify] [--timeout <secs>]

styrene [--socket <path>] tunnel list
styrene [--socket <path>] tunnel status <peer>
styrene [--socket <path>] tunnel establish <peer>
styrene [--socket <path>] tunnel teardown <peer>
```
