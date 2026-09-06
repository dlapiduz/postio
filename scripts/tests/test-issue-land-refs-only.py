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

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"
# Sandboxes go under `target/`, which git ignores: inside the worktree because
# the shared-tree guard only lifts its refusals for worktree paths, and not in
# its root because a killed run leaves the sandbox behind and `git add -A` in a
# worktree will commit it. `scripts/checks/check-test-sandboxes.py` says what
# that cost (#1225).
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

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
    root: Path,
    target: Path,
    stub_dir: Path,
    extra_args: list[str],
    message: str = "feat(dummy): add a file",
) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["CARGO_TARGET_DIR"] = str(target)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    return patience.run(
        ["bash", "scripts/issue-land.sh", "-m", message, "--no-merge", *extra_args],
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
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
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


def run_negation_case() -> None:
    """A `--refs-only` landing whose commit body would close the issue anyway.

    GitHub scans commit messages for `close|closes|closed|fix|...|resolved
    #<n>` and acts on the keyword without reading the negation in front of it.
    So the sentence a deliberately-partial commit most wants to write —
    "this does not close #1216" — is the one that closes it, and it did:
    #1216 is a p1 investigation with an unmet acceptance line, closed on merge
    by the commit that said it was not finishing it, under a PR body that had
    been careful to say `Refs` (#1234).

    `--refs-only` is the caller stating the intent, so the two can be checked
    against each other.
    """
    prefix = "[negation] "
    channel = pinned_channel()
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
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

        message = (
            "feat(dummy): add a file\n\n"
            "This does not close #1189: the acceptance is not met.\n"
        )
        result = land(root, target, stub_dir, ["--refs-only"], message)
        records = (stub_dir / "calls").read_bytes().split(b"\0")
        calls = [record.decode("utf-8") for record in records if record]
        output = result.stdout + result.stderr

        expect(
            f"{prefix}the landing is refused",
            result.returncode != 0,
            f"a --refs-only landing whose commit body closes #1189 went "
            f"through:\n{output}",
        )
        expect(
            f"{prefix}nothing was pushed",
            not [call for call in calls if call.startswith("pr create")],
            f"a PR was opened for it anyway:\n{calls}",
        )
        expect(
            f"{prefix}it says what to write instead",
            "#1189" in output and "Refs:" in output,
            f"the refusal did not name the issue and a wording that works:"
            f"\n{output}",
        )

        # Take the advice the refusal gave, which is the other half of it
        # being useful: the reworded body must land.
        git("commit", "-q", "--amend", "-m",
            "feat(dummy): add a file\n\nThis does not finish #1189: the "
            "acceptance is not met.\n", cwd=root)

        # A commit that closes some *other* issue is ordinary: a rider closed
        # alongside the anchor, a fix that finishes something else on the way
        # past. Only the issue being landed is the contradiction.
        git("commit", "-q", "--allow-empty", "-m",
            "chore(dummy): tidy up\n\nCloses #4242\n", cwd=root)
        (root / "dummy" / "src" / "another.rs").write_text("// nothing\n", encoding="utf-8")
        result = land(root, target, stub_dir, ["--refs-only"], "feat(dummy): more")
        expect(
            f"{prefix}the reworded body lands, and another issue's Closes is "
            f"left alone",
            result.returncode == 0,
            f"the wording the refusal recommended, or a commit closing #4242, "
            f"blocked a landing for #1189:\n{result.stdout}\n{result.stderr}",
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
    run_negation_case()

    print()
    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("all cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
