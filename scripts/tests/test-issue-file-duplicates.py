#!/usr/bin/env python3
"""Self-test for scripts/issue-file.sh, the search-before-filing step (#415).

One bug ended up with three issue numbers in two days -- #332, then #392 and
#406 filed by two different sessions. Both duplicates came straight off a
fresh reproduction, which is exactly the state where searching first feels
redundant: you are not wondering whether the bug exists, you just watched it
happen.

So the case that matters is the one that happened: a *closed* prior issue must
stop the filing. A search that only looked at open issues would have found
nothing both times and reported a clean sheet, which is worse than not
searching at all.

`gh` is stubbed on PATH and records what it was asked, so "did it search
before it filed" and "did it file" are both answerable without a network or a
repository.

Usage: scripts/tests/test-issue-file-duplicates.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "issue-file.sh"

FAILURES: list[str] = []

# Prints whatever `$STUB_DIR/found` holds for `issue list`, and records every
# call so the test can assert the *order*: searched, then filed or not.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/found"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then
    echo "https://example.com/issues/999"
    exit 0
fi
exit 0
"""


def run(stub_dir: Path, found: str, *args: str) -> subprocess.CompletedProcess[str]:
    (stub_dir / "found").write_text(found, encoding="utf-8")
    (stub_dir / "calls").write_text("", encoding="utf-8")
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        env=environment, capture_output=True, text=True, timeout=60,
    )


def calls(stub_dir: Path) -> str:
    return (stub_dir / "calls").read_text(encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = Path(directory)
        (stub_dir / "bin").mkdir()
        (stub_dir / "bin" / "gh").write_text(GH_STUB, encoding="utf-8")
        (stub_dir / "bin" / "gh").chmod(0o755)

        body = stub_dir / "body.md"
        body.write_text("what happened\n", encoding="utf-8")
        title = "issue-land.sh reports MERGE DID NOT LAND for merges that did land"

        # ── the incident: a CLOSED prior issue ───────────────────────────
        prior = "  #332 CLOSED\tissue-land's merge verification can false-negative\n"
        result = run(stub_dir, prior, "--title", title, "--body-file", str(body))
        if result.returncode != 2:
            FAILURES.append(
                "#415: a duplicate should stop the filing and exit 2, got "
                f"{result.returncode}:\n{result.stdout}\n{result.stderr}"
            )
        if "issue create" in calls(stub_dir):
            FAILURES.append(
                "#415: it filed anyway -- the whole point is that a closed "
                f"prior issue is still prior art:\n{calls(stub_dir)}"
            )
        if "#332" not in result.stdout:
            FAILURES.append(f"it did not show what it found:\n{result.stdout}")
        if "comment" not in result.stderr:
            FAILURES.append(
                "a session told 'no' needs to be told what to do instead:\n"
                f"{result.stderr}"
            )

        # ── the search really was a search ──────────────────────────────
        asked = calls(stub_dir)
        if "--state all" not in asked:
            FAILURES.append(
                "the search must include closed issues -- the duplicate that "
                f"started this was filed against a closed one:\n{asked}"
            )
        if "issue-land.sh" not in asked:
            FAILURES.append(
                f"the title's distinctive words are what it searches on:\n{asked}"
            )

        # ── nothing similar: it files ───────────────────────────────────
        result = run(stub_dir, "", "--title", title, "--body-file", str(body))
        if result.returncode != 0:
            FAILURES.append(
                f"a genuinely new issue was refused:\n{result.stdout}\n{result.stderr}"
            )
        if "issue create" not in calls(stub_dir):
            FAILURES.append(f"it did not file anything:\n{calls(stub_dir)}")

        # ── --anyway: read it, decided it is different ───────────────────
        result = run(stub_dir, prior, "--title", title, "--body-file", str(body), "--anyway")
        if result.returncode != 0:
            FAILURES.append(f"--anyway did not file:\n{result.stderr}")
        if "issue create" not in calls(stub_dir):
            FAILURES.append("--anyway must still file after showing the matches")
        if "#332" not in result.stdout:
            FAILURES.append(
                "--anyway must still *show* what it found; skipping the search "
                "would make the flag a way to never look"
            )

        # ── --search-only: never files, whatever it finds ────────────────
        result = run(stub_dir, prior, "--search-only", "--title", title)
        if result.returncode != 0 or "issue create" in calls(stub_dir):
            FAILURES.append("--search-only must report and file nothing")

        # ── widening: three terms find nothing, two do ──────────────────
        # #332 called this bug "issue-land's merge verification can
        # false-negative" and #392 called it "reports MERGE DID NOT LAND".
        # Three terms from either title finds neither of the other, because
        # GitHub ANDs them -- so a search that never widened would have
        # reported a clean sheet for the exact duplicate this exists to stop.
        widening = stub_dir / "bin" / "gh"
        widening.write_text(
            """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    # Two terms match; three do not. The word count is the stand-in for
    # GitHub's AND, which is what makes a narrow search miss.
    terms=$(printf '%s' "$*" | sed 's/.*--search //; s/ --state.*//')
    if [ "$(printf '%s' "$terms" | wc -w)" -le 2 ]; then
        cat "$STUB_DIR/found"
    fi
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then echo "created"; exit 0; fi
exit 0
""",
            encoding="utf-8",
        )
        widening.chmod(0o755)

        result = run(stub_dir, prior, "--title", title, "--body-file", str(body))
        if result.returncode != 2:
            FAILURES.append(
                "#415: a prior issue findable on two terms but not three was "
                f"missed -- the search must widen:\n{result.stdout}\n{result.stderr}"
            )
        if "issue create" in calls(stub_dir):
            FAILURES.append("it filed a duplicate the widened search had found")

        (stub_dir / "bin" / "gh").write_text(GH_STUB, encoding="utf-8")
        (stub_dir / "bin" / "gh").chmod(0o755)

        # ── a body is required to file, and not to search ────────────────
        result = run(stub_dir, "", "--title", title)
        if result.returncode != 1:
            FAILURES.append(
                "filing with no body should be a usage error, not an empty issue"
            )

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-file search-before-filing self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
