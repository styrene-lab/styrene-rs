#!/usr/bin/env python3
"""Generate canonical NomadNet request and response fixtures.

The fixtures are produced by executing the pinned Python NomadNet node handlers
(`nomadnet.Node.Node.serve_page` and `serve_file`) and the pinned Python RNS
request packing (`RNS.Link.Link.request`, `handle_request`, and the
`RNS.Resource` metadata framing). The generator writes nothing unless both
pinned revisions are importable and attested, so hand-authored bytes can never
be presented as upstream fixtures.

Run with the pinned Git checkouts on `PYTHONPATH` (the same layout the live
interop workflow uses) or with the pinned packages installed from Git.
"""

from __future__ import annotations

import hashlib
import importlib.metadata
import json
import os
import stat
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

IMPORT_ERROR = None
try:
    import RNS
    import RNS.vendor.umsgpack as msgpack
    from nomadnet import Node
    from nomadnet._version import __version__ as NOMADNET_ACTUAL_VERSION

    node_module = sys.modules["nomadnet.Node"]
except ImportError as error:  # pragma: no cover - exercised only without pins
    IMPORT_ERROR = error

RNS_REVISION = "b48b96e61676504e0a4e527b33b9a0b4495c6872"
RNS_VERSION = "1.4.2"
NOMADNET_REVISION = "ad10301569a39d4f43b3d21ae9fc392602c937ca"
NOMADNET_VERSION = "1.2.3"
FIXED_TIME = 1_750_000_000.0
OUTPUT_DIR = Path(__file__).resolve().parent.parent / "fixtures" / "nomadnet-v1"
GENERATOR = "tests/interop/python/generate_nomadnet_fixtures.py"
AUTHORITY_NOMADNET = f"nomadnet-{NOMADNET_VERSION}"
AUTHORITY_RNS = f"rns-{RNS_VERSION}"

FIELDS = {"field_name": "rust", "var_mode": "safe"}
LINK_ID = hashlib.sha256(b"nomadnet-v1-link").digest()[:16]

PAGE_INDEX = b">Fixture Static Page\nServed by the pinned NomadNet node.\n"
PAGE_PRIVATE = b">Fixture Private Page\nOnly the allow-listed identity may read this.\n"
PAGE_SECRET = b">Fixture Secret Page\nNo identity is allowed here.\n"
PAGE_DYNAMIC = (
    b"#!/bin/sh\n"
    b"printf '>Fixture Dynamic Page\\nremote=%s\\nlink=%s\\nfield_name=%s\\nvar_mode=%s\\n' "
    b'"${remote_identity:-none}" "${link_id:-none}" "${field_name:-none}" "${var_mode:-none}"\n'
)


def deterministic_bytes(label: str, size: int) -> bytes:
    output = bytearray()
    counter = 0
    seed = label.encode("ascii")
    while len(output) < size:
        output.extend(hashlib.sha256(seed + counter.to_bytes(4, "big")).digest())
        counter += 1
    return bytes(output[:size])


def identity(label: str) -> "RNS.Identity":
    first = hashlib.sha256(label.encode("ascii")).digest()
    return RNS.Identity.from_bytes(first + hashlib.sha256(first).digest())


def module_revision(module) -> str | None:
    root = Path(module.__file__).resolve().parent.parent
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    revision = result.stdout.strip()
    return revision if len(revision) == 40 else None


def installed_revision(package: str) -> str | None:
    try:
        distribution = importlib.metadata.distribution(package)
    except importlib.metadata.PackageNotFoundError:
        return None
    direct_url_text = distribution.read_text("direct_url.json")
    direct_url = json.loads(direct_url_text) if direct_url_text else {}
    return direct_url.get("vcs_info", {}).get("commit_id")


def check_pins() -> None:
    import nomadnet

    checks = [
        ("rns", RNS, RNS.__version__, RNS_VERSION, RNS_REVISION),
        ("nomadnet", nomadnet, NOMADNET_ACTUAL_VERSION, NOMADNET_VERSION, NOMADNET_REVISION),
    ]
    mismatches = []
    for package, module, actual_version, version, revision in checks:
        actual_revision = module_revision(module) or installed_revision(package)
        if actual_version != version or actual_revision != revision:
            mismatches.append(
                f"{package} version/revision {actual_version}/{actual_revision or 'unattested'} "
                f"!= {version}/{revision}"
            )
    if mismatches:
        raise RuntimeError("pinned package mismatch: " + ", ".join(mismatches))


class StubApp:
    """The subset of `NomadNetworkApp` that the node request handlers touch."""

    def __init__(self, pagespath: str, filespath: str):
        self.pagespath = pagespath
        self.filespath = filespath
        self.peer_settings = {"served_page_requests": 0, "served_file_requests": 0}

    def save_peer_settings(self) -> None:
        return None


def write_tree(root: Path) -> tuple[str, str]:
    pages = root / "pages"
    files = root / "files"
    pages.mkdir()
    files.mkdir()
    (pages / "index.mu").write_bytes(PAGE_INDEX)
    (pages / "private.mu").write_bytes(PAGE_PRIVATE)
    (pages / "secret.mu").write_bytes(PAGE_SECRET)
    (pages / "dynamic.mu").write_bytes(PAGE_DYNAMIC)
    (pages / "dynamic.mu").chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH)
    (files / "manual.bin").write_bytes(deterministic_bytes("nomadnet-v1-manual", 3000))
    return str(pages), str(files)


def packed_request(path: str, data) -> tuple[bytes, bytes, bytes]:
    # Mirrors RNS.Link.Link.request: [requested_at, path_hash, data] and the
    # request id as the truncated hash of the packed request.
    path_hash = RNS.Identity.truncated_hash(path.encode("utf-8"))
    packed = msgpack.packb([FIXED_TIME, path_hash, data])
    return packed, path_hash, RNS.Identity.truncated_hash(packed)


def main() -> int:
    if IMPORT_ERROR is not None:
        print(f"cannot generate NomadNet fixtures: {IMPORT_ERROR}", file=sys.stderr)
        return 2
    try:
        check_pins()
    except RuntimeError as error:
        print(f"cannot generate NomadNet fixtures: {error}", file=sys.stderr)
        return 2
    if RNS.vendor.platformutils.is_windows():
        print("cannot generate NomadNet fixtures on Windows: dynamic pages need executables", file=sys.stderr)
        return 2

    RNS.loglevel = 0
    allowed = identity("nomadnet-v1-allowed-reader")
    denied = identity("nomadnet-v1-denied-reader")
    artifacts: dict[str, bytes] = {}
    vectors: list[dict] = []

    def add(
        vector_id: str,
        artifact_name: str,
        payload: bytes,
        *,
        authority: str,
        kind: str,
        source_symbols: list[str],
        expected: dict,
    ) -> None:
        artifacts[artifact_name] = payload
        vectors.append(
            {
                "id": vector_id,
                "authority_id": authority,
                "kind": kind,
                "artifact": f"tests/interop/fixtures/nomadnet-v1/{artifact_name}",
                "sha256": hashlib.sha256(payload).hexdigest(),
                "generator": GENERATOR,
                "source_symbols": source_symbols,
                "expected": expected,
            }
        )

    with tempfile.TemporaryDirectory(prefix="nomadnet-fixtures-") as tmp:
        root = Path(tmp)
        pagespath, filespath = write_tree(root)
        allowed_private = (allowed.hash.hex() + "\n").encode("ascii")
        allowed_secret = ("00" * 16 + "\n").encode("ascii")
        (root / "pages" / "private.mu.allowed").write_bytes(allowed_private)
        (root / "pages" / "secret.mu.allowed").write_bytes(allowed_secret)

        node = Node.__new__(Node)
        node.app = StubApp(pagespath, filespath)

        page_sources = {
            "page_index.mu": (PAGE_INDEX, False),
            "page_private.mu": (PAGE_PRIVATE, False),
            "page_secret.mu": (PAGE_SECRET, False),
            "page_dynamic.mu": (PAGE_DYNAMIC, True),
            "allowed_private.txt": (allowed_private, False),
            "allowed_secret.txt": (allowed_secret, False),
        }
        for name, (payload, executable) in page_sources.items():
            add(
                f"nomadnet-source-{name}",
                name,
                payload,
                authority=AUTHORITY_NOMADNET,
                kind="page-source",
                source_symbols=["nomadnet.Node.Node.register_pages", "nomadnet.Node.Node.scan_pages"],
                expected={"type": "page-source", "executable": executable},
            )

        requests = {
            "index": ("/page/index.mu", None),
            "dynamic": ("/page/dynamic.mu", FIELDS),
            "private": ("/page/private.mu", None),
            "secret": ("/page/secret.mu", None),
            "file": ("/file/manual.bin", None),
        }
        request_ids: dict[str, bytes] = {}
        for label, (path, data) in requests.items():
            packed, path_hash, request_id = packed_request(path, data)
            request_ids[label] = request_id
            add(
                f"rns-request-{label}",
                f"request_{label}.msgpack",
                packed,
                authority=AUTHORITY_RNS,
                kind="request-envelope",
                source_symbols=["RNS.Link.Link.request", "RNS.Identity.Identity.truncated_hash"],
                expected={
                    "type": "request-envelope",
                    "path": path,
                    "path_hash_hex": path_hash.hex(),
                    "requested_at": FIXED_TIME,
                    "data_hex": msgpack.packb(data).hex(),
                    "request_id_hex": request_id.hex(),
                },
            )

        def serve(path: str, data, remote, label: str) -> bytes:
            response = node.serve_page(path, data, request_ids[label], LINK_ID, remote, FIXED_TIME)
            if not isinstance(response, bytes):
                raise RuntimeError(f"serve_page returned {type(response)!r} for {path}")
            return response

        static_response = serve("/page/index.mu", None, allowed, "index")
        add(
            "nomadnet-response-index",
            "response_index.bin",
            static_response,
            authority=AUTHORITY_NOMADNET,
            kind="page-response",
            source_symbols=["nomadnet.Node.Node.serve_page"],
            expected={"type": "page-response", "path": "/page/index.mu", "access": "public"},
        )
        add(
            "rns-response-envelope-index",
            "response_envelope_index.msgpack",
            msgpack.packb([request_ids["index"], static_response]),
            authority=AUTHORITY_RNS,
            kind="response-envelope",
            source_symbols=["RNS.Link.Link.handle_request"],
            expected={
                "type": "response-envelope",
                "request_id_hex": request_ids["index"].hex(),
                "response_artifact": "response_index.bin",
            },
        )

        dynamic_response = serve("/page/dynamic.mu", FIELDS, allowed, "dynamic")
        add(
            "nomadnet-response-dynamic",
            "response_dynamic.bin",
            dynamic_response,
            authority=AUTHORITY_NOMADNET,
            kind="page-response",
            source_symbols=["nomadnet.Node.Node.serve_page"],
            expected={
                "type": "page-response",
                "path": "/page/dynamic.mu",
                "access": "public",
                "remote_identity_hex": allowed.hash.hex(),
                "link_id_hex": LINK_ID.hex(),
                "environment": {
                    "remote_identity": allowed.hash.hex(),
                    "link_id": LINK_ID.hex(),
                    **FIELDS,
                },
            },
        )
        add(
            "nomadnet-response-private-allowed",
            "response_private_allowed.bin",
            serve("/page/private.mu", None, allowed, "private"),
            authority=AUTHORITY_NOMADNET,
            kind="page-response",
            source_symbols=["nomadnet.Node.Node.serve_page"],
            expected={
                "type": "page-response",
                "path": "/page/private.mu",
                "access": "allowed",
                "remote_identity_hex": allowed.hash.hex(),
            },
        )
        not_allowed = node_module.DEFAULT_NOTALLOWED.encode("utf-8")
        for label, path, remote, remote_hex in (
            ("private-denied", "/page/private.mu", denied, denied.hash.hex()),
            ("private-anonymous", "/page/private.mu", None, None),
            ("secret-denied", "/page/secret.mu", allowed, allowed.hash.hex()),
        ):
            response = serve(path, None, remote, path.rsplit("/", 1)[1].split(".")[0])
            if response != not_allowed:
                raise RuntimeError(f"expected the denial page for {label}")
            add(
                f"nomadnet-response-{label}",
                f"response_{label.replace('-', '_')}.bin",
                response,
                authority=AUTHORITY_NOMADNET,
                kind="page-response",
                source_symbols=["nomadnet.Node.Node.serve_page", "nomadnet.Node.DEFAULT_NOTALLOWED"],
                expected={
                    "type": "page-response",
                    "path": path,
                    "access": "denied",
                    "remote_identity_hex": remote_hex,
                },
            )

        file_response = node.serve_file("/file/manual.bin", None, request_ids["file"], allowed, FIXED_TIME)
        if not (isinstance(file_response, list) and len(file_response) == 2):
            raise RuntimeError("serve_file did not return a [handle, metadata] pair")
        handle, metadata = file_response
        with handle:
            file_data = handle.read()
        packed_metadata = msgpack.packb(metadata)
        # Mirrors RNS.Resource.Resource.__init__: 3-byte big-endian metadata
        # size followed by the packed metadata, prefixed to the resource data.
        metadata_prefix = struct.pack(">I", len(packed_metadata))[1:] + packed_metadata
        add(
            "nomadnet-file-manual",
            "file_manual.bin",
            file_data,
            authority=AUTHORITY_NOMADNET,
            kind="file-response",
            source_symbols=["nomadnet.Node.Node.serve_file"],
            expected={
                "type": "file-response",
                "path": "/file/manual.bin",
                "name": metadata["name"].decode("utf-8"),
                "metadata_artifact": "file_metadata.msgpack",
            },
        )
        add(
            "rns-file-metadata",
            "file_metadata.msgpack",
            packed_metadata,
            authority=AUTHORITY_RNS,
            kind="resource-metadata",
            source_symbols=["RNS.Link.Link.handle_request", "RNS.Resource.Resource.__init__"],
            expected={
                "type": "resource-metadata",
                "name": metadata["name"].decode("utf-8"),
                "resource_prefix_hex": metadata_prefix.hex(),
                "raw_data_artifact": "file_manual.bin",
            },
        )

    index = {
        "schema_version": 2,
        "authorities": {
            AUTHORITY_NOMADNET: {
                "repository": "https://github.com/markqvist/NomadNet.git",
                "revision": NOMADNET_REVISION,
                "release": NOMADNET_VERSION,
            },
            AUTHORITY_RNS: {
                "repository": "https://github.com/markqvist/Reticulum.git",
                "revision": RNS_REVISION,
                "release": RNS_VERSION,
            },
        },
        "parameters": {
            "requested_at": FIXED_TIME,
            "link_id_hex": LINK_ID.hex(),
            "fields": FIELDS,
            "identities": {
                "allowed": {
                    "public_key_hex": allowed.get_public_key().hex(),
                    "hash_hex": allowed.hash.hex(),
                },
                "denied": {
                    "public_key_hex": denied.get_public_key().hex(),
                    "hash_hex": denied.hash.hex(),
                },
            },
            "pages": [
                {"request_path": "/page/index.mu", "source": "page_index.mu", "allowed": None},
                {"request_path": "/page/private.mu", "source": "page_private.mu", "allowed": "allowed_private.txt"},
                {"request_path": "/page/secret.mu", "source": "page_secret.mu", "allowed": "allowed_secret.txt"},
                {"request_path": "/page/dynamic.mu", "source": "page_dynamic.mu", "allowed": None},
            ],
            "files": [{"request_path": "/file/manual.bin", "source": "file_manual.bin"}],
        },
        "vectors": vectors,
    }

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    for stale in OUTPUT_DIR.iterdir():
        if stale.is_file():
            stale.unlink()
    for name, payload in artifacts.items():
        (OUTPUT_DIR / name).write_bytes(payload)
    (OUTPUT_DIR / "index.json").write_text(json.dumps(index, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {len(artifacts)} NomadNet fixtures to {OUTPUT_DIR}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
