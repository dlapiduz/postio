#!/usr/bin/env python3
"""Self-test for scripts/release-await-nightly.sh.

A release no longer runs the full suite beside the nightly that is already
running it on the same commit; it waits for that run's answer. The answer
has to be "green" only when every gate passed, "red" (and a failing exit)
when one failed, and "run-here" whenever there is nothing to wait for, so
that no path ships without the suite having passed somewhere.

`gh` is stubbed: each call reads the next of a list of answers for its kind
(runs or jobs), repeating the last.

Usage: scripts/tests/test-release-await-nightly.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "release-await-nightly.sh"

FAILURES: list[str] = []

GH_STUB = """#!/usr/bin/env bash
case "$*" in *"/jobs"*) kind=jobs ;; *) kind=runs ;; esac
n=$(cat "$STUB_DIR/$kind.count" 2>/dev/null || echo 0); n=$((n + 1))
echo "$n" > "$STUB_DIR/$kind.count"
file="$STUB_DIR/$kind.$n"
[ -f "$file" ] || file=$(ls "$STUB_DIR"/$kind.[0-9]* | sort -t. -k2 -n | tail -1)
if [ "$(cat "$file")" = FAIL ]; then echo "HTTP 502" >&2; exit 1; fi
filter=""
while [ $# -gt 0 ]; do [ "$1" = --jq ] && filter="$2"; shift; done
jq -r "$filter" "$file"
"""

URL = "https://github.com/example/postio/actions/runs/7"


def runs(status: str | None, conclusion: str | None = None) -> str:
    if status is None:
        return json.dumps({"workflow_runs": []})
    return json.dumps({"workflow_runs": [
        {"id": 7, "status": status, "conclusion": conclusion, "html_url": URL}
    ]})


def jobs(*rows: tuple[str, str, str | None]) -> str:
    return json.dumps({"jobs": [
        {"name": name, "status": status, "conclusion": conclusion}
        for name, status, conclusion in rows
    ] + [{"name": "Coverage floors", "status": "completed", "conclusion": "failure"}]})


GREEN = jobs(
    ("Full suite", "completed", "success"),
    ("Storyboard passes (1/4)", "completed", "success"),
    ("Storyboard passes (2/4)", "completed", "success"),
)


def run(runs_replies: list[str], jobs_replies: list[str]) -> tuple[int, str, str, int]:
    with tempfile.TemporaryDirectory() as tmp:
        stub = Path(tmp)
        bin_dir = stub / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text(GH_STUB, encoding="utf-8")
        gh.chmod(0o755)
        for kind, replies in (("runs", runs_replies), ("jobs", jobs_replies)):
            for n, reply in enumerate(replies or [jobs()], start=1):
                (stub / f"{kind}.{n}").write_text(reply, encoding="utf-8")
        env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "STUB_DIR": str(stub),
            "GITHUB_SHA": "abc123",
            "GITHUB_REPOSITORY": "example/postio",
            "RELEASE_AWAIT_POLL": "0",
            "RELEASE_AWAIT_GRACE": "1",
            "RELEASE_AWAIT_LIMIT": "5",
        }
        done = patience.run(
            ["bash", str(SCRIPT)], env=env, capture_output=True, text=True, timeout=60
        )
        asked = int((stub / "runs.count").read_text()) if (stub / "runs.count").exists() else 0
        return done.returncode, done.stdout.strip(), done.stderr, asked


def case(name: str, got: tuple[int, str, str, int], code: int, answer: str) -> None:
    status, out, err, _ = got
    if status == code and out == f"answer={answer}":
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: exit {status}, stdout {out!r}, stderr {err[-400:]!r}")


def main() -> int:
    case("every gate passed is green, whatever coverage says",
         run([runs("completed", "failure")], [GREEN]), 0, "green")

    case("it waits while the gates run",
         run([runs("in_progress")] * 3 + [runs("completed", "success")],
             [jobs(("Full suite", "in_progress", None),
                   ("Storyboard passes (1/4)", "completed", "success")),
              jobs(("Full suite", "in_progress", None),
                   ("Storyboard passes (1/4)", "completed", "success")),
              GREEN]),
         0, "green")

    case("a failed gate is red and fails the job",
         run([runs("in_progress")],
             [jobs(("Full suite", "in_progress", None),
                   ("Storyboard passes (1/4)", "completed", "failure"))]),
         1, "red")

    case("the suite alone passing is not green",
         run([runs("in_progress"), runs("completed", "success")],
             [jobs(("Full suite", "completed", "success"))]),
         0, "run-here")

    case("a run cancelled by a newer push runs the suite here",
         run([runs("completed", "cancelled")], [GREEN]), 0, "run-here")

    case("a cancelled gate runs the suite here",
         run([runs("in_progress")],
             [jobs(("Full suite", "completed", "cancelled"),
                   ("Storyboard passes (1/4)", "in_progress", None))]),
         0, "run-here")

    case("gates skipped by the run run here",
         run([runs("completed", "success")],
             [jobs(("Full suite", "completed", "skipped"),
                   ("Storyboard passes (1/4)", "completed", "skipped"))]),
         0, "run-here")

    case("no run at all after the grace period runs here",
         run([runs(None)], []), 0, "run-here")

    got = run(["FAIL", "FAIL", runs("in_progress")], ["FAIL", GREEN])
    case("an API failure is asked again, not read as an answer", got, 0, "green")

    case("a wait that runs out runs here",
         run([runs("in_progress")], [jobs(("Full suite", "in_progress", None),
                                          ("Storyboard passes (1/4)", "in_progress", None))]),
         0, "run-here")

    if FAILURES:
        print("\nFAILED:", *FAILURES, sep="\n  ")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
