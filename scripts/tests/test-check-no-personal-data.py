#!/usr/bin/env python3
"""Self-test for scripts/checks/check-no-personal-data.py.

The check guards something that is usually absent, so on a clean tree it
passes whether it works or not — the shape of check that rots silently. It
also earned a self-test the hard way: it read **every** tracked file as UTF-8
with `errors="ignore"`, binaries included, and `errors="ignore"` *deletes*
invalid bytes rather than replacing them. Deleting bytes splices whatever was
on either side of them together, so a PNG's compressed pixel data produced a
perfectly-formed email address that does not exist anywhere in the file. That
failure blocked every landing in the repository until it was found, and it
was found by a session that had nothing to do with the image.

The two halves of the fix have to be tested together, because either one
alone would let a real leak through:

- **Skip binary files** (a NUL byte in the first 8 KiB, which is git's own
  test). A screenshot's text is pixels; there is nothing here to scan.
- **Decode the rest with `errors="replace"`**, so a byte that cannot be
  decoded becomes a separator rather than vanishing. `.eml` corpus fixtures
  are legitimately not UTF-8 — skipping every file that fails a strict decode
  would turn the guard off over exactly the files that describe mailboxes,
  which is why the discriminator is NUL and not decodability.

This file is itself in the check's `SKIP_PATHS`, for the reason the check
script is: proving the guard fires on a forbidden address means holding one.
The addresses below are invented and the domain is not reserved on purpose --
that is the whole point of them.

Usage: scripts/tests/test-check-no-personal-data.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-no-personal-data.py"

FAILURES: list[str] = []


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=bot@example.com", "-c", "user.name=Bot", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def run(root: Path, *args: str, deny: str = "") -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["POSTIO_DENY_NAMES"] = deny
    return subprocess.run(
        [sys.executable, str(CHECK), *args],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
    )


def case(name: str, files: dict[str, bytes], expect_fail: bool, deny: str = "") -> str:
    """Build a one-off repository holding `files` and check it."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        git("init", "-q", "-b", "main", cwd=root)
        for relative, body in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(body)
        git("add", "-A", cwd=root)
        result = run(root, deny=deny)
        failed = result.returncode != 0
        if failed != expect_fail:
            want = "fail" if expect_fail else "pass"
            FAILURES.append(
                f"{name}: expected the check to {want}\n"
                f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
            )
        return result.stdout + result.stderr


def spliceable_binary() -> bytes:
    """A binary file whose *deleted* bytes would fuse into an address.

    This is `site/assets/img/compose.png`'s failure in miniature: neither
    fragment is an address, and there is no address in the file, but delete
    the invalid UTF-8 between them and one appears.
    """
    return b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR" + b"grace" + b"\xff\xfe" + b"@realdomain.net"


def main() -> int:
    # The bug itself. A screenshot must not be able to invent an address.
    case("a binary file is not scanned", {"site/img/shot.png": spliceable_binary()}, False)

    # ... and the guard must still be a guard.
    case(
        "a real address in a text file still fails",
        {"crates/x/src/lib.rs": b'const WHO: &str = "grace@realdomain.net";\n'},
        True,
    )
    case(
        "a reserved-domain address passes",
        {"crates/x/src/lib.rs": b'const WHO: &str = "ada@example.com";\n'},
        False,
    )

    # A corpus fixture that is legitimately not UTF-8 is text, not binary,
    # and must still be read. Skipping on "does not decode" instead of "has a
    # NUL" would have turned the check off over the mail fixtures.
    case(
        "a latin-1 mail fixture is still scanned",
        {"corpus/latin1.eml": b"From: Gr\xe4ce <grace@realdomain.net>\r\n\r\nbody\r\n"},
        True,
    )

    # Deleting undecodable bytes must not fuse two halves of a text file into
    # an address either -- same defect, same file kind the corpus is made of.
    case(
        "undecodable bytes separate rather than splice",
        {"corpus/split.eml": b"grace\xff\xfe@realdomain.net\n"},
        False,
    )

    output = case(
        "a denied real name fails",
        {"docs/notes.md": "Reviewed by Grace Hopper.\n".encode()},
        True,
        deny="Grace Hopper",
    )
    if "Grace Hopper" in output:
        FAILURES.append(
            "the denied name was echoed into output that is public in CI:\n"
            f"{output}"
        )

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("check-no-personal-data self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
