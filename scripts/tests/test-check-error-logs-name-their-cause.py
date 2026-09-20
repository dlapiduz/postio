#!/usr/bin/env python3
"""Self-test for scripts/checks/check-error-logs-name-their-cause.py.

Against a fixture tree, so it proves the direction the check fails in
rather than that today's crates happen to be tidy.

The case that matters most is the last one. `{error}` in a message is
`format_args!` and captures the *local* `error`, not the field of that
name — so where a site logs a deliberately redacted value under that name,
asking it to interpolate would put the unredacted local into `MESSAGE` and
undo the redaction. The check must stay quiet there, and this is what says
so.

Usage: scripts/tests/test-check-error-logs-name-their-cause.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

CHECK = (
    Path(__file__).resolve().parent.parent
    / "checks"
    / "check-error-logs-name-their-cause.py"
)
FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return patience.run(
        ["python3", str(CHECK), "--root", str(root)],
        capture_output=True,
        text=True,
        timeout=30,
    )


def tree(root: Path, source: str) -> None:
    crate = root / "crates" / "postio-fixture" / "src"
    crate.mkdir(parents=True, exist_ok=True)
    (crate / "lib.rs").write_text(source, encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)

        tree(root, 'fn f() { tracing::error!(%error, "could not save the draft"); }\n')
        r = run(root)
        case("a cause in the field only fails", r.returncode != 0, "passed a silent cause")
        case("...and is named", "could not save the draft" in r.stdout, r.stdout)

        tree(root, 'fn f() { tracing::error!(%error, "could not save the draft: {error}"); }\n')
        r = run(root)
        case("a cause in the message passes", r.returncode == 0, r.stdout)

        tree(root, 'fn f() { tracing::error!("nothing went wrong here"); }\n')
        r = run(root)
        case("an error with no cause field passes", r.returncode == 0, r.stdout)

        tree(root, 'fn f() { tracing::warn!(%error, "a warning keeps its own counsel"); }\n')
        r = run(root)
        case("a warn is out of scope", r.returncode == 0, r.stdout)

        tree(
            root,
            'fn f() {\n'
            '    tracing::error!(\n'
            '        %error,\n'
            '        "a message broken across lines; \\\n'
            '         and carried on: {error}"\n'
            '    );\n'
            '}\n',
        )
        r = run(root)
        case("a line-continued message is one literal", r.returncode == 0, r.stdout)

        # The redaction trap, and the reason this check is narrow.
        tree(
            root,
            'fn f() {\n'
            '    tracing::error!(\n'
            '        error = %redact_addresses(&error.to_string()),\n'
            '        "no credential for this account"\n'
            '    );\n'
            '}\n',
        )
        r = run(root)
        case(
            "a redacted cause is left alone",
            r.returncode == 0,
            "asked a site to interpolate the local it redacts: " + r.stdout,
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("check-error-logs-name-their-cause self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
