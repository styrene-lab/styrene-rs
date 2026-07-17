#!/usr/bin/env python3
"""Populate an ext4 image with regular files, directories, and symlinks.

This deliberately does not read or copy host extended attributes. It is used
when the build root lives on virtiofs, where synthetic security.selinux xattrs
can be listed but not read by mke2fs -d.
"""
from __future__ import annotations

import argparse
import os
import subprocess
from pathlib import Path


def run(image: Path, command: str) -> None:
    subprocess.run(["debugfs", "-w", "-R", command, str(image)], check=True)


def quote(value: str) -> str:
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("image", type=Path)
    args = parser.parse_args()
    source = args.source
    image = args.image

    for root, dirs, files in os.walk(source, topdown=True, followlinks=False):
        root_path = Path(root)
        relative_root = root_path.relative_to(source)
        destination_root = "/" if relative_root == Path(".") else f"/{relative_root.as_posix()}"

        symlink_dirs = [name for name in dirs if (root_path / name).is_symlink()]
        dirs[:] = [name for name in dirs if name not in symlink_dirs]

        for name in dirs:
            run(image, f"mkdir {quote(destination_root.rstrip('/') + '/' + name)}")
        for name in symlink_dirs:
            path = root_path / name
            destination = destination_root.rstrip("/") + "/" + name
            run(image, f"symlink {quote(destination)} {quote(os.readlink(path))}")
        for name in files:
            path = root_path / name
            destination = destination_root.rstrip("/") + "/" + name
            if path.is_symlink():
                run(image, f"symlink {quote(destination)} {quote(os.readlink(path))}")
            elif path.is_file():
                run(image, f"write {quote(str(path))} {quote(destination)}")
            else:
                raise RuntimeError(f"unsupported staged file type: {path}")

    for root, dirs, files in os.walk(source, topdown=False, followlinks=False):
        root_path = Path(root)
        for name in files + dirs:
            path = root_path / name
            if path.is_symlink():
                continue
            mode = path.lstat().st_mode
            relative = path.relative_to(source).as_posix()
            # debugfs replaces the complete inode mode field. Preserve the file
            # type bits as well as permissions; writing only 0755 turns a
            # directory inode into an invalid untyped inode.
            run(image, f"set_inode_field {quote('/' + relative)} mode 0{mode:o}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
