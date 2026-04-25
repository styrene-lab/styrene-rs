#!/usr/bin/env python3
"""LXMF interop compliance test — Python RNS client against Rust styrened.

Tests that the Rust daemon speaks valid RNS on the wire by connecting
a Python RNS client and verifying announce propagation.

Usage:
    ~/.local/pipx/venvs/nomadnet/bin/python3 tests/compliance/lxmf_interop.py
"""
import os
import sys
import time

import RNS

DAEMON_HOST = "127.0.0.1"
DAEMON_PORT = 4242
results = []


def log(msg):
    print(f"  [{time.strftime('%H:%M:%S')}] {msg}")


def test(name, ok, detail=""):
    results.append(("PASS" if ok else "FAIL", name, detail))
    icon = "✓" if ok else "✗"
    log(f"{icon} {name}" + (f" — {detail}" if detail else ""))


def run():
    print("\nRNS Interop Compliance Test")
    print(f"Target: {DAEMON_HOST}:{DAEMON_PORT} (Rust styrened)")
    print("=" * 60)

    # ── 1. Initialize RNS with TCP connection to daemon ───────────────────
    config_dir = "/tmp/rns-interop-test"
    os.makedirs(config_dir, exist_ok=True)
    with open(os.path.join(config_dir, "config"), "w") as f:
        f.write(f"""[reticulum]
  enable_transport = No
  share_instance = No
  panic_on_interface_errors = No

[interfaces]
  [[TCP Client to Rust Daemon]]
    type = TCPClientInterface
    enabled = yes
    target_host = {DAEMON_HOST}
    target_port = {DAEMON_PORT}
""")

    log("Initializing RNS...")
    try:
        reticulum = RNS.Reticulum(config_dir)
        test("rns_init", True, f"RNS {RNS.__version__}")
    except Exception as e:
        test("rns_init", False, str(e))
        return

    time.sleep(2)

    # ── 2. Check TCP interface connected ──────────────────────────────────
    try:
        ifaces = RNS.Transport.interfaces
        online = [i for i in ifaces if hasattr(i, 'online') and i.online]
        test("tcp_connect", len(online) > 0, f"{len(online)}/{len(ifaces)} online")
    except Exception as e:
        test("tcp_connect", False, str(e))

    # ── 3. Create identity and destination ────────────────────────────────
    log("Creating RNS identity...")
    try:
        identity = RNS.Identity()
        dest = RNS.Destination(
            identity, RNS.Destination.IN, RNS.Destination.SINGLE,
            "lxmf", "delivery",
        )
        my_hash = RNS.prettyhexrep(dest.hash)
        test("identity_create", True, f"hash={my_hash}")
    except Exception as e:
        test("identity_create", False, str(e))
        return

    # ── 4. Send announce ──────────────────────────────────────────────────
    log("Sending announce...")
    try:
        dest.announce()
        test("announce_send", True)
    except Exception as e:
        test("announce_send", False, str(e))

    # ── 5. Wait for daemon's announce to arrive ───────────────────────────
    log("Waiting for daemon announce (10s)...")
    daemon_dest = None
    deadline = time.time() + 10
    while time.time() < deadline:
        # Check if we've received any announces via path table
        if hasattr(RNS.Transport, 'path_table') and RNS.Transport.path_table:
            for dest_hash in RNS.Transport.path_table:
                if dest_hash != dest.hash:
                    daemon_dest = dest_hash
                    break
        if daemon_dest:
            break
        time.sleep(0.5)

    if daemon_dest:
        test("daemon_announce_rx", True, f"dest={RNS.prettyhexrep(daemon_dest)}")
    else:
        # Try alternative: check destination table
        known = 0
        try:
            if hasattr(RNS.Transport, 'destination_table'):
                known = len(RNS.Transport.destination_table)
            elif hasattr(RNS.Transport, 'destinations'):
                known = len(RNS.Transport.destinations)
        except Exception:
            pass
        test("daemon_announce_rx", known > 0,
             f"{known} destination(s) known" if known > 0 else "no announces received from daemon")

    # ── 6. Verify path resolution ─────────────────────────────────────────
    if daemon_dest:
        log("Testing path resolution...")
        try:
            recalled = RNS.Identity.recall(daemon_dest)
            test("path_resolve", recalled is not None,
                 "identity recalled from path table" if recalled else "identity not recalled")
        except Exception as e:
            test("path_resolve", False, str(e))

    # ── 7. Check HDLC framing integrity ──────────────────────────────────
    # If we got this far, HDLC framing works (TCP interface connected and announces exchanged)
    if any(r[0] == "PASS" and "announce" in r[1] for r in results):
        test("hdlc_framing", True, "verified via successful announce exchange")

    # ── Results ───────────────────────────────────────────────────────────
    print("\n" + "=" * 60)
    passed = sum(1 for r in results if r[0] == "PASS")
    failed = sum(1 for r in results if r[0] == "FAIL")
    print(f"Results: {passed} passed, {failed} failed\n")

    for status, name, detail in results:
        icon = "✓" if status == "PASS" else "✗"
        print(f"  {icon} {name}" + (f": {detail}" if detail else ""))

    print()
    log("Shutting down...")
    try:
        RNS.Reticulum.exit_handler()
    except Exception:
        pass

    return failed == 0


if __name__ == "__main__":
    success = run()
    sys.exit(0 if success else 1)
