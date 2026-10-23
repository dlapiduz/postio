#!/usr/bin/env python3
"""Prove `check-no-clock-waits.py` refuses a real wait and allows the rest.

The shape it exists for: a test that sleeps for seconds on the merge path,
which is what made the host's two cases of 2026-10-22 flake under load. The
three ways a sleep is allowed -- a fake clock, the nightly tier's marker on
the case or its file, and a `POSTIO-NOT-A-WAIT:` waiver -- must each pass,
or the check would send every one of them to the nightly.

A guard that has never been seen to fail is a guard nobody should trust.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "scripts" / "checks" / "check-no-clock-waits.py"
MARKER = "POSTIO-MEASUREMENT:"

BARE = """\
//! A suite module.

#[tokio::test]
async fn waits_for_real() {
    tokio::time::sleep(Duration::from_secs(2)).await;
}
"""

PAUSED = BARE.replace("#[tokio::test]", "#[tokio::test(start_paused = true)]")

MARKED_CASE = BARE.replace("#[tokio::test]", f"/// {MARKER} it sleeps two seconds.\n#[tokio::test]")

MARKED_FILE = BARE.replace("//! A suite module.", f"//! A suite module.\n//!\n//! {MARKER} numbers a person reads.")

WAIVED = BARE.replace(
    "    tokio::time::sleep(Duration::from_secs(2)).await;",
    "    // POSTIO-NOT-A-WAIT: the stuck task is the subject.\n"
    "    tokio::spawn(async { tokio::time::sleep(Duration::from_secs(60)).await });",
)

SHORT = BARE.replace("from_secs(2)", "from_millis(50)")

SCALED = BARE.replace("Duration::from_secs(2)", "postio_test_support::scaled(Duration::from_secs(2))")


def run(tmp: Path, body: str) -> subprocess.CompletedProcess[str]:
    root = tmp / "tree"
    (root / "crates" / "p" / "tests" / "suite").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)
    (root / "crates" / "p" / "tests" / "suite" / "waits.rs").write_text(body)
    shutil.copy(CHECK, root / "scripts" / "checks" / CHECK.name)
    return patience.run(
        [sys.executable, str(root / "scripts" / "checks" / CHECK.name)],
        capture_output=True, text=True, timeout=120,
    )


def main() -> int:
    cases = [
        ("a two-second sleep on the merge path fails", BARE, True, "waits.rs:5"),
        ("a scaled two-second sleep still fails", SCALED, True, "waits.rs:5"),
        ("a sleep on a paused tokio clock passes", PAUSED, False, ""),
        ("a sleep in a case the nightly tier owns passes", MARKED_CASE, False, ""),
        ("a sleep in a file the nightly tier owns passes", MARKED_FILE, False, ""),
        ("a waived sleep that is the subject passes", WAIVED, False, ""),
        ("a sub-second sleep passes", SHORT, False, ""),
    ]
    failed = 0
    with tempfile.TemporaryDirectory() as raw:
        for i, (name, body, expect_fail, mentions) in enumerate(cases):
            result = run(Path(raw) / str(i), body)
            ok = (result.returncode != 0) == expect_fail and mentions in result.stderr
            print(f"{'ok   ' if ok else 'FAIL '} {name}")
            if not ok:
                failed += 1
                print(f"       exit {result.returncode}; stderr={result.stderr.strip()[:200]}", file=sys.stderr)
    if failed:
        print(f"\n{failed} case(s) failed.", file=sys.stderr)
        return 1
    print("check-no-clock-waits self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
