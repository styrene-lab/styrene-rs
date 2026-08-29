#!/usr/bin/env python3
"""Generate deterministic fixtures with the pinned Python RNS/LXMF packages."""

from __future__ import annotations

import hashlib
import importlib
import importlib.metadata
import json
from pathlib import Path
import sys

IMPORT_ERROR = None
try:
    import RNS
    import RNS.vendor.umsgpack as msgpack
    from LXMF.LXMF import APP_NAME, PN_META_NAME
    from LXMF.LXMRouter import LXMRouter
    from LXMF.LXMessage import LXMessage
    from LXMF.LXMPeer import LXMPeer
    import LXMF.LXStamper as stamper

    token_module = importlib.import_module("RNS.Cryptography.Token")
    x25519_module = importlib.import_module("RNS.Cryptography.X25519")
    identity_module = importlib.import_module("RNS.Identity")
    router_module = importlib.import_module("LXMF.LXMRouter")
    message_module = importlib.import_module("LXMF.LXMessage")
except ImportError as error:
    IMPORT_ERROR = error


RNS_REVISION = "b48b96e61676504e0a4e527b33b9a0b4495c6872"
LXMF_REVISION = "795fdaa2b0777c13033787d933d1afc94a2377cb"
FIXED_TIME = 1_750_000_000.0
OUTPUT_DIR = Path(__file__).resolve().parent.parent / "fixtures" / "lxmf-propagation-v1"


class DeterministicEntropy:
    def __init__(self, label: str):
        self.label = label.encode("ascii")
        self.counter = 0

    def __call__(self, size: int) -> bytes:
        output = bytearray()
        while len(output) < size:
            output.extend(hashlib.sha256(self.label + self.counter.to_bytes(4, "big")).digest())
            self.counter += 1
        return bytes(output[:size])


def identity(label: str) -> RNS.Identity:
    first = hashlib.sha256(label.encode("ascii")).digest()
    return RNS.Identity.from_bytes(first + hashlib.sha256(first).digest())


def deterministic_stamp(material: bytes, cost: int, expand_rounds: int) -> tuple[bytes, int]:
    workblock = stamper.stamp_workblock(material, expand_rounds=expand_rounds)
    for counter in range(1 << 20):
        candidate = hashlib.sha256(b"styrene-fixture-stamp-v1" + counter.to_bytes(4, "big")).digest()
        if stamper.stamp_valid(candidate, cost, workblock):
            return candidate, stamper.stamp_value(workblock, candidate)
    raise RuntimeError("deterministic stamp search exhausted")


def write_artifact(name: str, data: bytes, artifacts: list[dict]) -> None:
    path = OUTPUT_DIR / name
    path.write_bytes(data)
    artifacts.append({"path": name, "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)})


def check_pins() -> None:
    expected = {
        "rns": ("1.4.2", RNS_REVISION),
        "lxmf": ("1.1.0", LXMF_REVISION),
    }
    mismatches = []
    for package, (version, revision) in expected.items():
        distribution = importlib.metadata.distribution(package)
        actual = distribution.version
        direct_url_text = distribution.read_text("direct_url.json")
        direct_url = json.loads(direct_url_text) if direct_url_text else {}
        commit_id = direct_url.get("vcs_info", {}).get("commit_id")
        if actual != version or commit_id != revision:
            mismatches.append(
                f"{package} version/revision {actual}/{commit_id or 'unattested'} "
                f"!= {version}/{revision}"
            )
    if mismatches:
        raise RuntimeError("pinned package mismatch: " + ", ".join(mismatches))


def main() -> int:
    if IMPORT_ERROR is not None:
        print(f"cannot generate LXMF propagation fixtures: {IMPORT_ERROR}", file=sys.stderr)
        return 2
    try:
        check_pins()
    except (RuntimeError, importlib.metadata.PackageNotFoundError, json.JSONDecodeError) as error:
        print(f"cannot generate LXMF propagation fixtures: {error}", file=sys.stderr)
        return 2
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    artifacts: list[dict] = []

    original_times = (router_module.time.time, message_module.time.time)
    original_random = (identity_module.os.urandom, x25519_module.os.urandom, token_module.os.urandom)
    original_register_destination = RNS.Transport.register_destination
    original_generate = identity_module.X25519PrivateKey.__dict__["generate"]
    try:
        router_module.time.time = lambda: FIXED_TIME
        message_module.time.time = lambda: FIXED_TIME
        entropy = DeterministicEntropy("lxmf-propagation-v1")
        identity_module.os.urandom = entropy
        x25519_module.os.urandom = entropy
        token_module.os.urandom = entropy
        identity_module.X25519PrivateKey.generate = classmethod(
            lambda cls: cls.from_private_bytes(entropy(32))
        )
        RNS.Transport.register_destination = lambda destination: None

        node_identity = identity("lxmf-propagation-node-v1")
        propagation_destination = RNS.Destination(
            node_identity, RNS.Destination.IN, RNS.Destination.SINGLE, APP_NAME, "propagation"
        )
        fake_router = type("FixtureRouter", (), {})()
        fake_router.name = "Styrene fixture propagation node"
        fake_router.propagation_node = True
        fake_router.from_static_only = False
        fake_router.propagation_stamp_cost = 16
        fake_router.propagation_stamp_cost_flexibility = 3
        fake_router.peering_cost = 18
        fake_router.propagation_per_transfer_limit = 256
        fake_router.propagation_per_sync_limit = 10240
        fake_router.get_propagation_node_announce_metadata = lambda: (
            LXMRouter.get_propagation_node_announce_metadata(fake_router)
        )
        app_data = LXMRouter.get_propagation_node_app_data(fake_router)
        announce = propagation_destination.announce(app_data=app_data, send=False)
        announce.pack()
        write_artifact("announce_app_data.msgpack", app_data, artifacts)
        write_artifact("announce_packet.bin", announce.raw, artifacts)

        sender_identity = identity("lxmf-propagation-sender-v1")
        recipient_identity = identity("lxmf-propagation-recipient-v1")
        sender = RNS.Destination(
            sender_identity, RNS.Destination.IN, RNS.Destination.SINGLE, APP_NAME, "delivery"
        )
        recipient = RNS.Destination(
            recipient_identity, RNS.Destination.OUT, RNS.Destination.SINGLE, APP_NAME, "delivery"
        )
        message = LXMessage(
            recipient,
            sender,
            title="Fixture title",
            content="Pinned Python propagation payload",
            fields={0x01: b"fixture-field"},
            desired_method=LXMessage.PROPAGATED,
        )
        message.pack()
        _, transient_messages = msgpack.unpackb(message.propagation_packed)
        lxm_data = transient_messages[0]
        transient_id = RNS.Identity.full_hash(lxm_data)
        propagation_stamp, propagation_stamp_value = deterministic_stamp(
            transient_id, 1, stamper.WORKBLOCK_EXPAND_ROUNDS_PN
        )
        stamped_payload = lxm_data + propagation_stamp
        transfer_envelope = msgpack.packb([FIXED_TIME, [stamped_payload]])
        decrypted = recipient_identity.decrypt(lxm_data[16:])
        if decrypted != message.packed[16:]:
            raise RuntimeError("propagation payload did not decrypt to the canonical LXMF body")
        write_artifact("encrypted_lxmf.bin", lxm_data, artifacts)
        write_artifact("plaintext_lxmf_body.bin", decrypted, artifacts)
        write_artifact("propagation_transfer.msgpack", transfer_envelope, artifacts)

        duplicate_id = hashlib.sha256(b"already-held-transient-id").digest()
        peering_id = node_identity.hash + sender_identity.hash
        peering_key, peering_key_value = deterministic_stamp(
            peering_id, 1, stamper.WORKBLOCK_EXPAND_ROUNDS_PEERING
        )
        request_payloads = {
            "offer_request.msgpack": [peering_key, [transient_id, duplicate_id]],
            "offer_accept_all.msgpack": True,
            "offer_accept_some.msgpack": [transient_id],
            "offer_accept_none.msgpack": False,
            "message_list_request.msgpack": [None, None],
            "message_get_request.msgpack": [[transient_id], [duplicate_id], 256],
            "message_list_response.msgpack": [transient_id, duplicate_id],
            "message_get_response.msgpack": [lxm_data],
            "authorization_no_identity.msgpack": LXMPeer.ERROR_NO_IDENTITY,
            "authorization_no_access.msgpack": LXMPeer.ERROR_NO_ACCESS,
        }
        for name, value in request_payloads.items():
            write_artifact(name, msgpack.packb(value), artifacts)

        index = {
            "schema_version": 1,
            "generator": "tests/interop/python/generate_lxmf_propagation_fixtures.py",
            "upstreams": {
                "rns": {"revision": RNS_REVISION, "version": "1.4.2"},
                "lxmf": {"revision": LXMF_REVISION, "version": "1.1.0"},
            },
            "fixed_unix_time": FIXED_TIME,
            "destination": {
                "name": "lxmf.propagation",
                "hash_hex": propagation_destination.hash.hex(),
                "private_key_hex": node_identity.get_private_key().hex(),
                "metadata_name_key": PN_META_NAME,
            },
            "request_paths": {
                "offer": {
                    "path": LXMPeer.OFFER_REQUEST_PATH,
                    "hash_hex": RNS.Identity.truncated_hash(LXMPeer.OFFER_REQUEST_PATH.encode()).hex(),
                },
                "get": {
                    "path": LXMPeer.MESSAGE_GET_PATH,
                    "hash_hex": RNS.Identity.truncated_hash(LXMPeer.MESSAGE_GET_PATH.encode()).hex(),
                },
            },
            "message": {
                "recipient_destination_hash_hex": recipient.hash.hex(),
                "recipient_private_key_hex": recipient_identity.get_private_key().hex(),
                "sender_destination_hash_hex": sender.hash.hex(),
                "transient_id_hex": transient_id.hex(),
                "duplicate_transient_id_hex": duplicate_id.hex(),
                "propagation_stamp_hex": propagation_stamp.hex(),
                "propagation_stamp_cost": 1,
                "propagation_stamp_value": propagation_stamp_value,
                "peering_key_hex": peering_key.hex(),
                "peering_key_cost": 1,
                "peering_key_value": peering_key_value,
            },
            "artifacts": artifacts,
        }
        index_bytes = (json.dumps(index, indent=2, sort_keys=True) + "\n").encode("utf-8")
        (OUTPUT_DIR / "index.json").write_bytes(index_bytes)
        print(f"wrote {len(artifacts)} LXMF propagation artifacts to {OUTPUT_DIR}")
    finally:
        router_module.time.time, message_module.time.time = original_times
        identity_module.os.urandom, x25519_module.os.urandom, token_module.os.urandom = original_random
        identity_module.X25519PrivateKey.generate = original_generate
        RNS.Transport.register_destination = original_register_destination
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
