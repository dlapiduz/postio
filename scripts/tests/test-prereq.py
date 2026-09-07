#!/usr/bin/env python3
"""The self-tests' own rule about a missing prerequisite.

A self-test that cannot run needs to answer differently depending on where it
is. On a contributor's box a missing `cargo-deny` is a fact about their
machine and skipping is right — but it has to *say so*, or a headless local
run looks identical to a clean one. Under CI a missing prerequisite is a
broken runner, and skipping there is the failure `gtk_display_required.rs`
and `WindowServerRequiredTests` were both written to prevent: **a skip nobody
can tell from a pass is not a test.**

#1151 is why this exists here rather than only in Rust and Swift. The tooling
self-tests ran on Linux only, so `scripts/jobserver.sh`'s `sleep infinity` --
a GNU extension BSD rejects -- sat broken for months with a green board.
Running them on macOS as well means meeting prerequisites that are present on
one runner and absent on the other, and the moment that happens the trap is
open again, one layer down.

The *decision* is asserted for all four combinations rather than only the one
this machine can demonstrate, for the reason `windowServerVerdict` is: a
machine that has `cargo-deny` cannot show the missing case, and a branch
whose failing side has never been seen fail is the untested skip again.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "lib"))

import prereq  # noqa: E402


def case_the_decision_covers_every_combination() -> bool:
    expected = {
        # Present: nothing to decide, wherever we are.
        (True, True): prereq.FINE,
        (False, True): prereq.FINE,
        # Absent on a contributor's box: skip, but say it out loud.
        (False, False): prereq.SKIP_AND_SAY,
        # Absent under CI: the runner is wrong. A skip here is
        # indistinguishable from a pass, which is the whole point.
        (True, False): prereq.FAIL,
    }
    for (is_ci, present), want in expected.items():
        got = prereq.verdict(is_ci=is_ci, present=present)
        if got != want:
            print(
                f"  FAIL: verdict(is_ci={is_ci}, present={present}) "
                f"was {got!r}, wanted {want!r}",
                file=sys.stderr,
            )
            return False
    print("  ok  the decision is the same four answers every time")
    return True


def case_ci_is_read_from_the_environment_not_guessed() -> bool:
    """`CI` is what every runner sets, and what `macos-test.sh` already sets."""
    for value, want in (("true", True), ("1", True), ("", False)):
        got = prereq.is_ci({"CI": value})
        if got != want:
            print(f"  FAIL: CI={value!r} read as {got}, wanted {want}", file=sys.stderr)
            return False
    if prereq.is_ci({}) is not False:
        print("  FAIL: no CI variable at all should not read as CI", file=sys.stderr)
        return False
    print("  ok  CI is read, not inferred")
    return True


def _run(body: str, env_ci: str | None) -> subprocess.CompletedProcess:
    """Run `body` as a script that uses `require`, with CI set or not."""
    import os

    with tempfile.TemporaryDirectory() as tmp:
        script = Path(tmp) / "user.py"
        script.write_text(
            "import sys\n"
            f"sys.path.insert(0, {str(HERE.parent / 'lib')!r})\n"
            "import prereq\n" + body
        )
        environment = dict(os.environ)
        environment.pop("CI", None)
        if env_ci is not None:
            environment["CI"] = env_ci
        return subprocess.run(
            [sys.executable, str(script)],
            capture_output=True,
            text=True,
            env=environment,
        )


def case_a_missing_prerequisite_fails_loudly_under_ci() -> bool:
    result = _run("prereq.require('cargo-deny', present=False)\n", env_ci="true")
    if result.returncode == 0:
        print("  FAIL: a missing prerequisite passed under CI", file=sys.stderr)
        return False
    if "cargo-deny" not in result.stderr:
        print(f"  FAIL: it did not name what was missing: {result.stderr!r}", file=sys.stderr)
        return False
    print("  ok  a missing prerequisite is a failure under CI, and names itself")
    return True


def case_a_missing_prerequisite_skips_out_loud_locally() -> bool:
    result = _run("prereq.require('cargo-deny', present=False)\n", env_ci=None)
    if result.returncode != 0:
        print(f"  FAIL: it failed on a box that simply lacks the tool:\n{result.stderr}",
              file=sys.stderr)
        return False
    said = result.stdout + result.stderr
    if "cargo-deny" not in said or "skip" not in said.lower():
        print(f"  FAIL: it skipped silently: {said!r}", file=sys.stderr)
        return False
    print("  ok  a local skip says what it skipped and why")
    return True


def case_a_present_prerequisite_gets_out_of_the_way() -> bool:
    result = _run(
        "prereq.require('cargo-deny', present=True)\nprint('ran the real thing')\n",
        env_ci="true",
    )
    if result.returncode != 0 or "ran the real thing" not in result.stdout:
        print(f"  FAIL: a present prerequisite stopped the test:\n{result.stderr}",
              file=sys.stderr)
        return False
    print("  ok  a present prerequisite changes nothing")
    return True


def case_the_per_case_form_lets_the_rest_of_the_file_run() -> bool:
    """`require` exits; a file with cases that do *not* need the tool wants
    the other form, or a case added after the guarded ones is skipped by
    accident -- which is the same silent skip one level further out."""
    result = _run(
        "ran = prereq.available('cargo-deny', present=False)\n"
        "print('guarded case ran' if ran else 'guarded case stood down')\n"
        "print('the unguarded case still ran')\n",
        env_ci=None,
    )
    if result.returncode != 0:
        print(f"  FAIL: it stopped the file locally:\n{result.stderr}", file=sys.stderr)
        return False
    if "the unguarded case still ran" not in result.stdout:
        print(f"  FAIL: it exited instead of standing down: {result.stdout!r}", file=sys.stderr)
        return False
    if "cargo-deny" not in result.stdout + result.stderr:
        print("  FAIL: it stood down without saying what it skipped", file=sys.stderr)
        return False
    print("  ok  the per-case form stands one case down, not the file")
    return True


def case_the_per_case_form_still_fails_under_ci() -> bool:
    """The whole point survives the softer form: on a runner, missing is a
    failure however the caller asked."""
    result = _run("prereq.available('cargo-deny', present=False)\n", env_ci="true")
    if result.returncode == 0:
        print("  FAIL: a missing prerequisite passed under CI", file=sys.stderr)
        return False
    # The message, not just the exit status: any bug in this module also
    # exits non-zero, and a test that cannot tell those apart passes for
    # the wrong reason -- which is this file's own subject.
    if "cargo-deny" not in result.stderr or "FAIL" not in result.stderr:
        print(f"  FAIL: it did not fail *about* the prerequisite: {result.stderr!r}",
              file=sys.stderr)
        return False
    print("  ok  standing a case down is still a failure under CI, and says why")
    return True


def case_a_test_for_another_platform_declares_itself_rather_than_skipping() -> bool:
    """Exit 77, and a sentence saying why.

    `test-headless-runner.py` drives mutter over a Wayland socket, and
    `test-install-local-*.py` install a `.desktop` file and hicolor icons.
    Neither means anything on macOS, and neither is a broken runner. What
    they must not do is return 0 -- that is the silent skip, and it would
    claim coverage the macOS job does not have.
    """
    result = _run(
        "prereq.only_on('linux', reason='mutter is not a thing here')\n"
        "print('should never get here')\n",
        env_ci="true",
    )
    if sys.platform.startswith("linux"):
        # On Linux the guard has to get out of the way entirely.
        if result.returncode != 0 or "should never get here" not in result.stdout:
            print(f"  FAIL: it stood down on its own platform:\n{result.stderr}",
                  file=sys.stderr)
            return False
        print("  ok  a Linux-only test runs normally on Linux")
        return True

    if result.returncode != prereq.NOT_APPLICABLE_EXIT:
        print(
            f"  FAIL: stood down with {result.returncode}, not "
            f"{prereq.NOT_APPLICABLE_EXIT}; the runner counts that one",
            file=sys.stderr,
        )
        return False
    if "should never get here" in result.stdout:
        print("  FAIL: it carried on anyway", file=sys.stderr)
        return False
    said = result.stdout + result.stderr
    if "mutter is not a thing here" not in said:
        print(f"  FAIL: it stood down without saying why: {said!r}", file=sys.stderr)
        return False
    print("  ok  a test for another platform says so and stands down, loudly")
    return True


def main() -> int:
    cases = [
        case_the_decision_covers_every_combination,
        case_ci_is_read_from_the_environment_not_guessed,
        case_a_missing_prerequisite_fails_loudly_under_ci,
        case_a_missing_prerequisite_skips_out_loud_locally,
        case_a_present_prerequisite_gets_out_of_the_way,
        case_the_per_case_form_lets_the_rest_of_the_file_run,
        case_the_per_case_form_still_fails_under_ci,
        case_a_test_for_another_platform_declares_itself_rather_than_skipping,
    ]
    return 0 if all([case() for case in cases]) else 1


if __name__ == "__main__":
    sys.exit(main())
