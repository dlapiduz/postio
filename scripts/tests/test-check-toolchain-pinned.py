#!/usr/bin/env python3
"""Self-test for scripts/checks/check-toolchain-pinned.py.

The guard passes on the real tree once the pin is in place, and would pass
just as cheerfully if it had stopped looking at anything. So: throwaway trees
in a temp dir, one per way the rule can be met or broken, and an assertion for
each. The real repository is never touched and nothing here reaches the
network.

Usage: scripts/tests/test-check-toolchain-pinned.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-toolchain-pinned.py"

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

# The same bug in one line instead of a block. This is the shorter spelling and
# so the likelier way to reintroduce it, and the check used to miss it.
WORKFLOW_FLOATING_INLINE = """\
jobs:
  build:
    steps:
      - name: Install Rust toolchain
        run: rustup default stable
"""

# What fuzz.yml does. A dated nightly names one compiler as exactly as a
# version does, and `-Z sanitizer=address` has no stable spelling at any
# version, so this must be allowed -- see #147.
WORKFLOW_DATED_NIGHTLY = """\
jobs:
  fuzz:
    steps:
      - name: Install the fuzzing toolchain
        run: rustup toolchain install nightly-2026-08-24 --profile minimal
"""

# A bare `nightly` is still the bug: it floats exactly like `stable` does.
WORKFLOW_BARE_NIGHTLY = """\
jobs:
  fuzz:
    steps:
      - name: Install the fuzzing toolchain
        run: rustup toolchain install nightly --profile minimal
"""


# Rule 5: the interpreter. A job that runs the repository's scripts must set up
# the Python mise.toml names, through the one composite action.
PYTHON_ACTION = """\
runs:
  using: composite
  steps:
    - id: version
      shell: bash
      run: sed -n 's/^python = "\\(.*\\)"/\\1/p' mise.toml
    - uses: actions/setup-python@6d4fd22b20465ccc0f7354476588e8df7c9ca0e0 # v7.1.0
      with:
        python-version: ${{ steps.version.outputs.version }}
"""

# A literal version in the action is a second pin: it never reads mise.toml.
PYTHON_ACTION_LITERAL = """\
runs:
  using: composite
  steps:
    - uses: actions/setup-python@6d4fd22b20465ccc0f7354476588e8df7c9ca0e0 # v7.1.0
      with:
        python-version: "3.14.7"
"""

WORKFLOW_PYTHON_BARE = """\
jobs:
  checks:
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - name: Checks
        run: scripts/check.sh
"""

WORKFLOW_PYTHON_DIRECT = """\
jobs:
  checks:
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - name: Checks
        run: python3 scripts/commit-msg.py --title "$T"
"""

WORKFLOW_PYTHON_SET_UP = """\
jobs:
  checks:
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - uses: ./.github/actions/python
      - name: Checks
        run: |
          scripts/check.sh
          python3 scripts/commit-msg.py --title "$T"
"""

# The action before the checkout reads a mise.toml that is not there yet.
WORKFLOW_PYTHON_BEFORE_CHECKOUT = """\
jobs:
  checks:
    steps:
      - uses: ./.github/actions/python
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - name: Checks
        run: scripts/check.sh
"""

# `scripts/` in a cache key or a comment is not a script being run.
WORKFLOW_PYTHON_NOT_RUN = """\
jobs:
  build:
    steps:
      - uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          key: ${{ hashFiles('scripts/install-nextest.sh') }}
      - name: Build
        run: |
          # scripts/check.sh runs elsewhere
          cargo build
"""

# Only the composite action may name setup-python; a job doing it itself
# carries its own version, which is the drift this rule exists to stop.
WORKFLOW_SETUP_PYTHON_DIRECT = """\
jobs:
  checks:
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - uses: actions/setup-python@6d4fd22b20465ccc0f7354476588e8df7c9ca0e0 # v7.1.0
        with:
          python-version: "3.14"
      - name: Checks
        run: scripts/check.sh
"""


def build_tree(
    root: Path, *, toolchain: str | None, workflow: str, manifest: str | None = None,
    mise: str | None = None, python_action: str | None = None,
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
    if mise is not None:
        (root / "mise.toml").write_text(mise, encoding="utf-8")
    workflows = root / ".github" / "workflows"
    workflows.mkdir(parents=True)
    if python_action is not None:
        action = root / ".github" / "actions" / "python"
        action.mkdir(parents=True)
        (action / "action.yml").write_text(python_action, encoding="utf-8")
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
    mise: str | None = None,
    python_action: str | None = None,
) -> None:
    """Assert the check's verdict on one tree."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        build_tree(root, toolchain=toolchain, workflow=workflow, manifest=manifest, mise=mise,
                   python_action=python_action)
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
    case(
        "the one-line `run: rustup default stable` fails too",
        toolchain=exact,
        workflow=WORKFLOW_FLOATING_INLINE,
        expected=1,
    )

    # ── the fuzzing exception, and its edge ──────────────────────────────
    # A dated nightly is a pin. A bare one is not. The distinction is the
    # whole reason fuzz.yml is allowed to exist alongside this check.
    case(
        "a dated nightly is a pin, not a float",
        toolchain=exact,
        workflow=WORKFLOW_DATED_NIGHTLY,
        expected=0,
    )
    case(
        "a bare nightly is still refused",
        toolchain=exact,
        workflow=WORKFLOW_BARE_NIGHTLY,
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
    # mise sets RUSTUP_TOOLCHAIN for every shell in the repository, and that
    # beats rust-toolchain.toml. `rust = "latest"` in mise.toml resolved to
    # whatever was installed -- 1.98 -- and outlived the 1.99 bump (2026-10-01).
    for label, mise_toml, expected in (
        ("mise.toml naming rust is a second pin and fails", '[tools]\npython = "3.14.7"\nrust = "latest"\n', 1),
        ("mise.toml naming an exact rust fails too", '[tools]\nrust = "1.99.0"\n', 1),
        ("mise.toml without rust passes", '[tools]\npython = "3.14.7"\n', 0),
        ("rust named only in a comment passes", '# Rust: rust-toolchain.toml owns it\n[tools]\njq = "1.8.1"\n', 0),
    ):
        case(label, toolchain=exact, workflow=WORKFLOW_PINNED, expected=expected, mise=mise_toml)

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

    # ── rule 5: CI runs the Python mise.toml names ───────────────────────
    mise_py = '[tools]\npython = "3.14.7"\n'
    for label, workflow, action, expected in (
        ("a job running scripts/ without the python action fails", WORKFLOW_PYTHON_BARE, PYTHON_ACTION, 1),
        ("a job running python3 without the python action fails", WORKFLOW_PYTHON_DIRECT, PYTHON_ACTION, 1),
        ("a job that sets the python action up passes", WORKFLOW_PYTHON_SET_UP, PYTHON_ACTION, 0),
        ("the python action before the checkout fails", WORKFLOW_PYTHON_BEFORE_CHECKOUT, PYTHON_ACTION, 1),
        ("scripts/ in a cache key or a comment needs nothing", WORKFLOW_PYTHON_NOT_RUN, None, 0),
        ("a job using actions/setup-python itself fails", WORKFLOW_SETUP_PYTHON_DIRECT, PYTHON_ACTION, 1),
        ("a python action with a literal version fails", WORKFLOW_PYTHON_SET_UP, PYTHON_ACTION_LITERAL, 1),
        ("a workflow using the action when it is missing fails", WORKFLOW_PYTHON_SET_UP, None, 1),
    ):
        case(label, toolchain=exact, workflow=workflow, expected=expected,
             mise=mise_py, python_action=action)

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("check-toolchain-pinned self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
