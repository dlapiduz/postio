"""What a self-test does about a prerequisite it does not have.

The tooling self-tests ran on Linux only until #1151, so every prerequisite
they need was either present on that one runner or absent everywhere, and
"skip when the tool is missing" was a safe-looking shortcut. Running them on
macOS as well ends that: a tool present on `ubuntu-latest` and absent on
`macos-latest` turns a silent skip into a green job that tested nothing, on
exactly the platform the suite was added to cover.

So the rule is the one `gtk_display_required.rs` and
`WindowServerRequiredTests` already state, in the third language this
repository writes gates in:

* **Locally**, a missing prerequisite is a fact about somebody's machine.
  Skip -- and *say so*, or a run that tested half of what it claims looks
  exactly like a clean one.
* **Under CI**, a missing prerequisite is a broken runner. Fail. A skip
  nobody can tell from a pass is not a test.

`verdict` is a pure function of two booleans because the branch that has to
fail is the one no single machine can demonstrate: a box with `cargo-deny`
cannot show the missing case, and a branch whose failing side has never been
seen fail is the untested skip over again. `test-prereq.py` asserts all four.
"""

from __future__ import annotations

import os
import shutil
import sys
from typing import Mapping

#: Present. Nothing to decide.
FINE = "fine"
#: Absent, and that is expected here. Say so, do not fail.
SKIP_AND_SAY = "skip-and-say"
#: Absent where it should not be. A skip here would read as a pass.
FAIL = "fail"


def verdict(*, is_ci: bool, present: bool) -> str:
    """What to do about a prerequisite, given where we are."""
    if present:
        return FINE
    return FAIL if is_ci else SKIP_AND_SAY


def is_ci(environ: Mapping[str, str] | None = None) -> bool:
    """Whether this is a CI runner.

    Read from the environment rather than inferred from anything else --
    every runner sets `CI`, and `scripts/macos-test.sh`'s workflow step sets
    it explicitly for the same reason. An empty value is not CI: a variable
    somebody exported blank should not turn every local skip into a failure.
    """
    if environ is None:
        environ = os.environ
    return bool(environ.get("CI", ""))


#: What a self-test exits with to say it cannot be meaningful here.
#:
#: Not 0 and not 1: `run-self-tests.sh` counts and names these separately, so
#: a Linux-only test on a macOS runner shows up in the log as one that stood
#: down rather than as one that passed. 77 is the convention autotools and
#: `make check` already use for "skipped", so it is the least surprising
#: number to pick and the one least likely to collide with a real exit code.
NOT_APPLICABLE_EXIT = 77


def only_on(platform: str, *, reason: str) -> None:
    """Carry on if this is `platform`; otherwise say why and stand down.

    For a self-test whose subject does not exist here at all -- `test-headless-
    runner.py` drives mutter over a Wayland socket, `test-install-local-*.py`
    install a `.desktop` file and hicolor icons. Those are not broken runners
    and must not fail; what they must not do is return 0, which would claim
    coverage the run does not have.

    `platform` is matched as a prefix of `sys.platform`, so "linux" covers
    `linux` and `linux2` and "darwin" is exact.
    """
    if sys.platform.startswith(platform):
        return
    print(f"not applicable on {sys.platform}: {reason}")
    sys.exit(NOT_APPLICABLE_EXIT)


def have(tool: str) -> bool:
    """Whether `tool` is on PATH. The usual thing a prerequisite is."""
    return shutil.which(tool) is not None


def available(name: str, *, present: bool, environ: Mapping[str, str] | None = None) -> bool:
    """Whether one case may run; `False` after saying what it stood down for.

    The per-case form of [`require`]. Use it when the file also holds cases
    that do *not* need `name` -- exiting the process there would skip them
    too, and a case added below the guarded ones would be skipped by accident,
    which is this module's own failure one level further out.

    Under CI it does not return at all: a missing prerequisite is a broken
    runner however politely the caller asked.
    """
    answer = verdict(is_ci=is_ci(environ), present=present)
    if answer == FINE:
        return True
    if answer == FAIL:
        _fail(name)
    print(f"  skip: {name} is not installed; this case did not run")
    return False


def require(name: str, *, present: bool, environ: Mapping[str, str] | None = None) -> None:
    """Return if `name` is available; otherwise skip or fail, per `verdict`.

    Exits the process rather than returning a flag, because the caller is a
    self-test whose remaining cases cannot mean anything without it. Both
    exits name the prerequisite: a skip that does not say what it skipped is
    the thing this module exists to stop.
    """
    answer = verdict(is_ci=is_ci(environ), present=present)
    if answer == FINE:
        return
    if answer == FAIL:
        _fail(name)
    print(f"skip: {name} is not installed; the cases that need it did not run")
    sys.exit(0)


def _fail(name: str) -> None:
    """Say what is missing and stop. Never returns."""
    print(
        f"FAIL: {name} is missing on a CI runner. A self-test that skips here "
        f"is indistinguishable from one that passed, which is the failure "
        f"this guard exists for -- install {name} on the runner, or say out "
        f"loud that this test does not apply to this platform.",
        file=sys.stderr,
    )
    sys.exit(1)
