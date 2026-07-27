+++
id = "2b76980b-a4de-4d91-993b-3c2fa6d4d57b"
title = "Rust latest-stable policy and 1.97 baseline"
tags = []
aliases = []
source_format = "omegon_memory"
source_path = "omegon://memory/toolchain"
imported_at = "2026-07-12T21:13:11.968457Z"
imported_reference = true
kind = "memory_fact"
topic = "toolchain"

[publication]
enabled = false
visibility = "private"

+++

As of 2026-07-12 styrene-rs pins Rust 1.97.0 (latest stable released 2026-07-07) in rust-toolchain.toml and CI; workspace rust-version=1.97, local hardcoded crate pins replaced by workspace inheritance. Policy: track latest stable each ~6-week release cycle, not N-1. Commit a0cdf420. cargo check passes; release styrene/styrene-tui rebuilt and installed. Full cargo test exposes a pre-existing hang when entering the styrene-tui test binary (process remains alive before emitting test count); preceding suites show no failures. This TUI harness hang is the next baseline installation/instantiation issue.
