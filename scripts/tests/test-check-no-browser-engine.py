#!/usr/bin/env python3
"""Self-test for scripts/checks/check-no-browser-engine.py.

Throwaway trees, one per way an engine can come back, and the real tree,
which must pass only because of the named allowance.

Usage: scripts/tests/test-check-no-browser-engine.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-no-browser-engine.py"
FAILURES: list[str] = []

spec = importlib.util.spec_from_file_location("check", CHECK)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

CLEAN_CRATE = '[package]\nname = "postio-widgets"\n\n[dependencies]\ngtk4 = "0.11"\n'
WEBKIT_CRATE = CLEAN_CRATE + 'webkit6 = { version = "0.6" }\n'
LOCK_WEBKIT = '[[package]]\nname = "webkit2gtk"\nversion = "2.0.1"\n'
FLATPAK_WEBKIT = '{"modules": [{"name": "WebKitGTK", "sources": []}]}\n'


def run(files: dict[str, str]) -> tuple[int, str]:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        for relative, text in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        done = subprocess.run(
            [sys.executable, str(CHECK), "--root", str(root)],
            capture_output=True, text=True, check=False,
        )
        return done.returncode, done.stderr


def case(name: str, files: dict[str, str], expected: int, names: str | None = None) -> None:
    code, err = run(files)
    if code != expected:
        FAILURES.append(f"{name}: exit {code}, expected {expected}\n{err}")
    elif names is not None and names not in err:
        FAILURES.append(f"{name}: the failure does not name {names!r}\n{err}")


case("a clean tree passes", {"crates/postio-widgets/Cargo.toml": CLEAN_CRATE}, 0)
case(
    "webkit6 in a crate the allowance does not name fails",
    {"crates/postio-gtk/Cargo.toml": WEBKIT_CRATE},
    1,
    "crates/postio-gtk/Cargo.toml:6: names `webkit6`",
)
case(
    "webkit2gtk in the lock file fails",
    {"Cargo.lock": LOCK_WEBKIT},
    1,
    "Cargo.lock:2: names `webkit2gtk`",
)
case(
    "a WebKitGTK module in a Flatpak manifest fails",
    {"flatpak/dev.postio.Postio.json": FLATPAK_WEBKIT},
    1,
    "names `WebKitGTK`",
)
case(
    "a crate merely named after webkit is not an engine",
    {"crates/postio-widgets/Cargo.toml": CLEAN_CRATE + 'webkit6-shim-free = "1"\n'},
    0,
)

real = check.violations(HERE.parent)
if real:
    FAILURES.append("the real tree fails:\n" + "\n".join(real))
check.ALLOWED.pop("crates/postio-widgets/Cargo.toml", None)
if not any("crates/postio-widgets/Cargo.toml" in line for line in check.violations(HERE.parent)):
    if (HERE.parent / "crates/postio-widgets/Cargo.toml").read_text().find("webkit6") >= 0:
        FAILURES.append("without its allowance the composer's webkit6 is not reported")

if FAILURES:
    for failure in FAILURES:
        print(f"FAIL: {failure}", file=sys.stderr)
    sys.exit(1)
print("check-no-browser-engine: all cases behaved")
