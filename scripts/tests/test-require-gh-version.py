#!/usr/bin/env python3
"""Self-test for issue #558: a too-old `gh` must fail with a sentence, not a
traceback.

`gh issue list --json ...,blockedBy,...` (issue-claim.sh) needs `blockedBy`,
which shipped in `gh` 2.94.0 (cli/cli#13057, "Issues 2.0: issue types,
sub-issues, and relationships"). Before that release `gh` rejects the field,
writes its "Unknown JSON field" complaint to stderr, and leaves stdout empty
-- so the `python3 -c` piped after it tries to `json.load` an empty stream
and dies with `json.decoder.JSONDecodeError`, a traceback that names the
wrong problem entirely.

`scripts/lib/require-gh.sh` is the fix: every `scripts/issue-*.sh` sources
it, so a `gh` below the floor is refused in one clear sentence naming both
versions. Three of the four source it immediately after `set -euo pipefail`
and are driven here directly with no other setup.

`issue-land.sh` is the exception, on purpose: everything up to and including
`git push` -- the commit guard, the gates -- never touches `gh` at all (see
`test-issue-land-commit-guard.py`'s own docstring), so demanding a `gh`
version before that point would mean a dirty-tree refusal or a red gate
started needing `gh` installed when it never did before. Its `source` line
sits right after the push instead, before the first real `gh pr view` call.
Proving *that* placement behaviourally would mean duplicating the full
gate-passing git+cargo fixture `test-issue-land-merge.py` and
`test-issue-land-312.py` already build and already exercise successfully
with the same stubbed `gh` this file uses -- so this file checks the
placement statically (the source line appears after the `--wip` exit and
before the first `gh pr view`) and leaves the behavioural proof, that a
valid `gh` reaches that point and an old one would be refused there, to
those two.

This stubs `gh --version` at three points -- comfortably old, exactly at the
floor, and comfortably new -- and drives the library directly, plus each of
the three top-guarded scripts, through it, so a script that forgets to
source the guard is caught here rather than by the next session hitting the
traceback by hand.

Usage: scripts/tests/test-require-gh-version.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
REQUIRE_GH = HERE / "lib" / "require-gh.sh"
# Sourced immediately after `set -euo pipefail`, checked dynamically below
# with no other setup. `issue-land.sh` sources the same file later -- see
# the module docstring -- and is checked statically instead, further down.
TOP_GUARDED_SCRIPTS = [
    "issue-claim.sh",
    "issue-file.sh",
    "issue-release.sh",
]

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then
    printf 'gh version %s (2026-01-01)\\nhttps://github.com/cli/cli/releases/tag/v%s\\n' \\
        "$GH_STUB_VERSION" "$GH_STUB_VERSION"
    exit 0
fi
echo "gh stub: unexpected invocation: $*" >&2
exit 1
"""

WRAPPER = """#!/usr/bin/env bash
set -euo pipefail
source "{require_gh}"
echo "past the guard"
"""

FAILURES: list[str] = []


def gh_stub(stub_dir: Path) -> None:
    (stub_dir / "bin").mkdir(parents=True, exist_ok=True)
    gh = stub_dir / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)


def run(args: list[str], stub_dir: Path, version: str) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["GH_STUB_VERSION"] = version
    return subprocess.run(
        args, env=environment, capture_output=True, text=True, timeout=30
    )


def check_wrapper(stub_dir: Path, wrapper: Path, version: str) -> subprocess.CompletedProcess[str]:
    return run(["bash", str(wrapper)], stub_dir, version)


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = Path(directory) / "stub"
        gh_stub(stub_dir)

        wrapper = Path(directory) / "wrapper.sh"
        wrapper.write_text(
            WRAPPER.format(require_gh=REQUIRE_GH), encoding="utf-8"
        )
        wrapper.chmod(0o755)

        # -- too old: refused, with a sentence naming both versions --------
        result = check_wrapper(stub_dir, wrapper, "2.92.0")
        if result.returncode == 0:
            FAILURES.append(
                f"gh 2.92.0 should have been refused, but the wrapper exited 0:\n"
                f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
            )
        if "2.94.0" not in result.stderr:
            FAILURES.append(
                f"the refusal should name the required version 2.94.0:\n"
                f"--- stderr ---\n{result.stderr}"
            )
        if "2.92.0" not in result.stderr:
            FAILURES.append(
                f"the refusal should name the gh version actually found, 2.92.0:\n"
                f"--- stderr ---\n{result.stderr}"
            )
        if "Traceback" in result.stderr or "JSONDecodeError" in result.stderr:
            FAILURES.append(
                f"a Python traceback leaked through instead of a clean message:\n"
                f"--- stderr ---\n{result.stderr}"
            )
        if "past the guard" in result.stdout:
            FAILURES.append(
                "the wrapper kept running past the guard with too-old a gh:\n"
                f"--- stdout ---\n{result.stdout}"
            )

        # -- exactly at the floor: allowed -----------------------------------
        result = check_wrapper(stub_dir, wrapper, "2.94.0")
        if result.returncode != 0 or "past the guard" not in result.stdout:
            FAILURES.append(
                "gh exactly at the floor (2.94.0) should be accepted:\n"
                f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
            )

        # -- comfortably new: allowed -----------------------------------------
        result = check_wrapper(stub_dir, wrapper, "2.98.0")
        if result.returncode != 0 or "past the guard" not in result.stdout:
            FAILURES.append(
                "gh 2.98.0 should be accepted:\n"
                f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
            )

        # -- the three top-guarded scripts, with no other setup ----------------
        for name in TOP_GUARDED_SCRIPTS:
            script = HERE / name
            result = run(["bash", str(script)], stub_dir, "2.92.0")
            if result.returncode == 0:
                FAILURES.append(
                    f"{name} ran to completion despite gh 2.92.0 -- it is not "
                    f"sourcing require-gh.sh, or not doing so before anything "
                    f"else can succeed:\n--- stdout ---\n{result.stdout}"
                )
            if "2.94.0" not in result.stderr:
                FAILURES.append(
                    f"{name}: the refusal should name the required version:\n"
                    f"--- stderr ---\n{result.stderr}"
                )
            if "Traceback" in result.stderr or "JSONDecodeError" in result.stderr:
                FAILURES.append(
                    f"{name}: a Python traceback leaked through instead of a "
                    f"clean message:\n--- stderr ---\n{result.stderr}"
                )

    # -- issue-land.sh: the guard sits later, checked by position -------------
    land_text = (HERE / "issue-land.sh").read_text(encoding="utf-8")
    source_line = 'source "$(dirname "${BASH_SOURCE[0]}")/lib/require-gh.sh"'
    if source_line not in land_text:
        FAILURES.append("issue-land.sh does not source require-gh.sh at all")
    else:
        wip_exit = land_text.find('[ "$WIP" = 1 ]')
        source_at = land_text.find(source_line)
        first_gh_call = land_text.find("gh pr view")
        if wip_exit == -1 or first_gh_call == -1:
            FAILURES.append(
                "issue-land.sh no longer has the --wip exit or a `gh pr view` "
                "call this positional check anchors on -- update the anchors"
            )
        elif not (wip_exit < source_at < first_gh_call):
            FAILURES.append(
                "issue-land.sh's require-gh.sh source line must sit after the "
                "--wip exit (so a WIP push still never touches gh) and before "
                "the first `gh pr view` call (so it is checked before it is "
                f"needed): wip_exit={wip_exit} source_at={source_at} "
                f"first_gh_call={first_gh_call}"
            )

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("require-gh version check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
