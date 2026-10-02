#!/usr/bin/env python3
"""Self-test for specs/008-storyboards T062: the landing names an unreviewed
interaction change.

A branch that changes a GTK app's crates should arrive with a storyboard
review (`/ux-review`) for the tree it lands. `issue-land.sh` never refuses
on it (FR-023) -- the review is advisory and the catalogue is young -- but
it must not let a missing one pass silently either:

  * no `summary.md`, or one whose `storyboards-key:` is not the tree's
    current key: the PR body carries a `> [!WARNING]` naming `/ux-review`,
    the PR is labelled `interactions-unreviewed`, and the landing proceeds;
  * a current `summary.md`: its text goes into the PR body, no label;
  * a push to a PR that is already open, with a current summary: the
    summary is posted as a comment, because the body is not rewritten.

`gh` is stubbed and logs every call verbatim (NUL-separated, so a
multi-line body survives). `scripts/storyboards.sh` is stubbed to print a
fixed key, because the sandbox cannot build the storyboard tool, and how the
key is computed is `postio-storyboard`'s own test.

Usage: scripts/tests/test-issue-land-storyboards.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

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


GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\0' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    if printf '%s' "$*" | grep -q -- "--json url"; then
        echo "https://example.com/pull/1"; exit 0
    fi
    if printf '%s' "$*" | grep -q -- "--json state" && [ -n "${EXISTING_PR:-}" ]; then
        echo "OPEN"; exit 0
    fi
    if printf '%s' "$*" | grep -q -- "--json baseRefName"; then
        echo "main"; exit 0
    fi
    exit 0
fi
exit 0
"""

KEY_STUB = """#!/usr/bin/env bash
[ "$1" = key ] && { echo "k-current"; exit 0; }
exit 0
"""


BRANCH = "issue-4242-storyboard-warning"


def sandbox(base: Path, existing_pr: bool = False):
    channel = pinned_channel()
    target = base / "target"
    root = base / "repo"
    origin = base / "origin.git"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    gh_path = stub_dir / "bin" / "gh"
    gh_path.write_text(GH_STUB, encoding="utf-8")
    gh_path.chmod(0o755)
    (stub_dir / "calls").write_text("", encoding="utf-8")
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    root.mkdir()
    build_sandbox(root, channel)
    # The stand-in crate is a GTK app's crate, so the landing is about one.
    (root / "crates").mkdir()
    shutil.move(str(root / "dummy"), str(root / "crates" / "postio-gtk"))
    manifest = (root / "crates" / "postio-gtk" / "Cargo.toml").read_text()
    (root / "crates" / "postio-gtk" / "Cargo.toml").write_text(
        manifest.replace('name = "dummy"', 'name = "postio-gtk"')
    )
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/postio-gtk"]\nresolver = "2"\n', encoding="utf-8"
    )
    key = root / "scripts" / "storyboards.sh"
    key.write_text(KEY_STUB, encoding="utf-8")
    key.chmod(0o755)
    (root / ".gitignore").write_text("Design/review/\n", encoding="utf-8")
    git("init", "-q", "-b", "main", cwd=root)
    git("config", "user.email", "test@example.com", cwd=root)
    git("config", "user.name", "Test", cwd=root)
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)
    git("checkout", "-q", "-b", BRANCH, cwd=root)
    (root / "crates" / "postio-gtk" / "src" / "extra.rs").write_text("// nothing\n")
    return root, target, stub_dir


def land(root: Path, target: Path, stub_dir: Path, existing_pr: bool = False):
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["CARGO_TARGET_DIR"] = str(target)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    if existing_pr:
        environment["EXISTING_PR"] = "1"
    result = patience.run(
        ["bash", "scripts/issue-land.sh", "-m", "feat(gtk): add a file", "--no-merge"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )
    records = (stub_dir / "calls").read_bytes().split(b"\0")
    return result, [r.decode("utf-8") for r in records if r]


def summary(root: Path, key: str) -> None:
    review = root / "Design" / "review" / BRANCH
    review.mkdir(parents=True, exist_ok=True)
    (review / "summary.md").write_text(
        f"storyboards-key: {key}\n\nReview complete. 3 storyboards changed, all pass.\n"
    )


def case(name: str, key: str | None, existing_pr: bool, warned: bool, carried: str) -> None:
    prefix = f"[{name}] "
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        root, target, stub_dir = sandbox(Path(directory))
        if key is not None:
            summary(root, key)
        result, calls = land(root, target, stub_dir, existing_pr)
        if result.returncode != 0:
            FAILURES.append(f"{prefix}the landing failed:\n{result.stdout}\n{result.stderr}")
            print(f"  FAILED: {prefix}landing failed")
            return
        create = [c for c in calls if c.startswith("pr create")]
        labelled = [c for c in calls if c.startswith("pr edit") and "interactions-unreviewed" in c]
        comments = [c for c in calls if c.startswith("pr comment")]
        body = create[0] if create else ""
        if warned:
            expect(f"{prefix}the body warns and names /ux-review",
                   "[!WARNING]" in body and "/ux-review" in body, body)
            expect(f"{prefix}the PR is labelled", len(labelled) == 1, str(calls))
        else:
            expect(f"{prefix}no warning", "/ux-review" not in body, body)
            expect(f"{prefix}no label", not labelled, str(calls))
        if carried == "body":
            expect(f"{prefix}the summary is in the body", "Review complete" in body, body)
        if carried == "comment":
            expect(f"{prefix}the summary is commented", any("Review complete" in c for c in comments), str(calls))


def expect(case: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"  ok: {case}")
    else:
        FAILURES.append(f"{case}: {detail}")
        print(f"  FAILED: {case} — {detail}")


def main() -> int:
    print("issue-land storyboard-review self-test")
    case("no summary", None, False, warned=True, carried="")
    case("stale summary", "k-old", False, warned=True, carried="")
    case("current summary", "k-current", False, warned=False, carried="body")
    case("existing PR", "k-current", True, warned=False, carried="comment")
    print()
    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("all cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
