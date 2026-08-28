#!/usr/bin/env python3
"""Enforce Styrene workspace layering and publication boundaries."""

from __future__ import annotations

import json
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path
from typing import Any

ALLOWED_DEPENDENCY_LAYERS = {
    "foundation": {"foundation"},
    "protocol": {"foundation", "protocol"},
    "domain": {"foundation", "protocol", "domain"},
    "interface": {"foundation", "protocol", "domain", "interface"},
    "runtime": {"foundation", "protocol", "domain", "interface", "runtime"},
    "application": {
        "foundation",
        "protocol",
        "domain",
        "interface",
        "runtime",
        "application",
        "tooling",
    },
    "binding": {"foundation", "protocol", "domain", "interface", "runtime"},
    "tooling": {"foundation", "protocol", "domain", "interface", "runtime", "tooling"},
    "test": {
        "foundation",
        "protocol",
        "domain",
        "interface",
        "runtime",
        "application",
        "binding",
        "tooling",
        "test",
    },
}


def load_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as source:
        return tomllib.load(source)


def cargo_metadata(root: Path) -> dict[str, Any]:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "cargo metadata failed")
    return json.loads(result.stdout)


def validate(root: Path, metadata: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    root_manifest = load_toml(root / "Cargo.toml")
    workspace = root_manifest["workspace"]
    workspace_package = workspace["package"]
    expected_edition = workspace_package["edition"]
    expected_rust_version = workspace_package["rust-version"]
    if workspace.get("resolver") != "3":
        errors.append("workspace: resolver must be 3")
    packages = {package["name"]: package for package in metadata["packages"]}
    package_paths = {
        Path(package["manifest_path"]).parent.resolve(): package["name"]
        for package in metadata["packages"]
    }
    policy = metadata.get("metadata", {}).get("styrene", {})
    public = set(policy.get("public-crates", []))
    automated_release = set(policy.get("automated-release-crates", []))
    declared_layers = policy.get("layers", {})
    crate_layers: dict[str, str] = {}

    unknown_layers = set(declared_layers) - set(ALLOWED_DEPENDENCY_LAYERS)
    for layer in sorted(unknown_layers):
        errors.append(f"unknown architectural layer {layer!r}")

    for layer, crate_names in declared_layers.items():
        for crate_name in crate_names:
            previous = crate_layers.get(crate_name)
            if previous is not None:
                errors.append(f"{crate_name}: assigned more than once ({previous}, {layer})")
            else:
                crate_layers[crate_name] = layer

    for crate_name in sorted(set(packages) - set(crate_layers)):
        errors.append(f"{crate_name}: missing from workspace.metadata.styrene.layers")
    for crate_name in sorted(set(crate_layers) - set(packages)):
        errors.append(f"{crate_name}: catalogued but not present in the workspace")
    for crate_name in sorted(public - set(packages)):
        errors.append(f"{crate_name}: public crate is not present in the workspace")
    for crate_name in sorted(automated_release - public):
        errors.append(f"{crate_name}: automated release crate is not public")

    for crate_name, package in sorted(packages.items()):
        if package.get("edition") != expected_edition:
            errors.append(
                f"{crate_name}: edition {package.get('edition')!r} != workspace edition {expected_edition!r}"
            )
        if package.get("rust_version") != expected_rust_version:
            errors.append(
                f"{crate_name}: rust-version {package.get('rust_version')!r} != "
                f"workspace rust-version {expected_rust_version!r}"
            )
        publish_registries = package.get("publish")
        is_publishable = publish_registries is None or "crates-io" in publish_registries
        if crate_name in public and not is_publishable:
            errors.append(f"{crate_name}: public crate must publish to crates.io")
        if crate_name not in public and is_publishable:
            errors.append(f"{crate_name}: internal crate must set publish = false")

        source_layer = crate_layers.get(crate_name)
        if source_layer not in ALLOWED_DEPENDENCY_LAYERS:
            continue
        workspace_dependencies = []
        for dependency in package["dependencies"]:
            dependency_path = dependency.get("path")
            if dependency.get("kind") == "dev":
                continue
            if dependency_path is None:
                if dependency["name"] in packages:
                    errors.append(
                        f"{crate_name}: workspace package {dependency['name']} must use a local path dependency"
                    )
                continue
            resolved_path = Path(dependency_path)
            if not resolved_path.is_absolute():
                resolved_path = root / resolved_path
            dependency_name = package_paths.get(resolved_path.resolve())
            if dependency_name is None:
                errors.append(
                    f"{crate_name}: local path dependency {dependency['name']} is not a workspace member"
                )
                continue
            workspace_dependencies.append((dependency, dependency_name))

        dependency_counts = Counter(name for _, name in workspace_dependencies)
        for dependency_name, count in sorted(dependency_counts.items()):
            if count > 1:
                errors.append(
                    f"{crate_name}: dependency {dependency_name} is declared through multiple aliases"
                )

        for dependency, dependency_name in workspace_dependencies:
            target_layer = crate_layers.get(dependency_name)
            if target_layer not in ALLOWED_DEPENDENCY_LAYERS[source_layer]:
                errors.append(
                    f"{crate_name} ({source_layer}) must not depend on "
                    f"{dependency_name} ({target_layer})"
                )
            if crate_name in public:
                if dependency_name not in public:
                    errors.append(
                        f"{crate_name}: public crate depends on internal crate {dependency_name}"
                    )
                expected_requirement = f"^{packages[dependency_name]['version']}"
                if dependency.get("req") != expected_requirement:
                    errors.append(
                        f"{crate_name}: workspace path dependency {dependency_name} requirement "
                        f"{dependency.get('req')!r} != {expected_requirement!r}"
                    )

    workspace_dependencies = root_manifest["workspace"].get("dependencies", {})
    for dependency_key, specification in sorted(workspace_dependencies.items()):
        if not isinstance(specification, dict) or "path" not in specification:
            continue
        package_name = specification.get("package", dependency_key)
        package = packages.get(package_name)
        if package is None:
            errors.append(f"workspace dependency {dependency_key}: path package is not a member")
            continue
        if specification.get("version") != package["version"]:
            errors.append(
                f"workspace dependency {dependency_key}: version "
                f"{specification.get('version')!r} != package version {package['version']}"
            )

    release_config = load_toml(root / "release-plz.toml")
    release_packages = release_config.get("package", [])
    release_names = [package_policy["name"] for package_policy in release_packages]
    for crate_name, count in sorted(Counter(release_names).items()):
        if count > 1:
            errors.append(f"{crate_name}: duplicate release-plz package entry")
        if crate_name not in packages:
            errors.append(f"{crate_name}: release-plz package is not present in the workspace")

    enabled_release = set()
    for package_policy in release_packages:
        crate_name = package_policy["name"]
        enabled = package_policy.get("publish") is True and package_policy.get("release") is True
        if enabled:
            enabled_release.add(crate_name)
        if enabled and crate_name not in public:
            errors.append(f"{crate_name}: release-plz enables a crate not in public-crates")
    for crate_name in sorted(automated_release - enabled_release):
        errors.append(f"{crate_name}: automated release crate is not enabled in release-plz")
    for crate_name in sorted(enabled_release - automated_release):
        errors.append(f"{crate_name}: release-plz enables an unlisted automated release crate")

    return errors


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    try:
        errors = validate(root, cargo_metadata(root))
    except (OSError, RuntimeError, tomllib.TOMLDecodeError, json.JSONDecodeError) as error:
        print(f"workspace policy check failed: {error}", file=sys.stderr)
        return 2
    if errors:
        print("workspace policy check failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    print("workspace policy check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
