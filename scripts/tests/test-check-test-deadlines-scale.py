#!/usr/bin/env python3
"""Self-test for scripts/checks/check-test-deadlines-scale.py.

The check exists because #842's dial stops at the edge of
`postio_test_support`. Every deadline that goes through that crate answers to
`POSTIO_TEST_PATIENCE`; the forty-six written by hand do not, and all three
of the `gtk_suite` cases #957 names are among them. So the one lever a
session has for a local full-suite run on a loaded box -- turn the dial up --
reaches everything except the tests that flake.

The interesting half is the exception. Some deadlines *are* the subject: a
debounce window, or a negative assertion whose strength is the time it
waited. Scaling those changes what the test proves. They are allowed to stay
fixed and must say why, because a bare marker is a silencer and the next
person cannot tell a load-bearing number from one nobody revisited.

Usage: scripts/tests/test-check-test-deadlines-scale.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-test-deadlines-scale.py"

FAILURES: list[str] = []

HAND_ROLLED = """use std::time::{Duration, Instant};
#[test]
fn waits() {
    let deadline = Instant::now() + Duration::from_secs(20);
    assert!(deadline > Instant::now());
}
"""

QUALIFIED = """#[test]
fn waits() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    assert!(deadline > std::time::Instant::now());
}
"""

SCALED = """use std::time::{Duration, Instant};
use postio_test_support::scaled;
#[test]
fn waits() {
    let deadline = Instant::now() + scaled(Duration::from_secs(20));
    assert!(deadline > Instant::now());
}
"""

ANNOTATED = """use std::time::{Duration, Instant};
#[test]
fn waits() {
    // POSTIO-FIXED-DEADLINE: the debounce window is what this asserts.
    let deadline = Instant::now() + Duration::from_millis(60);
    assert!(deadline > Instant::now());
}
"""

BARE_MARKER = """use std::time::{Duration, Instant};
#[test]
fn waits() {
    // POSTIO-FIXED-DEADLINE:
    let deadline = Instant::now() + Duration::from_millis(60);
    assert!(deadline > Instant::now());
}
"""

# A helper documents its own fixed deadline where the reader meets it -- in
# the doc comment, not squeezed onto the `let`.
ANNOTATED_ON_THE_HELPER = """use std::time::{Duration, Instant};
/// Turn the loop for `time`, so a timeout gets to fire.
///
/// POSTIO-FIXED-DEADLINE: the caller passes the dwell window it is proving
/// did not elapse; scaling it would prove nothing.
fn wait(time: Duration) {
    let deadline = Instant::now() + time;
    assert!(deadline > Instant::now());
}
#[test]
fn waits() { wait(Duration::from_millis(60)); }
"""

# A deadline is a deadline wherever it is spelled; `patience()` is the other
# way through the dial.
PATIENCE = """use std::time::Instant;
#[test]
fn waits() {
    let deadline = Instant::now() + postio_test_support::patience();
    assert!(deadline > Instant::now());
}
"""

# A deadline computed once and reused. Refusing this would push every call
# site into repeating the `scaled` call, so the check follows the binding --
# one hop, same file.
VIA_BINDING = """use std::time::{Duration, Instant};
use postio_test_support::scaled;
#[test]
fn waits() {
    let limit = scaled(Duration::from_secs(20));
    let deadline = Instant::now() + limit;
    assert!(deadline > Instant::now());
}
"""

# The same shape with nothing scaled behind it is still a finding: following
# the binding must not become a way through.
VIA_UNSCALED_BINDING = """use std::time::{Duration, Instant};
#[test]
fn waits() {
    let limit = Duration::from_secs(20);
    let deadline = Instant::now() + limit;
    assert!(deadline > Instant::now());
}
"""

# rustfmt breaks a long `let deadline = ...` across lines. A check the
# formatter can silence is not a check.
WRAPPED = """use std::time::{Duration, Instant};
#[test]
fn waits() {
    let deadline =
        Instant::now() + Duration::from_secs(20);
    assert!(deadline > Instant::now());
}
"""

# `Instant::now() < deadline` is reading the clock, not setting a deadline.
LOOP_CONDITION = """use std::time::Instant;
#[test]
fn waits() {
    let deadline = Instant::now() + postio_test_support::patience();
    while Instant::now() < deadline {}
}
"""

# Source is not a test, so it is not this check's business: the app's own
# timeouts are product behaviour, not test patience.
IN_SOURCE = """use std::time::{Duration, Instant};
pub fn give_up_eventually() {
    let deadline = Instant::now() + Duration::from_secs(20);
    let _ = deadline;
}
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def build(root: Path, *, tests: dict[str, str], source: str = "// x\n") -> None:
    crate = root / "crates" / "postio-thing"
    (crate / "tests").mkdir(parents=True)
    (crate / "src").mkdir(parents=True)
    (crate / "src" / "lib.rs").write_text(source, encoding="utf-8")
    (crate / "Cargo.toml").write_text(
        '[package]\nname = "postio-thing"\nversion = "0.1.0"\n', encoding="utf-8"
    )
    for name, body in tests.items():
        path = crate / "tests" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")
    subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True)
    for args in (
        ["add", "-A"],
        ["-c", "user.email=t@example.com", "-c", "user.name=T", "commit", "-qm", "x"],
    ):
        subprocess.run(["git", *args], cwd=root, check=True)


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECK)], cwd=root, capture_output=True, text=True
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)

        # -- 1. the shape #957 flakes on ------------------------------------
        root = base / "hand-rolled"
        build(root, tests={"suite/case.rs": HAND_ROLLED})
        result = run(root)
        case(
            "a hand-rolled deadline the dial cannot reach fails",
            result.returncode == 1,
            f"exit {result.returncode}: this is gtk_composer_toolbar's 20s,"
            f" accepted:\n{result.stdout}{result.stderr}",
        )
        case(
            "and the report names the file and line",
            "suite/case.rs:4" in result.stderr,
            f"the failure does not say where to look:\n{result.stderr}",
        )

        # -- 2. reached through a full path too -----------------------------
        root = base / "qualified"
        build(root, tests={"case.rs": QUALIFIED})
        case(
            "std::time::Instant::now() is the same deadline",
            run(root).returncode == 1,
            "a fully-qualified path is how most of the suite spells it",
        )

        # -- 3. scaled is the fix -------------------------------------------
        root = base / "scaled"
        build(root, tests={"suite/case.rs": SCALED})
        result = run(root)
        case(
            "a scaled deadline passes",
            result.returncode == 0,
            f"exit {result.returncode}: {result.stdout}{result.stderr}",
        )

        root = base / "patience"
        build(root, tests={"case.rs": PATIENCE})
        case(
            "patience() is the other way through the dial",
            run(root).returncode == 0,
            "the shared default deadline must not be reported",
        )

        # -- 4. a duration that is the subject may stay fixed, with a reason -
        root = base / "annotated"
        build(root, tests={"case.rs": ANNOTATED})
        result = run(root)
        case(
            "a fixed deadline with a stated reason passes",
            result.returncode == 0,
            f"exit {result.returncode}: a debounce window is the subject,"
            f" not the patience:\n{result.stdout}{result.stderr}",
        )

        root = base / "on-the-helper"
        build(root, tests={"case.rs": ANNOTATED_ON_THE_HELPER})
        case(
            "the reason may sit in the helper's doc comment",
            run(root).returncode == 0,
            "a helper documents its fixed deadline where the reader meets it",
        )

        # -- 5. a bare marker is a silencer ---------------------------------
        root = base / "bare"
        build(root, tests={"case.rs": BARE_MARKER})
        result = run(root)
        case(
            "a marker with no reason after it is refused",
            result.returncode == 1,
            f"exit {result.returncode}: a bare marker turns the check off and"
            f" tells the next person nothing:\n{result.stdout}{result.stderr}",
        )
        case(
            "and says the reason is what is missing",
            "no reason" in result.stderr,
            f"the report does not explain the refusal:\n{result.stderr}",
        )

        # -- 6. a deadline bound above is followed, one hop ------------------
        root = base / "binding"
        build(root, tests={"case.rs": VIA_BINDING})
        result = run(root)
        case(
            "a deadline bound to a scaled duration above it passes",
            result.returncode == 0,
            f"exit {result.returncode}: computing the limit once is the normal"
            f" shape:\n{result.stdout}{result.stderr}",
        )

        root = base / "unscaled-binding"
        build(root, tests={"case.rs": VIA_UNSCALED_BINDING})
        case(
            "and an unscaled binding is still a finding",
            run(root).returncode == 1,
            "following the binding must not become a way through the check",
        )

        # -- 7. the formatter cannot silence it -----------------------------
        root = base / "wrapped"
        build(root, tests={"case.rs": WRAPPED})
        case(
            "a deadline rustfmt wrapped across lines is still found",
            run(root).returncode == 1,
            "a check the formatter can silence is not a check",
        )

        root = base / "loop-condition"
        build(root, tests={"case.rs": LOOP_CONDITION})
        result = run(root)
        case(
            "reading the clock in a loop condition is not a deadline",
            result.returncode == 0,
            f"exit {result.returncode}: `Instant::now() < deadline` sets"
            f" nothing:\n{result.stdout}{result.stderr}",
        )

        # -- 8. product code keeps its own timeouts -------------------------
        root = base / "source"
        build(root, tests={"case.rs": SCALED}, source=IN_SOURCE)
        case(
            "a deadline in src/ is not this check's business",
            run(root).returncode == 0,
            "the app's own timeouts are product behaviour, not test patience",
        )

    if FAILURES:
        print("\nFAILED", file=sys.stderr)
        for failure in FAILURES:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print("\ntest-check-test-deadlines-scale: all cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
