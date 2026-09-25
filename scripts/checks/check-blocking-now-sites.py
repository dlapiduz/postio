#!/usr/bin/env python3
"""Refuse a new `blocking::now` in the frontend (#1608).

`postio_session::blocking::now` drives a future to completion on the thread
that calls it. In `postio-app` and `postio-gtk` that thread is the GTK main
thread, and the futures were store reads and writes: a reply read two cold
connections and decoded a body before the composer opened, and every
autosave tick waited on the write permit behind whatever unit a background
sync was committing. CLAUDE.md's rule is that the UI never awaits the
network; this is the same rule for the store.

One use is legitimate by construction: WebKit's `cid:` resolver is a
synchronous foreign callback that cannot be made async. The rest are debt:
settings panels, startup, export, onboarding. They are listed below with
how many each file holds, and the list may only shrink.

# The rule

A file under ``crates/postio-app/src`` or ``crates/postio-gtk/src`` may call
``blocking::now(`` at most as many times as ``ALLOWED`` says, and a file not
in ``ALLOWED`` not at all. A file that holds *fewer* than its allowance must
have its number lowered here, so the list never claims debt that was paid.
The way off the list is the one #1608 took: read on the runtime and answer
through a channel the main context awaits.

# Exit status

0 clean, 1 a site was added or an allowance is stale, 2 the check could not
run.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOTS = ["crates/postio-app/src", "crates/postio-gtk/src"]
NEEDLE = "blocking::now("

# file -> how many `blocking::now(` calls it may hold. May only shrink.
ALLOWED = {
    "crates/postio-app/src/add_account.rs": 2,
    # install_resume (2), install_autosave's one-time recovery at mount (1).
    "crates/postio-app/src/compose.rs": 3,
    "crates/postio-app/src/lib.rs": 1,
    "crates/postio-app/src/onboarding.rs": 1,
    "crates/postio-app/src/orientation.rs": 2,
    "crates/postio-app/src/reading.rs": 1,
    "crates/postio-app/src/search.rs": 1,
    "crates/postio-app/src/settings_accounts.rs": 9,
    "crates/postio-app/src/settings_credential.rs": 1,
    "crates/postio-app/src/settings_egress.rs": 1,
    "crates/postio-app/src/settings_privacy.rs": 1,
    "crates/postio-app/src/sidebar_backfill.rs": 1,
}


def count(path: Path) -> int:
    held = 0
    for line in path.read_text(encoding="utf-8").splitlines():
        code = line.split("//", 1)[0]
        held += code.count(NEEDLE)
    return held


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    found: dict[str, int] = {}
    for base in ROOTS:
        directory = root / base
        if not directory.is_dir():
            print(f"blocking-now check could not run: {base} is missing", file=sys.stderr)
            return 2
        for path in sorted(directory.rglob("*.rs")):
            held = count(path)
            if held:
                found[str(path.relative_to(root))] = held

    problems = []
    for name, held in sorted(found.items()):
        allowed = ALLOWED.get(name, 0)
        if held > allowed:
            problems.append(
                f"  {name}: {held} call(s) to blocking::now, {allowed} allowed"
            )
    for name, allowed in sorted(ALLOWED.items()):
        held = found.get(name, 0)
        if held < allowed:
            problems.append(
                f"  {name}: allowance {allowed} but only {held} left -- lower it "
                f"in scripts/checks/check-blocking-now-sites.py"
            )
    if problems:
        print("blocking-now check FAILED\n")
        print("\n".join(problems))
        print(
            "\n`blocking::now` in the frontend runs a future on the GTK main\n"
            "thread. Read on the runtime and hand the answer back over a\n"
            "channel instead (see `compose::install_reply_source`, #1608)."
        )
        return 1
    print(f"blocking-now check passed ({sum(found.values())} site(s) in {len(found)} file(s), all listed).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
