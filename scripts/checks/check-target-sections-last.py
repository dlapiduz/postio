#!/usr/bin/env python3
"""A platform section must not sit inside a plain dependency table.

`main` did not build on Linux for a day because of this, and the diff that
did it looked tidy. `74d3b9c` added the macOS Keychain dependency to
`postio-imap` by inserting a header into the middle of an alphabetically
sorted list:

    [dependencies]
    anyhow = "1.0.104"
    ...
    rustls-platform-verifier = { version = "0.7", optional = true }
    [target.'cfg(target_os = "macos")'.dependencies]     <- here
    security-framework = "3.7.0"
    secrecy = { version = "0.10", default-features = false }
    serde = { version = "1.0.229", features = ["derive"] }
    ...
    zeroize = "1.9.0"

`security-framework` sorts between `rustls-platform-verifier` and `secrecy`,
so it reads as a sorted list and nothing looks wrong. But a TOML table runs
until the next header, so **every entry below that line became macOS-only** --
fifteen of them, including `postio-model`, `tokio`, `serde` and `thiserror`.
On Linux the crate had almost no dependencies and produced 219 compiler
errors. Issue #642.

# The rule

Every `[target.'cfg(...)'...]` table comes **after** every plain
`[dependencies]`, `[build-dependencies]` and `[dev-dependencies]` table in the
same manifest. Put platform sections at the foot of the file.

This is a placement rule rather than a correctness proof, and it is worth
being honest about why: TOML has no notion of a table somebody *meant* to keep
going, so the swallowing itself is not detectable. What is detectable is the
position that makes it possible. A platform section at the foot of the file
cannot swallow anything, because there is nothing below it to swallow.

# What this does not do

It does not check that platform-conditional code compiles for that platform --
nothing on a Linux box can. See `docs/engineering-notes.md` on cross-platform
dependencies for the layers that do: this check, `cargo check --target` where
the C dependencies allow it, and a CI runner for the rest.
"""

from __future__ import annotations

import sys
from pathlib import Path

# The tables a platform section must not precede.
PLAIN_TABLES = ("[dependencies]", "[build-dependencies]", "[dev-dependencies]")


def offending_manifests(root: Path) -> list[tuple[Path, int, str, int, str]]:
    """Every manifest where a `[target.…]` header precedes a plain dependency
    table, as (path, target line, target header, plain line, plain header)."""
    found = []
    manifests = [root / "Cargo.toml", *sorted((root / "crates").glob("*/Cargo.toml"))]
    for manifest in manifests:
        try:
            lines = manifest.read_text().splitlines()
        except OSError:
            continue

        first_target: tuple[int, str] | None = None
        for number, line in enumerate(lines, start=1):
            stripped = line.strip()
            if stripped.startswith("[target.") and first_target is None:
                first_target = (number, stripped)
            elif stripped in PLAIN_TABLES and first_target is not None:
                found.append(
                    (manifest.relative_to(root), *first_target, number, stripped)
                )
                break
    return found


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    offenders = offending_manifests(root)

    if offenders:
        print(
            "target-sections-last check FAILED: a platform section precedes a "
            "plain dependency table.\n",
            file=sys.stderr,
        )
        for path, target_line, target, plain_line, plain in offenders:
            print(
                f"  {path}:{target_line}\n"
                f"    {target}\n"
                f"  ...comes before {plain} at line {plain_line}.\n",
                file=sys.stderr,
            )
        print(
            "  A TOML table runs until the next header, so a platform section "
            "placed above other\n"
            "  dependency tables makes everything between them conditional -- "
            "silently, and only on\n"
            "  the platforms it excludes. That is what left `main` unbuildable "
            "on Linux (#642).\n\n"
            "  Fix: move the `[target.…]` table to the foot of the manifest, "
            "below every plain\n"
            "  dependency table. Nothing below it means nothing to swallow.",
            file=sys.stderr,
        )
        return 1

    counted = 1 + len(list((root / "crates").glob("*/Cargo.toml")))
    print(f"target-sections-last check passed ({counted} manifests).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
