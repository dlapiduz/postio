#!/usr/bin/env python3
"""Run `deny.toml` — the dependency policy nothing was running.

`deny.toml` states which licences Postio's dependency graph may carry, which
crates are banned, and that everything comes from crates.io. It has been in
the repository for a long time and no gate ever executed it, so it was a
policy in the sense that a sign is a policy.

What that cost, found in August 2026: `postio-gmail`, `postio-jmap` and
`postio-ui` each declared ``license = "GPL-3.0-or-later"`` in a workspace
whose root says ``MIT`` and whose other fifteen crates inherit it. Not a
relicensing — those three had never picked up workspace inheritance at all,
so they were also silently ``publish = true`` (the default) in a workspace
that says nothing is publishable, and carried no ``rust-version``. Three
manifests drifted away from the other fifteen and every gate stayed green.
Issue #639.

This is the same shape as `check-toolchain-pinned.py`: a rule that was
written down, believed, and unenforced.

# Which checks run here, and which deliberately do not

``bans``, ``licenses`` and ``sources`` run. They are deterministic — the same
tree gives the same answer today and next year — and they need no network.

``advisories`` does **not** run here, and that is not an oversight. The RustSec
database changes underneath you: an advisory published overnight would fail an
unrelated PR on a dependency the author never touched, on a morning nobody
chose. A gate that can go red without the tree changing teaches people to rerun
it until it passes, which is the opposite of what a gate is for. Security
advisories want a scheduled job that opens an issue, not a blocking check —
run ``cargo deny check advisories`` on its own for that.

# Why a missing cargo-deny is a skip rather than a failure

``mise.toml`` pins the tools the gates run on and leaves the four cargo-based
ones out on purpose, because each is a from-source build measured in minutes
and ``mise install`` is the first thing a newcomer runs. Hard-failing here
would make that decision untenable: a fresh clone could not pass `check.sh`
until it had built cargo-deny.

So this skips when the tool is absent — but says so on stdout, every time,
rather than passing silently. A check that can quietly do nothing is how
`deny.toml` got into this state; the notice is what keeps the gap visible on
any machine that has chosen not to close it.

Install it with ``cargo install cargo-deny`` (or add it to ``mise.toml`` if
the trade-off above is ever revisited).
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

# Deterministic, offline, and about this tree rather than about the world.
# See the module docstring for why `advisories` is not among them.
CHECKS = ["bans", "licenses", "sources"]

# cargo-deny takes a while on a cold registry and should never be the thing
# that hangs a land. Generous enough for a cold run, bounded enough to fail
# visibly rather than sit there.
TIMEOUT_SECONDS = 300


def main() -> int:
    root = Path(__file__).resolve().parents[2]

    if not (root / "deny.toml").is_file():
        print(
            "dependency-policy check FAILED: deny.toml is missing.\n"
            "  It states which licences the dependency graph may carry and "
            "where crates may come from.\n"
            "  Restore it, or delete this check deliberately rather than "
            "leaving a policy nothing reads.",
            file=sys.stderr,
        )
        return 1

    if shutil.which("cargo-deny") is None:
        # Loud, not silent. See the module docstring.
        print(
            "dependency-policy check SKIPPED: cargo-deny is not installed, so "
            "deny.toml went unread.\n"
            "  Install it with `cargo install cargo-deny` to close this gap. "
            "It is left out of mise.toml\n"
            "  on purpose (a from-source build measured in minutes), which is "
            "why this is a skip rather than a failure."
        )
        return 0

    try:
        result = subprocess.run(
            ["cargo", "deny", "check", *CHECKS],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired:
        print(
            f"dependency-policy check FAILED: cargo-deny did not finish within "
            f"{TIMEOUT_SECONDS}s.\n"
            "  Run `cargo deny check` by hand to see where it is stuck.",
            file=sys.stderr,
        )
        return 1
    except OSError as error:
        print(
            f"dependency-policy check FAILED: could not run cargo-deny: {error}",
            file=sys.stderr,
        )
        return 1

    if result.returncode != 0:
        # cargo-deny's own diagnostics name the crate, the licence and the path
        # that pulled it in, which is more than this script could reconstruct.
        # Its output goes to stderr; pass it through rather than summarising.
        sys.stderr.write(result.stderr)
        print(
            "\ndependency-policy check FAILED: see cargo-deny's output above.\n"
            "  A licence rejection on one of Postio's own crates usually means "
            "that crate's [package]\n"
            "  block is missing `license.workspace = true` rather than that "
            "anybody chose a licence --\n"
            "  compare it against crates/postio-storage/Cargo.toml, which "
            "inherits all six keys (#639).",
            file=sys.stderr,
        )
        return 1

    print(f"dependency-policy check passed ({', '.join(CHECKS)}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
