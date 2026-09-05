#!/usr/bin/env python3
"""Self-test for issue #1189: `issue-land.sh --refs-only`.

`issue-land.sh` always wrote `Closes #<n>` into the PR body it opens, which
closes the issue the moment auto-merge (#1107) merges the PR -- minutes
later, with nobody watching. That is right when the PR meets its issue's
acceptance in full, and wrong for a PR that is deliberately partial: a
measurement that changed the question, a fix for one of several acceptance
criteria. PR #1188 had to be hand-edited after the fact to say `Refs`
instead, caught only because the author happened to look before it merged.

`--refs-only` is the flag that says so up front, in the PR body itself, so
the omission reads as a decision rather than something a reviewer has to
notice was missing. Both spellings are checked here because the difference
is one word in generated text that nothing else in the gate chain looks at.

`gh` is stubbed on PATH; every call is logged verbatim, including the full
multi-line `--body` text `pr create` was given, so the assertions read the
same string a reviewer would see on GitHub rather than a stubbed opinion
about it. `--no-merge` throughout, so the script stops right after opening
the PR and never touches `gh pr merge` at all -- this test is about the
body text, not the merge.

Usage: scripts/tests/test-issue-land-refs-only.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"

STUB_CHECKS = [
    "check-crate-boundaries.py",
    "check-no-personal-data.py",
    "check-no-silent-tracking.py",
    "check-toolchain-pinned.py",
    "check-no-gtk-init-in-unit-tests.py",
    "check-runtime-crossings.py",
]

FAILURES: list[str] = []

# Every call is appended to $STUB_DIR/calls as one NUL-terminated record,
# including a multi-line `--body` verbatim, so the test can assert on the
# actual PR body text rather than on a stub's opinion of it. NUL rather
# than a newline separator: `--body`'s own content is multi-line, so a
# newline-joined log cannot tell "the next line of this body" from "the
# next call" apart. `pr view` reports no open PR (empty output, exit 0),
# which sends the script down the `pr create` branch every time -- there is
# nothing here to reuse across cases in one bare git remote.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\0' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    if printf '%s' "$*" | grep -q -- "--json url"; then
        echo "https://example.com/pull/1"
        exit 0
    fi
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
    exit 0
fi
exit 0
"""


def pinned_channel() -> str:
    text = (REPO_ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("channel"):
            return line.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("rust-toolchain.toml names no channel")


def build_sandbox(root: Path, channel: str) -> None:
    (root / "rust-toolchain.toml").write_text(
        f'[toolchain]\nchannel = "{channel}"\nprofile = "minimal"\n',
        encoding="utf-8",
    )
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["dummy"]\nresolver = "2"\n', encoding="utf-8"
    )
    dummy = root / "dummy" / "src"
    dummy.mkdir(parents=True)
    (root / "dummy" / "Cargo.toml").write_text(
        '[package]\nname = "dummy"\nversion = "0.1.0"\nedition = "2021"\n',
        encoding="utf-8",
    )
    (dummy / "lib.rs").write_text("pub fn x() {}\n", encoding="utf-8")

    scripts = root / "scripts"
    scripts.mkdir()
    (scripts / "checks").mkdir()
    shutil.copy(HERE / "check.sh", scripts / "check.sh")
    (scripts / "check.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", scripts / "lib")
    shutil.copy(ISSUE_LAND, scripts / "issue-land.sh")
    (scripts / "issue-land.sh").chmod(0o755)
    for name in STUB_CHECKS:
        (scripts / "checks" / name).write_text(
            "#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8"
        )
        (scripts / "checks" / name).chmod(0o755)


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def land(
    root: Path, target: Path, stub_dir: Path, extra_args: list[str]
) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["CARGO_TARGET_DIR"] = str(target)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "-m", "feat(dummy): add a file", "--no-merge", *extra_args],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def run_case(*, refs_only: bool) -> None:
    """One landing, in its own sandbox, with `--refs-only` or without."""
    prefix = f"[refs_only={refs_only}] "
    channel = pinned_channel()
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        target = base / "target"
        root = base / "repo"
        origin = base / "origin.git"
        stub_dir = base / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        gh = stub_dir / "bin" / "gh"
        gh.write_text(GH_STUB, encoding="utf-8")
        gh.chmod(0o755)
        (stub_dir / "calls").write_text("", encoding="utf-8")

        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
        root.mkdir()
        build_sandbox(root, channel)
        git("init", "-q", "-b", "main", cwd=root)
        git("config", "user.email", "test@example.com", cwd=root)
        git("config", "user.name", "Test", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "issue-1189-refs-only-check", cwd=root)
        (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")

        extra_args = ["--refs-only"] if refs_only else []
        result = land(root, target, stub_dir, extra_args)
        records = (stub_dir / "calls").read_bytes().split(b"\0")
        calls = [record.decode("utf-8") for record in records if record]

        if result.returncode != 0:
            FAILURES.append(
                f"{prefix}the landing failed:\n--- stdout ---\n{result.stdout}\n"
                f"--- stderr ---\n{result.stderr}\n--- gh calls ---\n{calls}"
            )
            return

        create_calls = [call for call in calls if call.startswith("pr create")]
        if len(create_calls) != 1:
            FAILURES.append(
                f"{prefix}expected exactly one `pr create`, got {len(create_calls)}:\n{calls}"
            )
            return
        body = create_calls[0]

        if refs_only:
            expect(
                f"{prefix}the body refs rather than closes",
                "Closes #1189" not in body,
                f"found a bare `Closes` in a --refs-only body:\n{body}",
            )
            expect(
                f"{prefix}the body names the issue with Refs",
                "Refs: #1189" in body,
                f"expected `Refs: #1189` in the body:\n{body}",
            )
            expect(
                f"{prefix}the body says the omission was deliberate",
                "deliberately not" in body and "Closes" in body,
                f"expected the body to say the missing Closes was deliberate:\n{body}",
            )
        else:
            expect(
                f"{prefix}the default still closes the issue",
                "Closes #1189" in body,
                f"expected `Closes #1189` in the default body:\n{body}",
            )
            expect(
                f"{prefix}the default body does not ref instead",
                "Refs: #1189" not in body,
                f"found an unexpected `Refs: #1189` in the default body:\n{body}",
            )


def expect(case: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"  ok: {case}")
    else:
        FAILURES.append(f"{case}: {detail}")
        print(f"  FAILED: {case} — {detail}")


def main() -> int:
    print("issue-land --refs-only self-test")
    run_case(refs_only=True)
    run_case(refs_only=False)

    print()
    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("all cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
