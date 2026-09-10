#!/usr/bin/env python3
"""Prove the landing gate builds rustdoc, and that the flags it uses can fail.

Nothing on the merge path built rustdoc at all before #1463 — not this gate,
and not CI, whose "Docs site build" job is `mdbook` and is gated on prose
paths. A pull request changing only Rust never ran rustdoc anywhere, so a
broken intra-doc link could only ever be found by the nightly, hours later,
on a job that had never once been green.

Two halves, because either alone is a gate that reports without checking:

* **Structural** — `issue-land.sh` still calls `run_rustdoc` per changed
  crate, in the branch an ordinary landing takes. Read from the source, the
  way `test-issue-land-doctests.py` reads its own.
* **Behavioural** — the flags `run_rustdoc` passes actually turn a broken
  link into a non-zero exit. A gate wired up correctly around
  `RUSTDOCFLAGS` that does not deny warnings is the failure this half exists
  to catch, and it is invisible to the structural half.

The behavioural half builds a dep-free crate in a temporary directory, well
outside this repository, so it picks up none of `.cargo/config.toml` and
needs no network.

Usage: scripts/tests/test-issue-land-rustdoc.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

# `scripts/lib` is not a package; the CI step that discovers self-tests runs
# every `scripts/tests/*.py` it finds, so the path goes on `sys.path` here the
# way the neighbouring tests do it.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

SCRIPT = Path(__file__).resolve().parent.parent / "issue-land.sh"
PROBLEMS: list[str] = []

MANIFEST = """[package]
name = "linkprobe"
version = "0.0.0"
edition = "2021"

[lib]
path = "lib.rs"

[workspace]
"""


def fail(what: str) -> None:
    print(f"FAIL  {what}", file=sys.stderr)
    PROBLEMS.append(what)


def structural() -> None:
    source = SCRIPT.read_text(encoding="utf-8")

    body = re.search(r"run_rustdoc\(\)\s*\{(.*?)\n\}", source, re.S)
    if not body:
        fail("run_rustdoc is gone or reshaped; this test cannot find it")
        return
    helper = body.group(1)
    for flag in ("-D warnings", "--no-deps", "--document-private-items"):
        if flag not in helper:
            fail(f"run_rustdoc no longer passes {flag!r}: {helper.strip()!r}")
    if "cargo doc" not in helper:
        fail(f"run_rustdoc does not build docs: {helper.strip()!r}")

    # Per changed crate, in the loop that always runs -- not the workspace,
    # which is the 23m35s job #833 moved off the merge path for good reasons.
    if 'run_rustdoc -p "$crate"' not in source:
        fail(
            "the landing does not build rustdoc per changed crate. A broken "
            "intra-doc link is then the nightly's to find, hours later "
            "(#1463)"
        )
    if re.search(r"run_rustdoc\s+--workspace", source):
        fail(
            "the landing builds rustdoc for the whole workspace; that is the "
            "long pole #833 removed from every pull request"
        )


def rustdoc_exit(doc_line: str, tmp: Path) -> subprocess.CompletedProcess[str]:
    """Build a one-file crate whose lib doc is `doc_line`, under the gate's flags."""
    (tmp / "Cargo.toml").write_text(MANIFEST, encoding="utf-8")
    (tmp / "lib.rs").write_text(
        f"//! {doc_line}\n\n/// A real item, so there is something to link to.\npub fn thing() {{}}\n",
        encoding="utf-8",
    )
    environment = dict(os.environ)
    environment["RUSTDOCFLAGS"] = "-D warnings -A rustdoc::private_intra_doc_links"
    environment["CARGO_TARGET_DIR"] = str(tmp / "target")
    return patience.run(
        ["cargo", "doc", "--no-deps", "--document-private-items", "--offline"],
        cwd=tmp, env=environment, capture_output=True, text=True, timeout=180,
    )


def behavioural() -> None:
    with tempfile.TemporaryDirectory() as raw:
        good = rustdoc_exit("Links to [`thing`], which exists.", Path(raw))
        if good.returncode != 0:
            fail(
                "a crate whose links all resolve did not document cleanly, so "
                f"this test cannot tell a real failure from its own: {good.stderr[-400:]}"
            )
            return

    with tempfile.TemporaryDirectory() as raw:
        bad = rustdoc_exit("Links to [`no_such_item`], which does not.", Path(raw))
        if bad.returncode == 0:
            fail(
                "a broken intra-doc link documented cleanly under the gate's "
                "RUSTDOCFLAGS -- the flags do not deny what they are there to deny"
            )
        elif "no_such_item" not in bad.stderr:
            fail(
                "the failure does not name the broken link, so a landing that "
                f"trips it says nothing useful: {bad.stderr[-400:]}"
            )


def main() -> int:
    structural()
    behavioural()
    if PROBLEMS:
        print(f"\n{len(PROBLEMS)} problem(s).", file=sys.stderr)
        return 1
    print("issue-land rustdoc gate check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
