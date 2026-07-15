# R36S-class Local Simulation

This target provides a fast developer smoke test for the constrained Linux deployment envelope. It builds and executes the real `styrene` product binary in an ARM64 Debian container under bounded CPU, memory, process, storage, and network conditions.

It is deliberately a **userspace simulation**, not RK3326 board emulation. It proves architecture portability, dependency closure, clean-room installation, and ephemeral runtime lifecycle. It does not prove panel/DTB compatibility, evdev mappings, battery or suspend behavior, GPU/display rendering, USB OTG, removable-media failure handling, or bootloader/kernel compatibility.

## Prerequisites

Podman is the default engine. Docker may be selected explicitly:

```bash
CONTAINER_ENGINE=docker just sim-r36s-build
```

On an ARM64 development host, the default `linux/arm64` image executes natively. On x86_64, the container engine needs ARM64 binfmt/QEMU support.

## Build

```bash
just sim-r36s-build
```

This builds `simulation/r36s/Containerfile` and creates:

```text
localhost/styrene-r36s-sim:dev
```

Override with `STYRENE_R36S_IMAGE`.

## Smoke test

```bash
just sim-r36s-smoke
```

The smoke test runs with:

- `linux/arm64` platform;
- 4 virtual CPUs;
- 768 MiB container memory ceiling;
- 128-process ceiling;
- 256 MiB `/state` tmpfs;
- network disabled;
- non-root runtime user.

It verifies:

1. the ARM64 executable starts and reports its version;
2. `styrene doctor` creates and validates a persistent installation under isolated state;
3. `styrene ghost-check` starts the actual embedded runtime, reaches IPC readiness, shuts down normally, and removes session state.

The 768 MiB limit is intentionally a bring-up value, not the proposed 64 MiB application budget. Lower it during characterization:

```bash
STYRENE_R36S_MEMORY=512m just sim-r36s-smoke
STYRENE_R36S_MEMORY=256m just sim-r36s-smoke
STYRENE_R36S_MEMORY=128m just sim-r36s-smoke
```

A successful run at a container memory limit is not equivalent to measured device RSS/PSS or a support claim.

## Kick the tires

Open a shell in the same constrained image:

```bash
just sim-r36s-shell
```

The shell keeps networking enabled for manual experiments and is intentionally interactive. State remains ephemeral unless the script is extended with an explicit developer-owned volume.

Useful commands inside it:

```bash
styrene --version
styrene doctor --root /state/doctor
styrene ghost-check --root /state/ghost --timeout 15
cat /proc/meminfo
ps -ef
```

## Follow-on simulation layers

This is layer 1 of the evidence ladder:

1. **Container userspace:** implemented here.
2. **Viewport/input conformance:** fixed 640×480 renderer plus synthetic gamepad action traces, after the interaction constraints are decided.
3. **System image VM/emulation:** boot a Nex-built ARM image under QEMU where the selected kernel/board support permits it.
4. **Physical hardware:** panel, input, lid/power, Wi-Fi, storage, and battery validation.

Do not add fake evdev or display claims to this container. Those should be deterministic conformance fixtures tied to measured device mappings, not ad hoc approximations.
