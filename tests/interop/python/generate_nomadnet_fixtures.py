#!/usr/bin/env python3
"""Fail-closed entry point for canonical pinned NomadNet fixture generation.

The generator intentionally writes nothing until the exact pinned NomadNet
package is importable. This prevents hand-authored bytes from being presented
as upstream fixtures when the executable reference is unavailable.
"""

from __future__ import annotations

import importlib.metadata
import json
import sys


NOMADNET_REVISION = "ad10301569a39d4f43b3d21ae9fc392602c937ca"
NOMADNET_VERSION = "1.2.8"


def main() -> int:
    try:
        distribution = importlib.metadata.distribution("nomadnet")
        actual = distribution.version
        import NomadNet  # noqa: F401
    except (importlib.metadata.PackageNotFoundError, ImportError):
        print(
            "cannot generate NomadNet fixtures: pinned nomadnet 1.2.8 "
            f"({NOMADNET_REVISION}) is not installed",
            file=sys.stderr,
        )
        return 2

    if actual != NOMADNET_VERSION:
        print(
            f"cannot generate NomadNet fixtures: nomadnet {actual} is available, "
            f"but {NOMADNET_VERSION} is required",
            file=sys.stderr,
        )
        return 2

    direct_url_text = distribution.read_text("direct_url.json")
    direct_url = json.loads(direct_url_text) if direct_url_text else {}
    commit_id = direct_url.get("vcs_info", {}).get("commit_id")
    if commit_id != NOMADNET_REVISION:
        print(
            f"cannot generate NomadNet fixtures: revision is {commit_id or 'unattested'}, "
            f"but {NOMADNET_REVISION} is required",
            file=sys.stderr,
        )
        return 2

    print(
        "pinned NomadNet is available, but its request handlers have not been inspected; "
        "refusing to fabricate page, field, file, or authorization bytes",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
