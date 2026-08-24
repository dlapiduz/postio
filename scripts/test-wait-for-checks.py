#!/usr/bin/env python3
"""Self-test for scripts/wait-for-checks.sh.

This is the script that decides whether `issue-land.sh` may merge, so the
case that matters most is the one that used to go the wrong way: a code
change whose checks have not registered yet must **not** be read as "nothing
to wait for" (#135, #139). The inverse -- a prose branch blocking forever on
a check that was deliberately never scheduled -- is the failure the old
heuristic existed to avoid, and is covered here too.

`gh` and `git` are stubbed on PATH: `git` prints a fixed diff, and `gh`
replays a scripted sequence of answers, one per call, so the registration
race can be reproduced deterministically instead of waited for.

Usage: scripts/test-wait-for-checks.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "wait-for-checks.sh"

FAILURES: list[str] = []

# Each stub call appends to a counter file and prints the reply for that call,
# so "nothing, nothing, then a check" is expressible as a list.
GH_STUB = """#!/usr/bin/env bash
n=$(cat "$STUB_DIR/calls" 2>/dev/null || echo 0)
echo $((n + 1)) > "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then
    case "$3" in
      --json)  reply=$(sed -n "$((n + 1))p" "$STUB_DIR/json")
               [ -n "$reply" ] || reply=$(tail -n 1 "$STUB_DIR/json")
               if [ "$reply" = "-" ]; then
                   echo "no checks reported on the 'x' branch" >&2
                   exit 8
               fi
               echo "$reply" ;;
      --watch) cat "$STUB_DIR/watch"
               exit "$(cat "$STUB_DIR/watch_status")" ;;
    esac
fi
exit 0
"""

GIT_STUB = """#!/usr/bin/env bash
if [ "$1" = "diff" ]; then cat "$STUB_DIR/diff"; exit 0; fi
exit 0
"""


def run(
    *,
    diff: list[str],
    json_replies: list[str],
    watch_status: int = 0,
    grace: int = 1,
    register_timeout: int = 2,
):
    with tempfile.TemporaryDirectory() as tmp:
        stub_dir = Path(tmp)
        bin_dir = stub_dir / "bin"
        bin_dir.mkdir()
        (bin_dir / "gh").write_text(GH_STUB, encoding="utf-8")
        (bin_dir / "git").write_text(GIT_STUB, encoding="utf-8")
        for name in ("gh", "git"):
            (bin_dir / name).chmod(0o755)
        (stub_dir / "diff").write_text("\n".join(diff) + "\n", encoding="utf-8")
        (stub_dir / "json").write_text("\n".join(json_replies) + "\n", encoding="utf-8")
        (stub_dir / "watch").write_text("all checks reported\n", encoding="utf-8")
        (stub_dir / "watch_status").write_text(str(watch_status), encoding="utf-8")

        env = dict(os.environ)
        env["PATH"] = f"{bin_dir}:{env['PATH']}"
        env["STUB_DIR"] = str(stub_dir)
        env["POSTIO_CHECKS_GRACE"] = str(grace)
        env["POSTIO_CHECKS_REGISTER_TIMEOUT"] = str(register_timeout)
        env["POSTIO_CHECKS_POLL"] = "1"
        return subprocess.run(
            ["bash", str(SCRIPT), "https://example.com/pr/1"],
            capture_output=True,
            text=True,
            env=env,
            stdin=subprocess.DEVNULL,
            timeout=60,
        )


def case(label: str, *, expected_status: int, expect_output: str = "", **kwargs) -> None:
    proc = run(**kwargs)
    if proc.returncode != expected_status:
        FAILURES.append(
            f"{label}: expected exit {expected_status}, got {proc.returncode}\n"
            f"  stdout: {proc.stdout.strip()}\n  stderr: {proc.stderr.strip()}"
        )
        return
    if expect_output and expect_output not in (proc.stdout + proc.stderr):
        FAILURES.append(f"{label}: expected {expect_output!r} in the output")


CODE = ["crates/postio-core/src/command.rs", "crates/postio-gtk/src/reader.rs"]
PROSE = ["README.md", "docs/engineering-notes.md"]
CHECK = '[{"name":"build","bucket":"pass"}]'
FAILED_BUCKET = '[{"name":"build","bucket":"fail"}]'
NONE = "-"


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    # The regression. A five-crate change whose workflow never registers must
    # refuse to merge. Before #139 this printed "nothing to wait for" and the
    # caller merged.
    case(
        "a code change with no check ever must refuse to merge",
        diff=CODE,
        json_replies=[NONE],
        expected_status=1,
        expect_output="Not merging",
    )
    # The other half of the same race: the check is simply late. Reading that
    # as "nothing scheduled" cost a re-run on three consecutive issues.
    case(
        "a check that registers late is waited for, not given up on",
        diff=CODE,
        json_replies=[NONE, NONE, CHECK],
        register_timeout=10,
        expected_status=0,
    )
    case(
        "a code change with a passing check merges",
        diff=CODE,
        json_replies=[CHECK],
        expected_status=0,
    )
    case(
        "a failing check refuses to merge",
        diff=CODE,
        json_replies=[CHECK],
        watch_status=1,
        expected_status=1,
        expect_output="Checks failed",
    )
    # The failure the old heuristic existed to avoid: ci.yml ignores prose on
    # purpose, so a docs branch must not block on a check nobody scheduled.
    case(
        "a prose-only change does not block on a check nobody scheduled",
        diff=PROSE,
        json_replies=[NONE],
        expected_status=0,
        expect_output="nothing to wait for",
    )
    # ...but if one shows up anyway -- a rerun, a workflow_dispatch, a filter
    # this script read differently from GitHub -- its verdict still counts.
    case(
        "a check that appears on a prose branch anyway is still obeyed",
        diff=PROSE,
        json_replies=[CHECK],
        watch_status=1,
        expected_status=1,
        expect_output="Checks failed",
    )
    # An empty JSON array is `gh`'s other way of saying nothing is registered,
    # and reading it as a check would merge on the strength of a stale answer.
    case(
        "an empty check array is not a registered check",
        diff=CODE,
        json_replies=["[]"],
        expected_status=1,
        expect_output="Not merging",
    )
    # #161: `--watch --fail-fast` returned success two seconds before CI's own
    # FAILURE conclusion was recorded, and issue-land.sh merged the red
    # commit. `watch_status=0` here plays that exact race -- the watch call
    # claims success -- and the second, non-watching read has to be the one
    # that actually refuses.
    case(
        "watch claiming success is not trusted on its own -- issue #161",
        diff=CODE,
        json_replies=[CHECK, FAILED_BUCKET],
        watch_status=0,
        expected_status=1,
        expect_output="not green after watching",
    )

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("wait-for-checks self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
