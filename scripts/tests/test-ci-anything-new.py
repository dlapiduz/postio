#!/usr/bin/env python3
"""Self-test for scripts/ci-anything-new.sh.

The script decides whether a scheduled workflow has anything to do: it
compares `main`'s head against the head the workflow's last completed run
tested. Three workflows ask it -- the nightly, the benches and mutation
testing -- and they disagree on one thing, which is what `--rerun-failed`
is for: the nightly must keep re-running while it is red, because ci.yml's
`Nightly is green` reads its last conclusion and an all-skipped run would
read as green. The other two have nothing reading their conclusion, and a
red run over an unchanged tree only says the same thing again at full price
(mutants: red seven nights of seven, 23% of a week's Linux minutes, #1710).

The direction every failure must go is "run": a missing answer from the API
is not evidence that nothing changed.

`gh` is stubbed on PATH and prints a fixed answer, or fails.

Usage: scripts/tests/test-ci-anything-new.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "ci-anything-new.sh"

FAILURES: list[str] = []

# Prints whatever `$STUB_DIR/reply` holds, after recording its arguments so
# a case can check which workflow was asked about. An absent reply file is
# the API failing.
GH_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/args"
if [ ! -f "$STUB_DIR/reply" ]; then
    echo "HTTP 502" >&2
    exit 1
fi
cat "$STUB_DIR/reply"
"""

HEAD = "a" * 40
OTHER = "b" * 40


def run(*, event: str, reply: str | None, args: list[str]) -> tuple[subprocess.CompletedProcess, str]:
    with tempfile.TemporaryDirectory() as tmp:
        stub_dir = Path(tmp)
        bin_dir = stub_dir / "bin"
        bin_dir.mkdir()
        (bin_dir / "gh").write_text(GH_STUB, encoding="utf-8")
        (bin_dir / "gh").chmod(0o755)
        if reply is not None:
            (stub_dir / "reply").write_text(reply, encoding="utf-8")

        env = dict(os.environ)
        env["PATH"] = f"{bin_dir}:{env['PATH']}"
        env["STUB_DIR"] = str(stub_dir)
        env["GITHUB_REPOSITORY"] = "example/postio"
        env["GITHUB_EVENT_NAME"] = event
        env["GITHUB_SHA"] = HEAD
        env["GITHUB_REF_NAME"] = "main"
        proc = patience.run(
            ["bash", str(SCRIPT), *args],
            capture_output=True,
            text=True,
            env=env,
            stdin=subprocess.DEVNULL,
            timeout=30,
        )
        asked = (stub_dir / "args").read_text(encoding="utf-8") if (stub_dir / "args").exists() else ""
        return proc, asked


def case(
    label: str,
    *,
    expect: str,
    event: str = "schedule",
    reply: str | None,
    args: list[str] | None = None,
    expect_asked: str = "",
) -> None:
    proc, asked = run(event=event, reply=reply, args=args or ["mutants.yml"])
    if proc.returncode != 0:
        FAILURES.append(
            f"{label}: exited {proc.returncode}\n"
            f"  stdout: {proc.stdout.strip()}\n  stderr: {proc.stderr.strip()}"
        )
        return
    # stdout is exactly what a workflow appends to $GITHUB_OUTPUT, so it has
    # to be the one line and nothing else -- prose goes to stderr.
    if proc.stdout != f"run={expect}\n":
        FAILURES.append(
            f"{label}: expected stdout 'run={expect}', got {proc.stdout!r}\n"
            f"  stderr: {proc.stderr.strip()}"
        )
        return
    if expect_asked and expect_asked not in asked:
        FAILURES.append(f"{label}: expected the API to be asked about {expect_asked!r}, got {asked!r}")
        return
    print(f"ok  {label}")


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    case(
        "an unchanged tree after a green run is skipped",
        reply=f"success\t{HEAD}",
        expect="false",
        expect_asked="workflows/mutants.yml/runs",
    )
    # Only `main`'s runs count. A full suite run on a feature branch -- an
    # agent asking for one before landing -- must not become "the last run"
    # a scheduled one compares against (2026-10-01).
    case(
        "the last run is looked up on the branch being run, not any branch",
        reply=f"success\t{HEAD}",
        expect="false",
        expect_asked="branch=main",
    )
    case(
        "new commits since the last run are run",
        reply=f"success\t{OTHER}",
        expect="true",
    )
    case(
        "a skipped run over the same head chains into another skip",
        reply=f"skipped\t{HEAD}",
        expect="false",
    )
    # The reason for the flag: nothing reads mutants' conclusion, so saying
    # "red" again over the same tree is pure cost.
    case(
        "a red run over an unchanged tree is skipped by default",
        reply=f"failure\t{HEAD}",
        expect="false",
    )
    # ...and the reason the nightly passes it.
    case(
        "with --rerun-failed, a red run over an unchanged tree runs again",
        reply=f"failure\t{HEAD}",
        args=["nightly.yml", "--rerun-failed"],
        expect="true",
        expect_asked="workflows/nightly.yml/runs",
    )
    case(
        "with --rerun-failed, a timed-out run is red too",
        reply=f"timed_out\t{HEAD}",
        args=["nightly.yml", "--rerun-failed"],
        expect="true",
    )
    case(
        "with --rerun-failed, a green unchanged tree is still skipped",
        reply=f"success\t{HEAD}",
        args=["nightly.yml", "--rerun-failed"],
        expect="false",
    )
    # Only the timer is ever skipped: a dispatch is somebody asking now, and
    # a workflow_call is a pull request proving it fixed something.
    for event in ("workflow_dispatch", "workflow_call", "pull_request"):
        case(
            f"a {event} always runs",
            event=event,
            reply=f"success\t{HEAD}",
            expect="true",
        )
    # Fail towards running: no answer is not "nothing changed".
    case("an API failure runs", reply=None, expect="true")
    case("no completed run yet runs", reply="", expect="true")
    case("a null answer runs", reply="null", expect="true")

    proc, _ = run(event="schedule", reply="", args=[])
    if proc.returncode == 0:
        FAILURES.append("no workflow named: expected a usage error, got exit 0")
    else:
        print("ok  no workflow named is a usage error")

    if FAILURES:
        print("\n".join(FAILURES), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
