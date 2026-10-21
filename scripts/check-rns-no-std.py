#!/usr/bin/env python3
"""Independent Rust 1.97 aarch64-unknown-none gate; see fixture README.md.

Only `prepare` omits compilation. It materializes a locked fixture for explicit
CI dependency acquisition. All validation commands are offline and locked.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import resource
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

TARGET = "aarch64-unknown-none"
COMPLETE = "RNS-NOSTD:COMPLETE"
BOOT_ONLY = "RNS-NOSTD:BOOT-ONLY"
FAILURE = "RNS-NOSTD:PANIC-OR-OOM"
PROFILES = ("", "alloc", "alloc,fernet-aes128")
FIXTURE = Path("tests/compile/rns-no-std")
SOURCE = Path(__file__).resolve().parents[1]
MAX_LOG = 64 * 1024 * 1024


class GateError(RuntimeError):
    """A failed check or unavailable prerequisite, never an implicit fallback."""


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def probe(args: list[str], cwd: Path, *, trim: bool = True) -> str:
    result = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=30, check=False)
    if result.returncode:
        raise GateError(f"probe failed: {args!r}: {result.stderr.strip()}")
    return result.stdout.strip() if trim else result.stdout


def snapshot(source: Path) -> dict[str, Any]:
    names = probe(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], source, trim=False)
    files = {}
    for name in sorted(set(names.rstrip("\0").split("\0")) - {""}):
        path = source / name
        if path.is_symlink():
            if name.startswith(("crates/libs/styrene-rns/", str(FIXTURE) + "/")):
                raise GateError(f"symlink in build input: {name}")
            content = hashlib.sha256(os.fsencode(os.readlink(path))).hexdigest()
        elif path.is_file():
            content = sha256(path)
        elif not path.exists():
            content = "deleted"
        else:
            raise GateError(f"unsupported source entry: {name}")
        mode = stat.S_IMODE(path.lstat().st_mode) if path.exists() or path.is_symlink() else 0
        files[name] = [mode, content]
    return {
        "head": probe(["git", "rev-parse", "HEAD"], source),
        "status": probe(["git", "status", "--porcelain=v1", "--untracked-files=all"], source, trim=False),
        "content_sha256": hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest(),
        "files": files,
    }


def unchanged(source: Path, before: dict[str, Any]) -> None:
    if snapshot(source) != before:
        raise GateError("source changed during validation; result invalidated")


def fixture_snapshot(root: Path) -> dict[str, str]:
    files = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise GateError(f"symlink in materialized fixture: {path}")
        if path.is_file():
            files[str(path.relative_to(root))] = sha256(path)
    return files


def external_root(path: Path, source: Path) -> Path:
    """Refuse symlinked or source-overlapping output roots before creating files."""
    path = path.absolute()
    if any(part.is_symlink() for part in (path, *path.parents)):
        raise GateError(f"symlinked output root: {path}")
    resolved = path.resolve()
    if resolved == source or resolved.is_relative_to(source) or source.is_relative_to(resolved):
        raise GateError(f"output overlaps source: {resolved}")
    path.mkdir(parents=True, exist_ok=True)
    if not path.is_dir():
        raise GateError(f"output root is not a directory: {path}")
    return path


def materialize(source: Path, destination: Path) -> None:
    """Copy templates, resolving only the exact RNS path; never run Cargo in source."""
    destination.mkdir()
    for path in sorted((source / FIXTURE).rglob("*")):
        if path.is_symlink():
            raise GateError(f"symlink in fixture: {path}")
        if not path.is_file() or path.name == "README.md":
            continue
        relative = path.relative_to(source / FIXTURE)
        target = destination / relative
        if path.name.endswith(".in"):
            target = target.with_suffix("")
        target.parent.mkdir(parents=True, exist_ok=True)
        if path.name.endswith(".in"):
            text = path.read_text().replace(
                "@RNS_PATH@", json.dumps(str(source / "crates/libs/styrene-rns"))
            )
            target.write_text(text)
        else:
            shutil.copyfile(path, target)
    for ancestor in (destination, *destination.parents):
        for config in ("config", "config.toml"):
            if (ancestor / ".cargo" / config).exists():
                raise GateError(f"ambient Cargo configuration at {ancestor / '.cargo' / config}")


def seed_cache(seed: Path, destination: Path) -> None:
    # Copy only public registry data: never credentials, config, Git credentials,
    # global cache databases or locks. Each run owns its writable Cargo home.
    registry = seed.resolve(strict=True) / "registry"
    if registry.is_symlink() or not registry.is_dir():
        raise GateError(f"reviewed registry cache missing: {registry}")
    for path in registry.rglob("*"):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise GateError(f"unsupported entry in registry cache: {path}")
    shutil.copytree(registry, destination / "registry")


def tool(path: str, name: str, cwd: Path, version_args: tuple[str, ...] = ("--version",)) -> dict[str, str]:
    executable = Path(path).expanduser()
    if not executable.is_absolute() or not executable.is_file() or not os.access(executable, os.X_OK):
        raise GateError(f"{name} must be an existing absolute executable path: {path}")
    resolved = executable.resolve(strict=True)
    version = probe([str(resolved), *version_args], cwd)
    return {"path": str(resolved), "version": version, "sha256": sha256(resolved)}


def result_ok(status: int, timed_out: bool, output: str, marker: str) -> bool:
    lines = output.splitlines()
    return (
        not timed_out
        and status == 0
        and lines.count(marker) == 1
        and all(line == marker for line in lines if line.startswith("RNS-NOSTD:"))
    )


def expected_std_failure(status: int, output: str) -> bool:
    errors = []
    for line in output.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") == "compiler-message" and message["message"]["level"] == "error":
            errors.append(message)
    return status == 101 and bool(errors) and all(
        error["target"]["name"] == "rns_no_std_negative"
        and (error["message"].get("code") or {}).get("code") == "E0463"
        and "can't find crate for `std`" in error["message"]["message"]
        and any(span["file_name"].endswith("src/lib.rs") for span in error["message"]["spans"])
        for error in errors
    )


def log_limit() -> None:
    resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_LOG, MAX_LOG))


def kill_group(pid: int) -> None:
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def run_command(
    args: list[str], cwd: Path, env: dict[str, str], log: Path, timeout: float
) -> dict[str, Any]:
    """Own one foreground process group; kill and reap it on deadline/cancellation."""
    started = datetime.now(timezone.utc).isoformat()
    timed_out = False
    with log.open("xb") as output:
        process = subprocess.Popen(
            args, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=output,
            stderr=subprocess.STDOUT, start_new_session=True, preexec_fn=log_limit,
        )
        try:
            status = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            kill_group(process.pid)
            status = process.wait()
        except BaseException:
            kill_group(process.pid)
            process.wait()
            raise
        finally:
            # Also stop any remaining descendants if their parent exited first.
            kill_group(process.pid)
    return {
        "args": args, "cwd": str(cwd), "environment": env, "started": started,
        "status": status, "timed_out": timed_out, "timeout_seconds": timeout,
        "log": log.name, "log_sha256": sha256(log),
    }


def qemu_args(qemu: str, artifact: Path) -> list[str]:
    return [
        qemu, "-machine", "virt-11.1,secure=off,virtualization=off", "-cpu", "cortex-a53",
        "-accel", "tcg,thread=single", "-smp", "1", "-m", "128M", "-nodefaults",
        "-nic", "none", "-display", "none", "-serial", "none", "-monitor", "none",
        "-no-reboot", "-semihosting-config", "enable=on,target=native", "-kernel", str(artifact),
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("prepare", "check", "execute", "bootstrap"))
    parser.add_argument("--cargo", required=True, help="absolute Cargo 1.97.0 executable")
    parser.add_argument("--rustc", required=True, help="absolute rustc 1.97.0 executable")
    parser.add_argument("--cache", type=Path, help="reviewed registry cache to copy (required except prepare)")
    parser.add_argument("--qemu", help="absolute QEMU 11.1.0 executable (execute/bootstrap only)")
    parser.add_argument("--timeout", type=int, default=60, choices=range(1, 121), metavar="1..120")
    args = parser.parse_args()
    source = SOURCE
    scratch = external_root(Path("/tmp/opencode"), source)
    run = Path(tempfile.mkdtemp(prefix="rns-no-std-", dir=scratch))
    store = external_root(Path.home() / ".local/state/styrene-lab/issue-78/runs/validation", source)
    evidence = store / run.name
    evidence.mkdir(mode=0o700)
    logs = run / "logs"
    logs.mkdir()
    report: dict[str, Any] = {
        "schema": 1, "run": str(run), "evidence": str(evidence), "source": str(source),
        "started": datetime.now(timezone.utc).isoformat(),
        "host": platform.node(), "platform": platform.platform(), "mode": args.mode,
        "target": TARGET, "commands": [], "artifacts": {}, "tools": {},
        "profiles": list(PROFILES),
        "evidence_class": {"prepare": "materialization-only", "check": "target-compile",
                           "execute": "synthetic-emulation", "bootstrap": "boot-substrate-only"}[args.mode],
        "review": "unreviewed; execution is not acceptance", "status": "blocked",
    }
    before = None
    fixture = run / "fixture"
    try:
        before = snapshot(source)
        report["source_before"] = before
        fixture_files = {name: entry for name, entry in before["files"].items()
                         if name.startswith(str(FIXTURE) + "/")}
        report["fixture_content_sha256"] = hashlib.sha256(
            json.dumps(fixture_files, sort_keys=True).encode()
        ).hexdigest()
        report["product_lock_sha256"] = sha256(source / "Cargo.lock")
        report["fixture_lock_sha256"] = sha256(source / FIXTURE / "Cargo.lock")
        materialize(source, fixture)
        report["fixture_before"] = fixture_snapshot(fixture)
        for directory in ("home", "tmp", "cargo-home", "target"):
            (run / directory).mkdir()
        cargo = tool(args.cargo, "cargo", run)
        rustc = tool(args.rustc, "rustc", run)
        report["tools"].update(cargo=cargo, rustc=rustc)
        report["tools"]["python"] = tool(sys.executable, "python", run)
        report["tools"]["git"] = tool(shutil.which("git") or "", "git", run)
        if not re.match(r"cargo 1\.97\.0\b", cargo["version"]) or not re.match(
            r"rustc 1\.97\.0\b", rustc["version"]
        ):
            raise GateError("matching Cargo/rustc 1.97.0 required")
        verbose = probe([rustc["path"], "-vV"], run)
        report["rustc_verbose"] = verbose
        report["target_cfg"] = probe([rustc["path"], "--print", "cfg", "--target", TARGET], run)
        for atomic in ('target_has_atomic="64"', 'target_has_atomic="ptr"'):
            if atomic not in report["target_cfg"].splitlines():
                raise GateError(f"target lacks required {atomic}")
        libdir = Path(probe([rustc["path"], "--print", "target-libdir", "--target", TARGET], run))
        for library in ("core", "alloc", "compiler_builtins"):
            if not list(libdir.glob(f"lib{library}-*.rlib")):
                raise GateError(f"missing Rust 1.97 target library {library}: {libdir}")
        report["target_libraries"] = {str(path): sha256(path) for path in sorted(libdir.glob("*.rlib"))}
        env = {
            "PATH": os.environ.get("PATH", os.defpath), "HOME": str(run / "home"),
            "TMPDIR": str(run / "tmp"), "CARGO_HOME": str(run / "cargo-home"),
            "CARGO_TARGET_DIR": str(run / "target"), "CARGO_BUILD_JOBS": "2",
            "RUSTC": rustc["path"], "RUSTFLAGS": "-Dwarnings", "CARGO_NET_OFFLINE": "true",
            "CARGO_TERM_COLOR": "never", "LC_ALL": "C",
        }
        report["environment"] = env
        if args.mode == "prepare":
            report["status"] = "prepared-only"
            print(run)
            return 0
        if args.cache is None:
            raise GateError("--cache is required; validation never downloads dependencies")
        seed_cache(args.cache, run / "cargo-home")
        report["cache_seed"] = str(args.cache.resolve())
        if args.mode in ("execute", "bootstrap"):
            if args.qemu is None:
                raise GateError("--qemu is required for target execution")
            qemu = tool(args.qemu, "qemu", run)
            if not qemu["version"].startswith("QEMU emulator version 11.1.0"):
                raise GateError("QEMU 11.1.0 required by the reviewed boot contract")
            sysroot = Path(probe([rustc["path"], "--print", "sysroot"], run))
            host = next(line.removeprefix("host: ") for line in verbose.splitlines() if line.startswith("host: "))
            linker = tool(str(sysroot / "lib/rustlib" / host / "bin/rust-lld"), "rust-lld", run,
                          ("-flavor", "gnu", "--version"))
            report["tools"].update(qemu=qemu, linker=linker)

        def command(label: str, command_args: list[str], cwd: Path = fixture,
                    overrides: dict[str, str] | None = None, timeout: float = 600) -> tuple[dict[str, Any], str]:
            unchanged(source, before)
            if fixture_snapshot(fixture) != report["fixture_before"]:
                raise GateError("materialized fixture changed during validation")
            result = run_command(command_args, cwd, env | (overrides or {}), logs / f"{label}.log", timeout)
            report["commands"].append(result)
            unchanged(source, before)
            if fixture_snapshot(fixture) != report["fixture_before"]:
                raise GateError("materialized fixture changed during validation")
            return result, (logs / result["log"]).read_text(errors="replace")

        def cargo_command(label: str, verb: str, extra: list[str], **kwargs: Any) -> tuple[dict[str, Any], str]:
            return command(label, [cargo["path"], verb, "--offline", "--locked", *extra], **kwargs)

        def passed(result: dict[str, Any]) -> None:
            if result["timed_out"] or result["status"]:
                raise GateError(f"command failed: {result['log']} (exit {result['status']})")

        result, _ = command("allocator-build", [rustc["path"], "--edition=2024", "-Dwarnings",
                            "--test", str(fixture / "boot/src/bump.rs"), "-o", str(run / "allocator-tests")])
        passed(result)
        result, _ = command("allocator-tests", [str(run / "allocator-tests")], timeout=30)
        passed(result)

        if args.mode in ("check", "execute"):
            # Negative control is separate from the product graph so baseline
            # dependency errors cannot accidentally satisfy its diagnostic test.
            result, output = cargo_command(
                "negative", "check", ["--lib", "--target", TARGET, "--message-format=json"],
                cwd=fixture / "negative",
            )
            if result["timed_out"] or not expected_std_failure(result["status"], output):
                raise GateError("negative control did not fail specifically for missing target std")
            report["negative_control"] = "intended-E0463"
            for index, features in enumerate(PROFILES):
                options = ["--no-default-features", "--target", TARGET]
                if features:
                    options += ["--features", features]
                result, _ = cargo_command(f"tree-{index}", "tree", ["-p", "rns-no-std-smoke", "-e", "normal,build,features", *options])
                passed(result)
                result, _ = cargo_command(f"metadata-{index}", "metadata", [
                    "--format-version", "1", "--filter-platform", TARGET, "--no-default-features",
                    *(["--features", features] if features else []),
                ])
                passed(result)
                result, _ = cargo_command(f"check-{index}", "check", ["-p", "rns-no-std-smoke", "--lib", "-vv", *options])
                passed(result)

        if args.mode in ("execute", "bootstrap"):
            scenarios = [("panic", "scenario-panic"), ("oom", "scenario-oom")]
            scenarios += [("boot", "")] if args.mode == "bootstrap" else [
                (f"smoke-{index}", ",".join(filter(None, ("smoke", features))))
                for index, features in enumerate(PROFILES)
            ]
            for label, features in scenarios:
                mapfile = run / f"{label}.map"
                flags = (f"-Dwarnings -C target-cpu=cortex-a53 -C linker={linker['path']} "
                         f"-C link-arg=-T{fixture / 'boot/linker.ld'} -C link-arg=-Map={mapfile}")
                result, _ = cargo_command(f"link-{label}", "build", [
                    "-p", "rns-no-std-boot", "--release", "--no-default-features", "--target", TARGET,
                    *(["--features", features] if features else []),
                ], overrides={"RUSTFLAGS": flags})
                passed(result)
                artifact = run / "target" / TARGET / "release/rns-no-std-boot"
                saved = evidence / f"{label}.elf"
                shutil.copyfile(artifact, saved)
                shutil.copyfile(mapfile, evidence / mapfile.name)
                report["artifacts"][label] = {"elf_sha256": sha256(saved), "map_sha256": sha256(mapfile)}
                result, output = command(f"execute-{label}", qemu_args(qemu["path"], saved), timeout=args.timeout)
                if label in ("panic", "oom"):
                    if result["timed_out"] or result["status"] != 1 or output.splitlines() != [FAILURE]:
                        raise GateError(f"{label} did not terminate with the allocation-free failure marker and exit 1")
                elif not result_ok(result["status"], result["timed_out"], output,
                                   BOOT_ONLY if label == "boot" else COMPLETE):
                    raise GateError(f"{label} lacks its exact completion marker and zero exit")
        report["status"] = "passed-bootstrap-only" if args.mode == "bootstrap" else "passed"
        return 0
    except (GateError, OSError, subprocess.SubprocessError) as error:
        report["error"] = str(error)
        report["status"] = "failed" if report["commands"] else "blocked"
        print(f"rns-no-std: {error}", file=sys.stderr)
        return 1
    finally:
        if before is not None:
            report["source_after"] = snapshot(source)
            if report["source_after"] != before:
                report["status"] = "invalid-source-mutation"
                # `command` checks mutation too; prepare needs this final check.
        shutil.copytree(logs, evidence / "logs")
        if fixture.exists():
            report["fixture_after"] = fixture_snapshot(fixture)
            if "fixture_before" in report and report["fixture_after"] != report["fixture_before"]:
                report["status"] = "invalid-source-mutation"
            shutil.copytree(fixture, evidence / "fixture")
        report["evidence_files"] = {
            str(path.relative_to(evidence)): sha256(path)
            for path in sorted(evidence.rglob("*")) if path.is_file()
        }
        (evidence / "summary.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        print(f"rns-no-std evidence: {evidence}", file=sys.stderr)
        if report["status"] == "invalid-source-mutation":
            raise GateError("source changed during validation; result invalidated")


if __name__ == "__main__":
    sys.exit(main())
