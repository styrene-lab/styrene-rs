#!/usr/bin/env python3
"""Generate the bounded canonical RNS 1.5.1 seed fixture set."""

from __future__ import annotations

import argparse
import importlib
import json
import pathlib
import subprocess
import sys
from types import SimpleNamespace

REVISION = "149e4151095adf098b8f53eab0c03b37169e8559"
TYPE1 = bytes.fromhex("017f000102030405060708090a0b0c0d0e0f00a5")
TYPE2 = bytes.fromhex(
    "417f101112131415161718191a1b1c1d1e1f000102030405060708090a0b0c0d0e0f00a5"
)
TOKEN_KEY = bytes(range(64))
TOKEN_IV = bytes(range(0xA0, 0xB0))
TOKEN_PLAINTEXT = b"RNS 1.5.1 token"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reticulum-checkout", required=True, type=pathlib.Path)
    parser.add_argument(
        "--output",
        type=pathlib.Path,
        default=pathlib.Path("tests/interop/fixtures/rns/rns-1.5.1"),
    )
    args = parser.parse_args()
    revision = subprocess.run(
        ["git", "-C", str(args.reticulum_checkout), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if revision != REVISION:
        raise SystemExit(f"expected Reticulum {REVISION}, got {revision}")

    sys.path.insert(0, str(args.reticulum_checkout))
    import RNS  # type: ignore[import-not-found]  # noqa: PLC0415
    channel_module = importlib.import_module("RNS.Channel")
    discovery_module = importlib.import_module("RNS.Discovery")
    resource_module = importlib.import_module("RNS.Resource")
    token_module = importlib.import_module("RNS.Cryptography.Token")
    msgpack = importlib.import_module("RNS.vendor.umsgpack")

    for raw in (TYPE1, TYPE2):
        packet = RNS.Packet(None, raw)
        if packet.unpack() is not True:
            raise SystemExit("canonical RNS rejected a seed packet")

    mutations = [
        ("type1-incomplete-destination", TYPE1[:17], "malformed_packet"),
        ("type1-zero-data", TYPE1[:19], "empty_data"),
        ("hop-128", TYPE1[:1] + bytes([128]) + TYPE1[2:], "excessive_hops"),
        ("hop-255", TYPE1[:1] + bytes([255]) + TYPE1[2:], "excessive_hops"),
        ("type2-incomplete-transport", TYPE2[:17], "malformed_packet"),
        ("type2-incomplete-destination", TYPE2[:34], "malformed_packet"),
        ("type2-zero-data", TYPE2[:35], "empty_data"),
    ]
    admission = []
    for case_id, raw, rejection in mutations:
        if RNS.Packet(None, raw).unpack() is not False:
            raise SystemExit(f"canonical RNS unexpectedly accepted {case_id}")
        admission.append(
            {"id": case_id, "raw_hex": raw.hex(), "accepted": False, "class": rejection}
        )

    original_urandom = token_module.os.urandom
    token_module.os.urandom = lambda length: TOKEN_IV if length == len(TOKEN_IV) else original_urandom(length)
    try:
        token = token_module.Token(TOKEN_KEY)
        encrypted = token.encrypt(TOKEN_PLAINTEXT)
        if token.decrypt(encrypted) != TOKEN_PLAINTEXT:
            raise SystemExit("canonical RNS token round trip failed")
    finally:
        token_module.os.urandom = original_urandom

    bitrate_cases = [
        None,
        0,
        4,
        5,
        6,
        9,
        100,
        500,
        1000,
        62500,
        2243903,
        (1 << 64) - 1,
    ]
    interface_cases = [
        [{"online": False, "bitrate": 5}, {"online": True, "bitrate": 1000}],
        [{"online": True, "bitrate": 500}, {"online": True, "bitrate": 1000}],
        [
            {"online": True, "bitrate": None},
            {"online": True, "bitrate": 0},
            {"online": True, "bitrate": 1000},
        ],
        [{"online": False, "bitrate": 5}, {"online": True, "bitrate": 0}],
    ]
    original_interfaces = RNS.Transport.interfaces
    original_lowest_bitrate = RNS.Transport.lowest_interface_bitrate
    original_highest_bitrate = RNS.Transport.highest_interface_bitrate
    try:
        online_bitrate_selection = []
        for interfaces in interface_cases:
            RNS.Transport.interfaces = [SimpleNamespace(**interface) for interface in interfaces]
            RNS.Transport.lowest_interface_bitrate = None
            RNS.Transport.highest_interface_bitrate = None
            RNS.Transport.prioritize_interfaces()
            online_bitrate_selection.append(
                {
                    "interfaces": interfaces,
                    "expected_lowest": RNS.Transport.lowest_interface_bitrate,
                }
            )

        medium_path_grace = []
        for bitrate in bitrate_cases:
            RNS.Transport.lowest_interface_bitrate = bitrate
            seconds = RNS.Transport.medium_path_timeout()
            medium_path_grace.append(
                {"bitrate": bitrate, "expected_nanos": round(seconds * 1_000_000_000)}
            )
    finally:
        RNS.Transport.interfaces = original_interfaces
        RNS.Transport.lowest_interface_bitrate = original_lowest_bitrate
        RNS.Transport.highest_interface_bitrate = original_highest_bitrate

    link_proof_extra_grace = []
    for bitrate in bitrate_cases:
        interface = None if bitrate is None else SimpleNamespace(bitrate=bitrate)
        seconds = RNS.Transport.extra_link_proof_timeout(interface)
        link_proof_extra_grace.append(
            {"bitrate": bitrate, "expected_nanos": round(seconds * 1_000_000_000)}
        )

    destination_identity = RNS.Identity.from_bytes(bytes(range(64)))
    initiator_identity = RNS.Identity.from_bytes(bytes(range(64, 128)))
    link_id = bytes(range(0xA0, 0xB0))
    link_mtu_vectors = []
    for mtu in (500, 1024, 1280, 2048):
        signalling = RNS.Link.signalling_bytes(mtu, RNS.Link.MODE_DEFAULT)
        request_payload = initiator_identity.get_public_key() + signalling
        signed_data = (
            link_id
            + destination_identity.pub_bytes
            + destination_identity.sig_pub_bytes
            + signalling
        )
        proof_payload = (
            destination_identity.sign(signed_data)
            + destination_identity.pub_bytes
            + signalling
        )
        request = SimpleNamespace(data=request_payload)
        proof = SimpleNamespace(data=proof_payload)

        link = object.__new__(RNS.Link)
        link.mtu = mtu
        link.rtt = 0
        link.update_mdu()
        channel = channel_module.Channel(channel_module.LinkChannelOutlet(link))
        resource_link = SimpleNamespace(
            mtu=mtu,
            mdu=link.mdu,
            rtt=1.0,
            traffic_timeout_factor=6,
        )
        resource = resource_module.Resource(None, resource_link, advertise=False)

        link_mtu_vectors.append(
            {
                "mtu": mtu,
                "signalling_hex": signalling.hex(),
                "request_payload_hex": request_payload.hex(),
                "request_length": len(request_payload),
                "proof_payload_hex": proof_payload.hex(),
                "proof_length": len(proof_payload),
                "proof_signed_data_hex": signed_data.hex(),
                "destination_public_key_hex": destination_identity.get_public_key().hex(),
                "link_id_hex": link_id.hex(),
                "decoded_request_mtu": RNS.Link.mtu_from_lr_packet(request),
                "decoded_proof_mtu": RNS.Link.mtu_from_lp_packet(proof),
                "mode": RNS.Link.mode_from_lr_packet(request),
                "packet_mdu": link.mdu,
                "channel_mdu": channel.mdu,
                "resource_sdu": resource.sdu,
            }
        )

    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "packet-type1-hop127.bin").write_bytes(TYPE1)
    (args.output / "packet-type2-hop127.bin").write_bytes(TYPE2)
    (args.output / "token-valid.bin").write_bytes(encrypted)
    invalid_token = encrypted[:-1] + bytes([encrypted[-1] ^ 0x01])
    (args.output / "token-invalid-tag.bin").write_bytes(invalid_token)
    (args.output / "token-truncated-tag.bin").write_bytes(encrypted[:-1])
    discovery_info = {
        discovery_module.INTERFACE_TYPE: "TCPServerInterface",
        discovery_module.TRANSPORT: True,
        discovery_module.TRANSPORT_ID: bytes(range(16)),
        discovery_module.TRANSPORT_IMPL: discovery_module.IMPLEMENTATION_NAME,
        discovery_module.TRANSPORT_VERS: discovery_module.RNS_VERSION,
        discovery_module.NAME: "Relay One",
        discovery_module.LATITUDE: None,
        discovery_module.LONGITUDE: None,
        discovery_module.HEIGHT: None,
        discovery_module.OP_ADDR: bytes(range(0x10, 0x20)),
        discovery_module.REACHABLE_ON: "relay.example",
        discovery_module.PORT: 4242,
    }
    discovery_cases = []
    for case_id, updates, remove, accepted in [
        ("valid-operator", {}, [], True),
        ("absent-operator", {}, [discovery_module.OP_ADDR], True),
        ("operator-wrong-type", {discovery_module.OP_ADDR: "not-bytes"}, [], False),
        ("operator-wrong-length", {discovery_module.OP_ADDR: bytes(15)}, [], False),
        ("transport-id-wrong-length", {discovery_module.TRANSPORT_ID: bytes(15)}, [], False),
        ("transport-wrong-type", {discovery_module.TRANSPORT: 1}, [], False),
        ("implementation-wrong-type", {discovery_module.TRANSPORT_IMPL: 1}, [], False),
        ("name-wrong-type", {discovery_module.NAME: 1}, [], False),
    ]:
        info = dict(discovery_info)
        info.update(updates)
        for key in remove:
            info.pop(key)
        discovery_cases.append(
            {"id": case_id, "packed_hex": msgpack.packb(info).hex(), "accepted": accepted}
        )
    (args.output / "interface-discovery-vectors.json").write_text(
        json.dumps(discovery_cases, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    (args.output / "packet-admission.json").write_text(
        json.dumps(admission, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    (args.output / "bitrate-deadlines.json").write_text(
        json.dumps(
            {
                "medium_path_grace": medium_path_grace,
                "link_proof_extra_grace": link_proof_extra_grace,
                "online_bitrate_selection": online_bitrate_selection,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    (args.output / "link-mtu-vectors.json").write_text(
        json.dumps(
            {
                "disabled_discovery": {
                    "request_length": RNS.Link.ECPUBSIZE + RNS.Link.LINK_MTU_SIZE,
                    "signalling_hex": RNS.Link.signalling_bytes(
                        RNS.Reticulum.MTU, RNS.Link.MODE_DEFAULT
                    ).hex(),
                },
                "unsupported_forwarding": {
                    "request_length_after_stripping": RNS.Link.ECPUBSIZE,
                    "confirmed_mtu": RNS.Reticulum.MTU,
                },
                "vectors": link_mtu_vectors,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
