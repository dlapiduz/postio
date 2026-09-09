#!/usr/bin/env python3
"""`issue-land.sh` says what happened when auto-merge cannot be armed (#1355).

`gh pr merge --auto` fails for several reasons that are not failures of the
landing, and under `set -euo pipefail` a bare call made every one of them look
like one: gates passed, branch pushed, PR open -- and a non-zero exit with no
line saying which of those was untrue. On one feature branch four of six
consecutive landings ended that way, for four different reasons. An exit code
that is wrong most of the time is one nobody reads, and then the landing where
a check really did fail is the one that gets missed.

So each known failure returns 0 -- the landing is what the exit code is about
-- and the line differs, naming what will merge the PR and how.

The function is lifted out of the script and run against a stubbed `gh`
rather than the whole landing being driven, because what is under test is the
classification and nothing else. The lift itself is checked: if
`arm_auto_merge` is renamed or its shape changes, extraction fails loudly
rather than testing nothing.

Usage: scripts/tests/test-issue-land-auto-merge-classification.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "issue-land.sh"

# message from `gh`, expected fragment of our output, expected exit status
CASES = [
    (
        "",  # succeeds
        "auto-merge armed on",
        0,
    ),
    (
        "GraphQL: Pull request Pull request is in clean status (enablePullRequestAutoMerge)",
        "no required checks to wait for",
        0,
    ),
    (
        "GraphQL: Pull request Protected branch rules not configured for this branch",
        "the base branch has no required checks",
        0,
    ),
    (
        "HTTP 503: 503 Service Unavailable (https://api.github.com/graphql)",
        "GitHub was unavailable, three times",
        0,
    ),
    (
        "something nobody has seen before",
        "a reason this script does not",
        0,
    ),
]


def lift_function(name: str) -> str:
    """The function's own text, from the script that ships it."""
    source = SCRIPT.read_text(encoding="utf-8")
    match = re.search(rf"^{name}\(\) \{{$", source, re.MULTILINE)
    if not match:
        raise SystemExit(f"{name} is not in {SCRIPT} -- this test is not testing it")
    end = source.index("\n}\n", match.start()) + len("\n}\n")
    return source[match.start() : end]


def main() -> int:
    body = lift_function("arm_auto_merge")
    failures: list[str] = []

    with tempfile.TemporaryDirectory() as tmp:
        stub_dir = Path(tmp) / "bin"
        stub_dir.mkdir()

        for message, expected, expected_status in CASES:
            # `sleep` is stubbed away too: the retrying branches wait fifteen
            # seconds between them, which is right in a landing and absurd in
            # a test.
            (stub_dir / "sleep").write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            (stub_dir / "sleep").chmod(0o755)
            gh = stub_dir / "gh"
            if message:
                gh.write_text(
                    f'#!/usr/bin/env bash\nprintf "%s\\n" {message!r} >&2\nexit 1\n',
                    encoding="utf-8",
                )
            else:
                gh.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            gh.chmod(0o755)

            driver = Path(tmp) / "driver.sh"
            driver.write_text(
                "set -euo pipefail\n"
                'URL="https://example.com/pull/1"\n'
                'ISSUE="42"\n'
                f"{body}\n"
                "arm_auto_merge\n",
                encoding="utf-8",
            )
            result = subprocess.run(
                ["bash", str(driver)],
                capture_output=True,
                text=True,
                env={"PATH": f"{stub_dir}:/usr/bin:/bin"},
                check=False,
            )
            output = result.stdout + result.stderr
            label = message or "(auto-merge armed)"
            if result.returncode != expected_status:
                failures.append(
                    f"{label!r}: exited {result.returncode}, expected "
                    f"{expected_status}. A landing that worked must not report "
                    f"that it did not.\n{output}"
                )
            elif expected not in output:
                failures.append(f"{label!r}: said nothing about {expected!r}.\n{output}")

    if failures:
        print(f"{len(failures)} case(s) failed:\n", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print(f"issue-land auto-merge classification check passed ({len(CASES)} cases).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
