#!/usr/bin/env python3
"""Run the iOS-to-Android local-hub message acceptance lane."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys
import time
import uuid
import xml.etree.ElementTree as ET
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Callable, Iterable


REPO_ROOT = Path(__file__).resolve().parent.parent
SCENARIO_ID = "mobile.messaging.cross-platform-ios-to-android"
CORPUS_CASE_ID = "mobile.messaging.cross-platform-roundtrip"
APP_BUNDLE_ID = "io.styrene.mesh"
MAX_XCODE_LOG_BYTES = 1_048_576


class RunnerError(RuntimeError):
    pass


def utc_now() -> str:
    return datetime.now(UTC).isoformat()


def run(
    command: list[str],
    *,
    timeout: float = 120,
    check: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=timeout,
        env=env,
        check=False,
    )
    if check and result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise RunnerError(f"command failed ({result.returncode}): {' '.join(command)}\n{detail}")
    return result


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_bounds(value: str) -> tuple[int, int]:
    match = re.fullmatch(r"\[(\d+),(\d+)]\[(\d+),(\d+)]", value)
    if not match:
        raise RunnerError(f"invalid Android UI bounds: {value}")
    left, top, right, bottom = map(int, match.groups())
    return ((left + right) // 2, (top + bottom) // 2)


class AndroidUI:
    def __init__(self, adb: Path, serial: str) -> None:
        self.adb = adb
        self.serial = serial

    def command(self, *arguments: str, timeout: float = 30) -> subprocess.CompletedProcess[str]:
        return run([str(self.adb), "-s", self.serial, *arguments], timeout=timeout)

    def dump(self) -> tuple[str, ET.Element]:
        output = self.command("exec-out", "uiautomator", "dump", "/dev/tty").stdout
        start = output.find("<?xml")
        end = output.rfind("</hierarchy>")
        if start < 0 or end < 0:
            raise RunnerError("Android did not return a semantic UI hierarchy")
        xml = output[start : end + len("</hierarchy>")]
        try:
            return xml, ET.fromstring(xml)
        except ET.ParseError as error:
            raise RunnerError("Android returned malformed semantic UI XML") from error

    @staticmethod
    def texts(root: ET.Element) -> list[str]:
        return [value for node in root.iter() if (value := node.attrib.get("text"))]

    @staticmethod
    def find(
        root: ET.Element,
        predicate: Callable[[ET.Element], bool],
    ) -> ET.Element | None:
        return next((node for node in root.iter() if predicate(node)), None)

    @staticmethod
    def clickable_ancestor(root: ET.Element, node: ET.Element) -> ET.Element:
        parents = {child: parent for parent in root.iter() for child in parent}
        current: ET.Element | None = node
        while current is not None:
            if current.attrib.get("clickable") == "true":
                return current
            current = parents.get(current)
        raise RunnerError(f"Android semantic element is not actionable: {node.attrib}")

    def tap_node(self, root: ET.Element, node: ET.Element) -> None:
        target = self.clickable_ancestor(root, node)
        x, y = parse_bounds(target.attrib.get("bounds", ""))
        self.command("shell", "input", "tap", str(x), str(y))

    def tap_text(self, value: str, *, contains: bool = False) -> None:
        _, root = self.dump()

        def matches(node: ET.Element) -> bool:
            candidates = (node.attrib.get("text", ""), node.attrib.get("content-desc", ""))
            return any(value in candidate if contains else value == candidate for candidate in candidates)

        node = self.find(root, matches)
        if node is None:
            raise RunnerError(f"Android semantic element not found: {value}")
        self.tap_node(root, node)

    def wait_for_text(self, value: str, *, timeout: float, contains: bool = False) -> tuple[str, ET.Element]:
        deadline = time.monotonic() + timeout
        last_texts: list[str] = []
        while time.monotonic() < deadline:
            xml, root = self.dump()
            last_texts = self.texts(root)
            if any(value in text if contains else value == text for text in last_texts):
                return xml, root
            time.sleep(0.5)
        raise RunnerError(f"Android text did not appear: {value}; visible={last_texts}")


def booted_ios_simulator(requested_id: str | None) -> dict[str, Any]:
    payload = json.loads(run(["xcrun", "simctl", "list", "devices", "booted", "--json"]).stdout)
    devices = [device for runtime in payload["devices"].values() for device in runtime]
    if requested_id:
        devices = [device for device in devices if device["udid"] == requested_id]
    ios_devices = [device for device in devices if "iPhone" in device["name"]]
    if len(ios_devices) != 1:
        raise RunnerError("exactly one booted iPhone simulator is required; pass --simulator-id to disambiguate")
    return ios_devices[0]


def bounded_log(path: Path) -> None:
    if path.stat().st_size <= MAX_XCODE_LOG_BYTES:
        return
    with path.open("rb") as handle:
        handle.seek(-MAX_XCODE_LOG_BYTES, os.SEEK_END)
        retained = handle.read()
    path.write_bytes(b"[earlier xcodebuild output omitted]\n" + retained)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--android-serial", default="emulator-5554")
    parser.add_argument("--android-sdk", type=Path)
    parser.add_argument("--simulator-id")
    parser.add_argument("--apk", type=Path, default=REPO_ROOT / "android/app/build/outputs/apk/debug/app-debug.apk")
    args = parser.parse_args()

    sdk = args.android_sdk or Path(os.environ.get("ANDROID_HOME", Path.home() / "Library/Android/sdk"))
    if not (sdk / "platform-tools/adb").is_file():
        homebrew_sdk = Path("/opt/homebrew/share/android-commandlinetools")
        if args.android_sdk is None and "ANDROID_HOME" not in os.environ and (homebrew_sdk / "platform-tools/adb").is_file():
            sdk = homebrew_sdk
    adb = sdk / "platform-tools/adb"
    if not adb.is_file():
        raise RunnerError(f"adb not found under Android SDK: {sdk}")
    apk = args.apk.resolve()
    if not apk.is_file():
        raise RunnerError(f"Android APK not found: {apk}; run just android-deploy")
    for executable in ("xcodebuild", "xcrun", "just", "git"):
        if shutil.which(executable) is None:
            raise RunnerError(f"required executable not found: {executable}")

    simulator = booted_ios_simulator(args.simulator_id)
    correlation_id = f"{SCENARIO_ID}-{datetime.now(UTC).strftime('%Y%m%dT%H%M%SZ')}-{uuid.uuid4().hex[:8]}"
    android_profile = f"android-{uuid.uuid4().hex[:12]}"
    ios_profile = f"ios-{uuid.uuid4().hex[:12]}"
    message = f"styrene integration {correlation_id}"
    run_root = REPO_ROOT / "target/mobile-integration/runs" / correlation_id
    run_root.mkdir(parents=True)
    xcode_log = run_root / "xcodebuild.log"
    result_bundle = run_root / "ios.xcresult"
    evidence_path = run_root / "evidence.json"
    milestones: list[dict[str, str]] = []
    evidence: dict[str, Any] = {
        "schema_version": 1,
        "scenario_id": SCENARIO_ID,
        "corpus_case_id": CORPUS_CASE_ID,
        "corpus_action_slice": ["launch-cross-platform-pair", "send-a-to-b"],
        "correlation_id": correlation_id,
        "started_at": utc_now(),
        "message": message,
        "android": {"serial": args.android_serial, "profile": android_profile, "apk_sha256": sha256(apk)},
        "ios": {"simulator": simulator, "profile": ios_profile},
        "milestones": milestones,
        "outcome": "running",
    }
    revision = run(["git", "rev-parse", "HEAD"]).stdout.strip()
    evidence["source"] = {
        "revision": revision,
        "dirty": bool(run(["git", "status", "--porcelain"]).stdout.strip()),
    }

    def milestone(name: str, detail: str = "") -> None:
        milestones.append({"at": utc_now(), "name": name, "detail": detail})

    hub_was_running = run(["./scripts/local-mobile-hub.sh", "status"], check=False).returncode == 0
    ui = AndroidUI(adb, args.android_serial)
    failure: BaseException | None = None
    try:
        if not hub_was_running:
            run(["just", "mobile-hub-start"], timeout=180)
            milestone("hub_started")
        else:
            milestone("hub_reused")
        run(["./scripts/local-mobile-hub.sh", "android-probe", args.android_serial])
        milestone("hub_ready", "host and Android transport probes passed")

        ui.command("get-state")
        evidence["android"]["release"] = ui.command("shell", "getprop", "ro.build.version.release").stdout.strip()
        evidence["android"]["model"] = ui.command("shell", "getprop", "ro.product.model").stdout.strip()
        ui.command("install", "-r", str(apk), timeout=180)
        ui.command("shell", "am", "force-stop", APP_BUNDLE_ID)
        ui.command(
            "shell", "am", "start", "-W", "-n", f"{APP_BUNDLE_ID}/.MainActivity",
            "--es", "io.styrene.mesh.integration.PROFILE", android_profile,
            "--es", "io.styrene.mesh.integration.HUB_ADDRESS", "10.0.2.2:4242",
            "--es", "io.styrene.mesh.integration.DISPLAY_NAME", "'Android A'",
            "--ez", "io.styrene.mesh.integration.RESET_STATE", "true",
            timeout=60,
        )
        ui.wait_for_text("Mesh connected", timeout=30)
        ui.tap_text("Network")
        ui.wait_for_text("Announce", timeout=10)
        ui.tap_text("Announce")
        milestone("android_connected", "Mesh connected after the Android hub transport probe")
        milestone("android_launched", android_profile)

        derived_data = REPO_ROOT / "target/mobile-integration/ios-derived-data"
        build_command = [
            "xcodebuild",
            "-project", "ios/StyreneMobile.xcodeproj",
            "-scheme", "StyreneMobileIntegration",
            "-configuration", "Debug",
            "-destination", f"id={simulator['udid']}",
            "-derivedDataPath", str(derived_data),
            "build-for-testing",
        ]
        with xcode_log.open("w", encoding="utf-8") as log_handle:
            build_result = subprocess.run(
                build_command,
                cwd=REPO_ROOT,
                stdout=log_handle,
                stderr=subprocess.STDOUT,
                text=True,
                timeout=180,
                check=False,
            )
        if build_result.returncode != 0:
            raise RunnerError(f"iOS integration test build failed; inspect {xcode_log}")

        xctestrun_files = list((derived_data / "Build/Products").glob("StyreneMobileIntegration_*.xctestrun"))
        if len(xctestrun_files) != 1:
            raise RunnerError(f"expected one generated iOS test manifest, found {len(xctestrun_files)}")
        xctestrun = xctestrun_files[0]
        with xctestrun.open("rb") as handle:
            test_manifest = plistlib.load(handle)
        test_target = test_manifest.get("StyreneMobileUITests")
        if not isinstance(test_target, dict):
            raise RunnerError("generated iOS test manifest does not contain StyreneMobileUITests")
        for environment_key in ("EnvironmentVariables", "TestingEnvironmentVariables"):
            environment = test_target.setdefault(environment_key, {})
            environment["STYRENE_INTEGRATION_MESSAGE"] = message
            environment["STYRENE_IOS_PROFILE"] = ios_profile
        with xctestrun.open("wb") as handle:
            plistlib.dump(test_manifest, handle)
        shutil.copy2(xctestrun, run_root / xctestrun.name)

        test_command = [
            "xcodebuild",
            "-xctestrun", str(xctestrun),
            "-destination", f"id={simulator['udid']}",
            "-resultBundlePath", str(result_bundle),
            "test-without-building",
        ]
        with xcode_log.open("a", encoding="utf-8") as log_handle:
            xcode_process = subprocess.Popen(
                test_command,
                cwd=REPO_ROOT,
                stdout=log_handle,
                stderr=subprocess.STDOUT,
                text=True,
            )
            deadline = time.monotonic() + 240
            while xcode_process.poll() is None and time.monotonic() < deadline:
                try:
                    ui.tap_text("Announce")
                except RunnerError:
                    pass
                time.sleep(2)
            if xcode_process.poll() is None:
                xcode_process.terminate()
                try:
                    xcode_process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    xcode_process.kill()
                raise RunnerError("iOS integration test timed out")
            if xcode_process.returncode != 0:
                raise RunnerError(f"iOS integration test failed; inspect {xcode_log}")
        bounded_log(xcode_log)
        milestone("ios_message_queued", message)

        ui.tap_text("Messages")
        unread_xml = ""
        unread_deadline = time.monotonic() + 30
        while time.monotonic() < unread_deadline:
            try:
                ui.tap_text("Refresh")
            except RunnerError:
                pass
            time.sleep(1)
            unread_xml, root = ui.dump()
            counts = [int(match.group(1)) for text in ui.texts(root) if (match := re.search(r"(\d+) unread", text))]
            if counts and max(counts) > 0:
                break
        else:
            raise RunnerError("Android did not expose an unread inbound message")
        (run_root / "android-unread.xml").write_text(unread_xml, encoding="utf-8")
        milestone("android_unread_incremented", str(max(counts)))

        conversation = ui.find(root, lambda node: bool(re.fullmatch(r"\d+ messages", node.attrib.get("text", ""))))
        if conversation is None:
            raise RunnerError("Android inbound conversation was not visible")
        ui.tap_node(root, conversation)
        received_xml, _ = ui.wait_for_text(message, timeout=20)
        (run_root / "android-received.xml").write_text(received_xml, encoding="utf-8")
        milestone("android_message_received", message)

        try:
            ui.tap_text("Back")
        except RunnerError:
            ui.command("shell", "input", "keyevent", "4")
        ui.tap_text("Refresh")
        time.sleep(1)
        read_xml, root = ui.dump()
        counts = [int(match.group(1)) for text in ui.texts(root) if (match := re.search(r"(\d+) unread", text))]
        if not counts or max(counts) != 0:
            raise RunnerError(f"Android unread state did not clear after opening conversation: {counts}")
        (run_root / "android-read.xml").write_text(read_xml, encoding="utf-8")
        milestone("android_unread_cleared")

        evidence["outcome"] = "passed"
        evidence["finished_at"] = utc_now()
        print(f"cross-platform integration passed: {correlation_id}")
    except BaseException as error:
        failure = error
        evidence["outcome"] = "failed"
        evidence["error"] = str(error)
        evidence["finished_at"] = utc_now()
    finally:
        cleanup_errors: list[str] = []
        try:
            ui.command("shell", "am", "force-stop", APP_BUNDLE_ID, timeout=30)
        except RunnerError as error:
            cleanup_errors.append(str(error))
        ios_cleanup = run(
            ["xcrun", "simctl", "terminate", simulator["udid"], APP_BUNDLE_ID],
            check=False,
            timeout=30,
        )
        if ios_cleanup.returncode not in (0, 3):
            cleanup_errors.append(ios_cleanup.stderr.strip() or "failed to stop iOS app")
        milestone("apps_stopped")
        if not hub_was_running:
            run(["./scripts/local-mobile-hub.sh", "stop"], check=False)
            milestone("hub_stopped")
        hub_log = REPO_ROOT / "target/mobile-integration/hub/styrened.log"
        if hub_log.is_file():
            lines = hub_log.read_text(encoding="utf-8", errors="replace").splitlines()
            (run_root / "hub.log").write_text("\n".join(lines[-200:]) + "\n", encoding="utf-8")
        if xcode_log.is_file():
            bounded_log(xcode_log)
        if cleanup_errors:
            evidence["cleanup_errors"] = cleanup_errors
            evidence["outcome"] = "failed"
            if failure is None:
                failure = RunnerError("; ".join(cleanup_errors))
        evidence_path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"evidence: {evidence_path}")

    if failure is not None:
        raise RunnerError(str(failure)) from failure
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RunnerError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
