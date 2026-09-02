#!/usr/bin/env python3
"""Reject automated attribution and model-generated boilerplate."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


PROHIBITED_NAMES = (
    "anth" + "ropic",
    "clau" + "de",
)
AUTOMATION_NAMES = PROHIBITED_NAMES + (
    "chat" + "gpt",
    "co" + "pilot",
    "ge" + "mini",
    "co" + "dex",
)
PROHIBITED_NAME_PATTERN = "|".join(re.escape(name) for name in PROHIBITED_NAMES)
AUTOMATION_NAME_PATTERN = "|".join(re.escape(name) for name in AUTOMATION_NAMES)
PROHIBITED_PATTERNS = (
    (
        "prohibited assistant or vendor name",
        re.compile(rf"\b(?:{PROHIBITED_NAME_PATTERN})\b", re.IGNORECASE),
    ),
    (
        "model-attribution boilerplate",
        re.compile(
            r"\b(?:generated\s+with|as\s+an\s+(?:ai|artificial\s+intelligence)\s+language\s+model|"
            r"(?:ai|llm)[ -]generated)\b",
            re.IGNORECASE,
        ),
    ),
    (
        "non-human attribution trailer",
        re.compile(
            rf"(?:co-authored-by|authored-by|signed-off-by|generated\s+by|written\s+by|assisted\s+by)"
            rf".*(?:\[bot\]|\bbot@|\b(?:ai|llm|{AUTOMATION_NAME_PATTERN})\b)",
            re.IGNORECASE,
        ),
    ),
)


@dataclass(frozen=True)
class SourceLine:
    source: str
    number: int
    text: str


@dataclass(frozen=True)
class Violation:
    source: str
    number: int
    reason: str
    text: str


def find_violations(lines: Iterable[SourceLine]) -> list[Violation]:
    violations = []
    for line in lines:
        for reason, pattern in PROHIBITED_PATTERNS:
            if pattern.search(line.text):
                violations.append(Violation(line.source, line.number, reason, line.text))
                break
    return violations


def added_lines(diff: str) -> Iterable[SourceLine]:
    source = "<staged change>"
    new_line = 0
    in_hunk = False
    for raw_line in diff.splitlines():
        if raw_line.startswith("diff --git "):
            in_hunk = False
            continue
        if not in_hunk and raw_line.startswith("+++ "):
            source = raw_line[4:]
            if source.startswith("b/"):
                source = source[2:]
            continue
        if raw_line.startswith("@@ "):
            match = re.search(r"\+(\d+)", raw_line)
            if match is not None:
                new_line = int(match.group(1))
                in_hunk = True
            continue
        if in_hunk and raw_line.startswith("+"):
            yield SourceLine(source, new_line, raw_line[1:])
            new_line += 1
        elif in_hunk and raw_line.startswith(" "):
            new_line += 1


def staged_lines() -> Iterable[SourceLine]:
    result = subprocess.run(
        [
            "git",
            "diff",
            "--cached",
            "--no-ext-diff",
            "--no-color",
            "--unified=0",
            "--diff-filter=ACMR",
        ],
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "could not inspect staged changes")
    return added_lines(result.stdout)


def message_lines(path: Path) -> Iterable[SourceLine]:
    with path.open(encoding="utf-8", errors="replace") as message:
        for number, text in enumerate(message, start=1):
            yield SourceLine(str(path), number, text.rstrip("\n"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    inputs = parser.add_mutually_exclusive_group(required=True)
    inputs.add_argument("--staged", action="store_true", help="check added lines in the staged diff")
    inputs.add_argument("--commit-message", type=Path, metavar="PATH", help="check a commit message")
    arguments = parser.parse_args()

    try:
        lines = staged_lines() if arguments.staged else message_lines(arguments.commit_message)
        violations = find_violations(lines)
    except (OSError, RuntimeError) as error:
        print(f"human-attribution check failed: {error}", file=sys.stderr)
        return 2

    if not violations:
        return 0

    print("human-attribution check failed:", file=sys.stderr)
    for violation in violations:
        excerpt = violation.text.strip()
        print(
            f"  {violation.source}:{violation.number}: {violation.reason}: {excerpt}",
            file=sys.stderr,
        )
    print("Remove automated attribution and describe the change in your own words.", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
