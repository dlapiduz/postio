#!/usr/bin/env python3
"""Self-test for scripts/check-toolchain-pinned.py.

The guard passes on the real tree once the pin is in place, and would pass
just as cheerfully if it had stopped looking at anything. So: throwaway trees
in a temp dir, one per way the rule can be met or broken, and an assertion for
each. The real repository is never touched and nothing here reaches the
network.

Usage: scripts/test-check-toolchain-pinned.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = HERE / "check-toolchain-pinned.py"

FAILURES: list[str] = []

WORKFLOW_PINNED = """\
jobs:
  build:
    steps:
      - name: Install Rust toolchain
        run: rustup show active-toolchain
"""

WORKFLOW_FLOATING = """\
jobs:
  build:
    steps:
      - name: Install Rust toolchain
        run: |
          rustup toolchain install stable --profile minimal
          rustup default stable
"""


def build_tree(
    root: Path, *, toolchain: str | None, workflow: str, manifest: str | None = None
) -> None:
    """A git repository with an optional rust-toolchain.toml and one workflow.

    `manifest` is the workspace `Cargo.toml`. Omitted for the cases that
    predate the MSRV rule, which is correct: with no manifest there is no
    `rust-version` to disagree with the pin, and the rule stays silent."""
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    if toolchain is not None:
        (root / "rust-toolchain.toml").write_text(toolchain, encoding="utf-8")
    if manifest is not None:
        (root / "Cargo.toml").write_text(manifest, encoding="utf-8")
    workflows = root / ".github" / "workflows"
    workflows.mkdir(parents=True)
    (workflows / "ci.yml").write_text(workflow, encoding="utf-8")


def case(
    name: str,
    *,
    toolchain: str | None,
    workflow: str,
    expected: int,
    env_toolchain: str | None = None,
    strict: bool = False,
    manifest: str | None = None,
) -> None:
    """Assert the check's verdict on one tree."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        build_tree(root, toolchain=toolchain, workflow=workflow, manifest=manifest)
        # The check finds its root from its own location, not the cwd, so it
        # has to be run from a copy that lives inside the throwaway tree.
        scripts = root / "scripts"
        scripts.mkdir()
        (scripts / CHECK.name).write_bytes(CHECK.read_bytes())
        argv = [sys.executable, str(scripts / CHECK.name)] + (
            ["--strict"] if strict else []
        )
        environment = dict(os.environ)
        environment.pop("RUSTUP_TOOLCHAIN", None)
        if env_toolchain is not None:
            environment["RUSTUP_TOOLCHAIN"] = env_toolchain
        status = subprocess.run(
            argv, cwd=root, capture_output=True, text=True, env=environment
        ).returncode
    if status != expected:
        FAILURES.append(f"{name}: expected exit {expected}, got {status}")


def main() -> int:
    exact = '[toolchain]\nchannel = "1.98.0"\n'

    # ── the shape that is correct ────────────────────────────────────────
    case(
        "an exact pin with a workflow that honours it passes",
        toolchain=exact,
        workflow=WORKFLOW_PINNED,
        expected=0,
    )

    # ── the state issue #38 describes ────────────────────────────────────
    case(
        "no rust-toolchain.toml at all fails",
        toolchain=None,
        workflow=WORKFLOW_PINNED,
        expected=1,
    )
    case(
        "a workflow that installs `stable` fails even with a pin present",
        toolchain=exact,
        workflow=WORKFLOW_FLOATING,
        expected=1,
    )

    # ── rule 3: an MSRV nothing ever builds ──────────────────────────────
    # The manifest said 1.90 while the pin said 1.98, so the claim rested on
    # nothing. These pin the rule's edges rather than just its happy path.
    def manifest_with(version: str | None) -> str:
        head = "[workspace]\nresolver = \"3\"\nmembers = []\n\n[workspace.package]\n"
        return head if version is None else head + f'rust-version = "{version}"\n'

    case(
        "an MSRV older than the pin fails",
        toolchain=exact,
        workflow=WORKFLOW_PINNED,
        manifest=manifest_with("1.90"),
        expected=1,
    )
    case(
        "an MSRV matching the pin passes",
        toolchain=exact,
        workflow=WORKFLOW_PINNED,
        manifest=manifest_with("1.98"),
        expected=0,
    )
    case(
        "no rust-version at all passes",
        toolchain=exact,
        workflow=WORKFLOW_PINNED,
        manifest=manifest_with(None),
        expected=0,
    )
    # Numeric comparison, not lexicographic: "1.9" > "1.10" as strings, and a
    # check that got this wrong would pass the one case most likely to appear.
    case(
        "an MSRV of 1.9 against a 1.10 pin fails",
        toolchain='[toolchain]\nchannel = "1.100.0"\n',
        workflow=WORKFLOW_PINNED,
        manifest=manifest_with("1.9"),
        expected=1,
    )

    # ── a pin that is not a pin ──────────────────────────────────────────
    # `channel = "stable"` in the file floats exactly like the workflow did,
    # so it must not be mistaken for a fix.
    for floating in ("stable", "beta", "nightly", "1.98"):
        expected = 0 if floating == "1.98" else 1
        case(
            f'channel = "{floating}" is {"accepted" if expected == 0 else "refused"}',
            toolchain=f'[toolchain]\nchannel = "{floating}"\n',
            workflow=WORKFLOW_PINNED,
            expected=expected,
        )

    # ── RUSTUP_TOOLCHAIN, which beats the file ───────────────────────────
    # Reported, never fatal by default: CI's verdict must not depend on a
    # developer's shell. `--strict` is for whoever wants it enforced.
    case(
        "a mismatched RUSTUP_TOOLCHAIN warns but passes",
        toolchain=exact,
        workflow=WORKFLOW_PINNED,
        env_toolchain="1.97.1",
        expected=0,
    )
    case(
        "a mismatched RUSTUP_TOOLCHAIN fails under --strict",
        toolchain=exact,
        workflow=WORKFLOW_PINNED,
        env_toolchain="1.97.1",
        strict=True,
        expected=1,
    )
    case(
        "a RUSTUP_TOOLCHAIN that agrees with the pin is not a skew",
        toolchain=exact,
        workflow=WORKFLOW_PINNED,
        env_toolchain="1.98.0-x86_64-unknown-linux-gnu",
        strict=True,
        expected=0,
    )

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("check-toolchain-pinned self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
