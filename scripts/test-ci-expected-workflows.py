#!/usr/bin/env python3
"""Self-test for scripts/ci-expected-workflows.py.

The predicate it implements decides whether `issue-land.sh` is allowed to
merge without seeing a check, so a wrong answer in one direction wastes a
re-run and in the other merges code CI never looked at (#139, #131). Both
directions are exercised here, against synthetic workflow trees in a temp
dir and against this repository's real `.github/workflows`.

The last case is #135 verbatim in shape: a multi-crate Rust change that the
old `gh pr checks` heuristic called "prose-only" and merged unchecked.

Usage: scripts/test-ci-expected-workflows.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
SCRIPT = HERE / "ci-expected-workflows.py"

FAILURES: list[str] = []

# Shaped like the real ci.yml: a `paths-ignore` list defined once behind a
# YAML anchor and reused by the `pull_request` trigger through an alias. The
# alias is the part a naive line-oriented reader gets wrong, and getting it
# wrong means every pull request looks unfiltered.
CI = """\
name: CI
on:
  push:
    branches: [main]
    paths-ignore: &prose
      - '*.md'
      - '.claude/**'
      - 'docs/PRODUCT.md'
  pull_request:
    paths-ignore: *prose
  workflow_dispatch:

jobs:
  build:
    runs-on: ubuntu-latest
"""

# A positive `paths` filter, the opposite polarity to the one above.
HOOKS = """\
name: Hooks
on:
  push:
    paths:
      - '.claude/hooks/**'
      - '.github/workflows/hooks.yml'
  pull_request:
    paths:
      - '.claude/hooks/**'
      - '.github/workflows/hooks.yml'
"""

# Tags only. Nothing a pull request does can schedule it.
RELEASE = """\
name: Release
on:
  push:
    tags: ["v*"]
  workflow_dispatch:
"""

# No filters at all: every pull request runs it.
ALWAYS = """\
name: Always
on:
  pull_request:
"""


def run(workflows: dict[str, str], paths: list[str], *, base: str = "main"):
    """Run the script against a throwaway workflow directory."""
    with tempfile.TemporaryDirectory() as tmp:
        wf = Path(tmp) / ".github" / "workflows"
        wf.mkdir(parents=True)
        for filename, body in workflows.items():
            (wf / filename).write_text(body, encoding="utf-8")
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--workflows",
                str(wf),
                "--base",
                base,
                *paths,
            ],
            capture_output=True,
            text=True,
            # Closed, not inherited: with no paths on the command line the
            # script falls back to reading a diff from stdin, and a test that
            # left it open would hang instead of exercising the empty case.
            stdin=subprocess.DEVNULL,
        )


def case(
    label: str,
    *,
    workflows: dict[str, str],
    paths: list[str],
    expected: list[str],
    base: str = "main",
) -> None:
    """`expected` is the workflow names that should run, in file order."""
    proc = run(workflows, paths, base=base)
    if proc.returncode == 2:
        FAILURES.append(f"{label}: the script errored\n{proc.stderr}")
        return
    got = [line for line in proc.stdout.splitlines() if line.strip()]
    if got != expected:
        FAILURES.append(f"{label}: expected {expected}, got {got}")
        return
    # Exit status is the shell-facing half of the contract: 0 means something
    # will run and must be waited for, 1 means nothing will.
    want_status = 0 if expected else 1
    if proc.returncode != want_status:
        FAILURES.append(
            f"{label}: expected exit {want_status}, got {proc.returncode}"
        )


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    all_three = {"ci.yml": CI, "hooks.yml": HOOKS, "release.yml": RELEASE}

    case(
        "a Rust change is not prose and must be waited for",
        workflows=all_three,
        paths=["crates/postio-core/src/lib.rs"],
        expected=["CI"],
    )
    case(
        "an ignored file on its own schedules nothing",
        workflows=all_three,
        paths=["README.md"],
        expected=[],
    )
    case(
        "an alias to an anchored ignore list is resolved, not skipped",
        workflows={"ci.yml": CI},
        paths=["docs/PRODUCT.md"],
        expected=[],
    )
    case(
        "one non-ignored file among ignored ones still runs CI",
        workflows=all_three,
        paths=["README.md", "docs/PRODUCT.md", "crates/postio-gtk/src/list.rs"],
        expected=["CI"],
    )
    case(
        "a positive paths filter matches its own directory",
        workflows=all_three,
        paths=[".claude/hooks/guard-shared-tree.py"],
        expected=["Hooks"],
    )
    case(
        "a positive paths filter ignores everything else",
        workflows={"hooks.yml": HOOKS},
        paths=["crates/postio-core/src/lib.rs"],
        expected=[],
    )
    case(
        "a tags-only workflow never runs on a pull request",
        workflows={"release.yml": RELEASE},
        paths=["crates/postio-core/src/lib.rs", "README.md"],
        expected=[],
    )
    case(
        "a pull_request trigger with no filters always runs",
        workflows={"always.yml": ALWAYS},
        paths=["README.md"],
        expected=["Always"],
    )
    case(
        "two workflows can both be expected",
        workflows=all_three,
        paths=[".claude/hooks/guard-shared-tree.py", "crates/postio-core/src/x.rs"],
        expected=["CI", "Hooks"],
    )
    case(
        "an empty diff schedules nothing",
        workflows=all_three,
        paths=[],
        expected=[],
    )

    # Glob semantics. GitHub's filter patterns are not shell globs: `*` stops
    # at a slash and `**` does not. `'*.md'` in the real ci.yml is there to
    # ignore top-level prose only -- if `*` crossed directories it would
    # silently ignore every markdown file in the tree, including generated
    # ones a test exists to catch drifting.
    star = """\
name: Star
on:
  pull_request:
    paths-ignore:
      - '*.md'
"""
    case(
        "* does not cross a slash",
        workflows={"star.yml": star},
        paths=["docs/notes.md"],
        expected=["Star"],
    )
    case(
        "* matches at the top level",
        workflows={"star.yml": star},
        paths=["README.md"],
        expected=[],
    )

    doublestar = """\
name: DoubleStar
on:
  pull_request:
    paths-ignore:
      - 'docs/**'
"""
    case(
        "** crosses slashes",
        workflows={"ds.yml": doublestar},
        paths=["docs/decisions/0004-body.md"],
        expected=[],
    )

    negated = """\
name: Negated
on:
  pull_request:
    paths-ignore:
      - 'docs/**'
      - '!docs/keybindings.md'
"""
    case(
        "a later ! pattern rescues a path an earlier one ignored",
        workflows={"neg.yml": negated},
        paths=["docs/keybindings.md"],
        expected=["Negated"],
    )
    case(
        "a ! pattern does not rescue its siblings",
        workflows={"neg.yml": negated},
        paths=["docs/ARCHITECTURE.md"],
        expected=[],
    )

    branched = """\
name: Branched
on:
  pull_request:
    branches: [release]
"""
    case(
        "a base-branch filter that does not match keeps the workflow out",
        workflows={"br.yml": branched},
        paths=["crates/postio-core/src/lib.rs"],
        expected=[],
    )
    case(
        "a base-branch filter that matches lets it in",
        workflows={"br.yml": branched},
        paths=["crates/postio-core/src/lib.rs"],
        expected=["Branched"],
        base="release",
    )

    # The regression, against the real workflows rather than a fixture. #135
    # was a five-crate change that the old heuristic called prose and merged
    # before CI existed. If this repository's own ci.yml ever stops expecting
    # a check for a change like that, this is the case that says so.
    #
    # PAUSED 2026-08-25: ci.yml's `push`/`pull_request` triggers are commented
    # out (private-repo minutes, see ci.yml's own note), so nothing is
    # expected for *any* change right now, prose or not -- that is
    # workflow_dispatch-only, correctly. When those triggers come back,
    # restore this to the pre-pause assertion: exit 0 and "CI" in stdout.
    real = REPO / ".github" / "workflows"
    if real.is_dir():
        proc = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--workflows",
                str(real),
                "--base",
                "main",
                "crates/postio-core/src/command.rs",
                "crates/postio-gtk/src/reader.rs",
                "crates/postio-storage/src/lib.rs",
            ],
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
        )
        if proc.returncode != 1 or proc.stdout.strip():
            FAILURES.append(
                "a five-crate change against the real workflows must expect "
                "nothing while CI is paused "
                f"(exit {proc.returncode}, stdout {proc.stdout!r})"
            )
        prose = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--workflows",
                str(real),
                "--base",
                "main",
                "README.md",
                "docs/engineering-notes.md",
            ],
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
        )
        if prose.returncode != 1 or prose.stdout.strip():
            FAILURES.append(
                "a prose-only change against the real workflows must expect "
                f"nothing (exit {prose.returncode}, stdout {prose.stdout!r})"
            )

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("ci-expected-workflows self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
