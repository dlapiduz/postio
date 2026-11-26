#!/usr/bin/env python3
"""Self-test for scripts/checks/check-drivers-keep-parity.py.

Throwaway trees with a copy of the check: a controller with two intents and
two inputs, three drivers, and each way a gap is found, allowed or closed.

Usage: scripts/tests/test-check-drivers-keep-parity.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CHECK = REPO_ROOT / "scripts" / "checks" / "check-drivers-keep-parity.py"
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)
FAILURES: list[str] = []

FOCUS = """pub enum Input {
    Command(CommandId),
    /// The message on screen.
    Shown(Option<MessageId>),
}

pub enum Intent {
    Redraw,
    Toast { text: String },
}
"""
FULL = """use postio_focus::{Input, Intent};
fn f(i: Intent) { match i { Intent::Redraw => {}, Intent::Toast { .. } => {} } }
fn g() { send(Input::Command(c)); send(Input::Shown(None)); }
"""
NO_SHOWN = FULL.replace("send(Input::Shown(None));", "")


def run(drivers: dict[str, str], baseline: str, *args: str):
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        root = Path(directory)
        checks = root / "scripts" / "checks"
        checks.mkdir(parents=True)
        shutil.copy(CHECK, checks / CHECK.name)
        (checks / "driver-parity-baseline.txt").write_text(baseline)
        focus = root / "crates/postio-focus/src/lib.rs"
        focus.parent.mkdir(parents=True)
        focus.write_text(FOCUS)
        for crate in ("postio-gtk", "postio-ffi", "postio-tui"):
            path = root / "crates" / crate / "src" / "lib.rs"
            path.parent.mkdir(parents=True)
            path.write_text(drivers.get(crate, FULL))
        result = subprocess.run([sys.executable, str(checks / CHECK.name), *args], capture_output=True, text=True)
        written = (checks / "driver-parity-baseline.txt").read_text()
    return result, written


def expect(name: str, drivers: dict[str, str], baseline: str, ok: bool, *seen: str) -> None:
    result, _ = run(drivers, baseline)
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}{result.stderr}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


expect("every driver handles everything", {}, "", True)
expect("an input one driver omits fails", {"postio-tui": NO_SHOWN}, "", False, "postio-tui does not handle Input::Shown")
expect("an intent a driver omits fails", {"postio-ffi": FULL.replace("Intent::Toast { .. } => {}", "_ => {}")}, "", False,
       "postio-ffi does not handle Intent::Toast")
expect("an input no driver sends is not a gap", {d: NO_SHOWN for d in ("postio-gtk", "postio-ffi", "postio-tui")}, "", True)
expect("a gap in the baseline passes", {"postio-tui": NO_SHOWN}, "postio-tui Input::Shown  # no window stack\n", True)
expect("a closed gap asks for its line to go", {}, "postio-tui Input::Shown\n", True, "is handled now")
expect("an alias counts", {"postio-tui": FULL.replace("use postio_focus::{Input, Intent};", "use postio_focus::{Input as Told, Intent};")
       .replace("Input::", "Told::")}, "", True)
expect("the frontend's own Input does not count", {"postio-tui": "use postio_focus::Intent;\n" + FULL.split("\n", 1)[1]}, "", False,
       "postio-tui does not handle Input::Shown")
expect("a test module does not count", {"postio-tui": NO_SHOWN + "#[cfg(test)]\nmod tests { fn t() { Input::Shown(None); } }\n"}, "", False,
       "Input::Shown")
expect("a comment does not count", {"postio-tui": NO_SHOWN + "// Input::Shown(None)\n"}, "", False, "Input::Shown")

result, written = run({"postio-tui": NO_SHOWN}, "postio-gtk Intent::Redraw  # gone\npostio-tui Input::Shown  # no window stack\n", "--update")
if written != "# A gap without a why predates this check; a new one needs `# why`.\npostio-tui Input::Shown  # no window stack\n":
    FAILURES.append(f"--update keeps the why and drops the closed gap:\n{written}")

if FAILURES:
    print("\n\n".join(FAILURES))
    sys.exit(1)
print("drivers-keep-parity self-test: ok")
