#!/usr/bin/env python3
"""Self-test for scripts/pre-commit.py, run as the real git hook.

Each case commits in a sandbox repository wired the way issue-claim.sh
wires a worktree (core.hooksPath = .githooks), so what is asserted is what
a commit does: refused, with the file and the fix named, or recorded.

Usage: scripts/tests/test-pre-commit.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
FAILURES: list[str] = []
# Built, not written: a literal address on an unreserved domain in this file
# would trip the very check it exercises when the repository is scanned.
REAL_ADDRESS = "grace" + "@" + "realdomain" + ".net"

FORMATTED = "fn main() {}\n"
UNFORMATTED = "fn  main(){}\n"


def env() -> dict[str, str]:
    e = {k: v for k, v in os.environ.items() if k != "RUSTUP_TOOLCHAIN"}
    e.update(GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1", POSTIO_DENY_NAMES="Gracewell")
    return e


def git(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-C", str(repo), "-c", "user.name=Ada", "-c", "user.email=ada@example.com", *args],
        capture_output=True, text=True, env=env(),
    )


def sandbox(scratch: Path) -> Path:
    repo = scratch / "repo"
    for rel in ("scripts/pre-commit.py", "scripts/checks/check-no-personal-data.py",
                ".githooks/pre-commit"):
        (repo / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / rel, repo / rel)
    subprocess.run(["git", "init", "-q", "-b", "main", str(repo)], check=True, env=env())
    git(repo, "config", "core.hooksPath", ".githooks")
    git(repo, "add", ".")
    git(repo, "commit", "-qm", "chore(scripts): start", "--no-verify")
    return repo


def attempt(case: str, files: dict[str, str], ok: bool, says: tuple[str, ...] = (),
            after_staging: dict[str, str] | None = None) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        repo = sandbox(Path(tmp))
        for rel, text in files.items():
            (repo / rel).parent.mkdir(parents=True, exist_ok=True)
            (repo / rel).write_text(text)
            git(repo, "add", rel)
        for rel, text in (after_staging or {}).items():
            (repo / rel).write_text(text)
        result = git(repo, "commit", "-qm", "test(core): a case")
        output = result.stdout + result.stderr
        if (result.returncode == 0) != ok:
            FAILURES.append(f"{case}: expected {'commit' if ok else 'refusal'}, got exit "
                            f"{result.returncode}\n{output}")
        elif missing := [s for s in says if s not in output]:
            FAILURES.append(f"{case}: output does not say {missing}:\n{output}")
        elif REAL_ADDRESS in output or "Gracewell" in output:
            FAILURES.append(f"{case}: the refusal printed the value it protects:\n{output}")
        else:
            print(f"ok  {case}")


if shutil.which("rustfmt") is None:
    print("rustfmt is not installed here; skipping")
    sys.exit(77)

attempt("formatted Rust commits", {"src/lib.rs": FORMATTED}, True)
attempt("unformatted Rust is refused, naming the file and the fix",
        {"src/lib.rs": UNFORMATTED}, False, ("src/lib.rs", "rustfmt --edition 2024"))
attempt("the staged content is what is checked",
        {"src/lib.rs": FORMATTED}, True, after_staging={"src/lib.rs": UNFORMATTED})
attempt("a staged unformatted file is refused even if the tree is fixed",
        {"src/lib.rs": UNFORMATTED}, False, ("src/lib.rs",), after_staging={"src/lib.rs": FORMATTED})
attempt("Rust that does not parse is the compiler's to report",
        {"src/lib.rs": "fn main( {}\n"}, True)
attempt("vendored Rust is not ours to format", {"vendor/x/lib.rs": UNFORMATTED}, True)
attempt("a reserved address commits", {"README.md": "Write to ada@example.com\n"}, True)
attempt("an unreserved address is refused, without printing it",
        {"README.md": f"Write to {REAL_ADDRESS}\n"}, False, ("README.md", "personal data"))
attempt("a denylisted name is refused, without printing it",
        {"README.md": "Thanks to Gracewell\n"}, False, ("README.md", "personal data"))
attempt("both problems are reported at once",
        {"src/lib.rs": UNFORMATTED, "README.md": f"{REAL_ADDRESS}\n"}, False,
        ("src/lib.rs", "README.md"))

if FAILURES:
    print("\n".join(FAILURES), file=sys.stderr)
    sys.exit(1)
