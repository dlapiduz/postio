#!/usr/bin/env python3
"""Refuse a floating rustc: CI and every session must agree on one compiler.

CI used to run ``rustup default stable`` while the repository pinned nothing,
so the compiler that decided whether a PR was green was whatever shipped most
recently, and every session here built with whatever it happened to have
installed. That is not a hypothetical: rustc 1.98.0 (2026-08-18) tightened
``unused_imports`` for redundant glob imports and flagged ``use adw::prelude::*``
in ``compose::tests``. Sessions on 1.97.1 saw nothing, every local gate passed,
and the clippy job failed on ``-D warnings`` after the push -- a failure that
was unreproducible locally *by construction*. Issue #38.

It recurs on every rustc release, always the same way: green locally, red in
CI, on a lint nobody wrote.

# The rules

1. ``rust-toolchain.toml`` exists and names an **exact version**, not a
   channel. ``stable`` in that file floats exactly like ``rustup default
   stable`` did; the point of the pin is that a compiler change becomes a
   commit somebody made on purpose.
2. No GitHub workflow selects a toolchain of its own. A ``rustup default
   stable`` anywhere in ``.github/workflows`` silently wins over the file for
   that job, which is the bug this check exists to prevent -- reintroduced in
   one line, in a file nobody re-reads.

# What this check cannot see, and what to do about it

``RUSTUP_TOOLCHAIN`` in the environment **overrides rust-toolchain.toml**.
rustup's precedence is env var, then ``rustup override``, then the file. So a
machine that exports it -- this project's own workstation does, from
``~/.config/mise/config.toml`` -- ignores the pin while looking pinned, which
is the original bug wearing the fix's clothes.

A repository check cannot police anyone's shell, so this reports the skew
rather than failing on it: an exit status keyed to a developer's environment
would make CI's answer depend on the runner's, which is the thing being
fixed. ``--strict`` turns the warning into a failure for anyone who wants it
enforced locally. See ``docs/engineering-notes.md``.

# Exit status

0 clean, 1 the pin is missing or a workflow overrides it, 2 could not run.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

# `rustup default <channel>` / `rustup toolchain install <channel>` where the
# channel is a floating name rather than a version. `1.98.0` is fine; `stable`
# is the bug.
FLOATING = re.compile(
    r"^\s*rustup\s+(?:default|toolchain\s+install)\s+(stable|beta|nightly)\b",
    re.MULTILINE,
)

# `channel = "1.98.0"` in rust-toolchain.toml. Deliberately strict: a channel
# name here floats just as hard as it did in the workflow.
EXACT = re.compile(r'^\s*channel\s*=\s*"(\d+\.\d+(?:\.\d+)?)"\s*$', re.MULTILINE)


def repository_root() -> Path:
    """The checkout this script lives in."""
    try:
        top = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
            cwd=Path(__file__).resolve().parent,
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError) as error:
        print(f"cannot find the repository root: {error}", file=sys.stderr)
        raise SystemExit(2) from error
    return Path(top)


def pinned_version(root: Path) -> str | None:
    """The exact version `rust-toolchain.toml` names, if it names one."""
    path = root / "rust-toolchain.toml"
    if not path.is_file():
        return None
    found = EXACT.search(path.read_text(encoding="utf-8"))
    return found.group(1) if found else None


def floating_workflows(root: Path) -> list[str]:
    """Workflow lines that pick a toolchain instead of honouring the file."""
    offenders: list[str] = []
    workflows = root / ".github" / "workflows"
    for path in sorted(workflows.glob("*.yml")) + sorted(workflows.glob("*.yaml")):
        text = path.read_text(encoding="utf-8")
        for match in FLOATING.finditer(text):
            line = text[: match.start()].count("\n") + 1
            offenders.append(f"{path.relative_to(root)}:{line}: {match.group(0).strip()}")
    return offenders


def report_environment_skew(pinned: str) -> None:
    """Say so when this shell will ignore the pin. Never fatal -- see the docs."""
    override = os.environ.get("RUSTUP_TOOLCHAIN")
    if override and not override.startswith(pinned):
        print(
            f"warning: RUSTUP_TOOLCHAIN={override} overrides rust-toolchain.toml "
            f"({pinned}), so this shell is not building with the pinned compiler.\n"
            f"         rustup reads the environment before the file. On this "
            f"project's workstation the export comes from ~/.config/mise/config.toml.",
            file=sys.stderr,
        )


def main() -> int:
    strict = "--strict" in sys.argv[1:]
    root = repository_root()

    pinned = pinned_version(root)
    if pinned is None:
        path = root / "rust-toolchain.toml"
        why = "does not name an exact version" if path.is_file() else "is missing"
        print(
            f"toolchain check FAILED: rust-toolchain.toml {why}.\n"
            f"  CI and every session must agree on one compiler, or a lint that "
            f"fires in one and not the other turns main red on a change nobody "
            f"made. Add:\n\n"
            f'    [toolchain]\n    channel = "1.98.0"\n',
            file=sys.stderr,
        )
        return 1

    offenders = floating_workflows(root)
    if offenders:
        print(
            f"toolchain check FAILED: {len(offenders)} workflow step(s) select a "
            f"floating toolchain, which wins over rust-toolchain.toml ({pinned}):",
            file=sys.stderr,
        )
        for offender in offenders:
            print(f"  {offender}", file=sys.stderr)
        print(
            "  Drop the selection and let the file decide; `rustup show` installs "
            "what it names.",
            file=sys.stderr,
        )
        return 1

    override = os.environ.get("RUSTUP_TOOLCHAIN")
    skewed = bool(override) and not override.startswith(pinned)
    if skewed:
        report_environment_skew(pinned)
        if strict:
            return 1

    print(f"toolchain check passed (pinned to {pinned}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
