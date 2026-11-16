#!/usr/bin/env python3
"""Landing a branch stacked on another PR's branch.

A branch can depend on an open PR's branch (`postio-base` names it). PRs merge
squashed, so once the base lands, its commits are on `main` under other
hashes and a plain rebase replays them. `issue-land.sh` must instead rebase
`--onto origin/main` from the base's old tip, record `main`, and retarget an
existing PR; while the base is still open it must land against the base.

`git` is real against a bare local remote; only `gh` is stubbed.

  A  base squash-merged, PR open   -> own commit only on main, base is main,
                                      PR retargeted with `gh pr edit --base main`
  B  base squash-merged, no PR yet -> the new PR is opened against main
  C  base still open               -> PR against the base, says it will retarget
  D  base is main                  -> `gh pr list` is never asked

Usage: scripts/tests/test-issue-land-stacked.py
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

import patience  # noqa: E402
import prereq  # noqa: E402

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
LAND = HERE / "issue-land.sh"
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

FAILURES: list[str] = []

STUB_CHECKS = [
    "check-crate-boundaries.py",
    "check-no-personal-data.py",
    "check-no-silent-tracking.py",
    "check-toolchain-pinned.py",
    "check-no-gtk-init-in-unit-tests.py",
    "check-runtime-crossings.py",
]

# `merged.txt` holds the old base tip when the base's PR has merged;
# `open.txt` exists when this branch already has an open PR.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
    if printf '%s' "$*" | grep -q -- "--state open"; then
        [ -e "$STUB_DIR/open.txt" ] && echo 1 || echo 0
        exit 0
    fi
    cat "$STUB_DIR/merged.txt" 2>/dev/null
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    if printf '%s' "$*" | grep -q -- "--json number"; then exit 1; fi
    if printf '%s' "$*" | grep -q -- "--json url"; then
        echo "https://example.com/pull/1"; exit 0
    fi
    if printf '%s' "$*" | grep -q -- "--json state"; then
        [ -e "$STUB_DIR/open.txt" ] && echo OPEN
        exit 0
    fi
    if printf '%s' "$*" | grep -q -- "--json baseRefName"; then
        grep -q 'pr edit --base main' "$STUB_DIR/calls" && echo main || cat "$STUB_DIR/prbase.txt"
        exit 0
    fi
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then echo "[]"; exit 0; fi
exit 0
"""


def git(*args: str, cwd: Path) -> str:
    done = subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd, check=False, capture_output=True, text=True,
    )
    if done.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {done.stderr}")
    return done.stdout.strip()


def pinned_channel() -> str:
    for line in (REPO_ROOT / "rust-toolchain.toml").read_text(encoding="utf-8").splitlines():
        if line.strip().startswith("channel"):
            return line.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("no channel pinned")


def build_repo(root: Path, channel: str) -> None:
    (root / "rust-toolchain.toml").write_text(
        f'[toolchain]\nchannel = "{channel}"\nprofile = "minimal"\n', encoding="utf-8")
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["dummy"]\nresolver = "2"\n', encoding="utf-8")
    (root / "dummy" / "src").mkdir(parents=True)
    (root / "dummy" / "Cargo.toml").write_text(
        '[package]\nname = "dummy"\nversion = "0.1.0"\nedition = "2021"\n', encoding="utf-8")
    (root / "dummy" / "src" / "lib.rs").write_text("pub fn x() {}\n", encoding="utf-8")
    workflows = root / ".github" / "workflows"
    workflows.mkdir(parents=True)
    (workflows / "ci.yml").write_text("name: CI\non:\n  workflow_dispatch:\n", encoding="utf-8")
    scripts = root / "scripts"
    (scripts / "checks").mkdir(parents=True)
    shutil.copy(HERE / "check.sh", scripts / "check.sh")
    (scripts / "check.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", scripts / "lib")
    for source in (LAND, HERE / "wait-for-checks.sh", HERE / "checks" / "ci-expected-workflows.py"):
        into = scripts / "checks" if source.parent.name == "checks" else scripts
        shutil.copy(source, into / source.name)
        (into / source.name).chmod(0o755)
    for name in STUB_CHECKS:
        (scripts / "checks" / name).write_text("#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8")
        (scripts / "checks" / name).chmod(0o755)


def run_case(base: Path, channel: str, *, recorded: str, base_merged: bool, pr_open: bool):
    """origin/main + `chore/base` (one commit) + `chore/stacked` cut from it."""
    origin, root, stub = base / "origin.git", base / "repo", base / "stub"
    (stub / "bin").mkdir(parents=True)
    gh = stub / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub / "calls").write_text("", encoding="utf-8")
    (stub / "prbase.txt").write_text(recorded + "\n", encoding="utf-8")
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    root.mkdir()
    build_repo(root, channel)
    git("init", "-q", "-b", "main", cwd=root)
    git("config", "user.email", "test@example.com", cwd=root)
    git("config", "user.name", "Test", cwd=root)
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)

    git("checkout", "-q", "-b", "chore/base", cwd=root)
    (root / "BASE").write_text("the base's work\n", encoding="utf-8")
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "chore: the base's work", cwd=root)
    git("push", "-q", "origin", "chore/base", cwd=root)
    old_tip = git("rev-parse", "HEAD", cwd=root)

    git("checkout", "-q", "-b", "chore/stacked", cwd=root)
    (root / "dummy" / "src" / "extra.rs").write_text("// stacked\n", encoding="utf-8")
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "feat(dummy): the stacked work", cwd=root)
    if recorded != "main":
        gitdir = Path(git("rev-parse", "--absolute-git-dir", cwd=root))
        (gitdir / "postio-base").write_text(recorded + "\n", encoding="utf-8")

    if base_merged:
        # What GitHub does: one squash commit on main with different hashes,
        # and the merged branch deleted.
        other = base / "other"
        subprocess.run(["git", "clone", "-q", str(origin), str(other)], check=True)
        (other / "BASE").write_text("the base's work\n", encoding="utf-8")
        git("add", "-A", cwd=other)
        git("commit", "-q", "-m", "chore: the base's work (#1)", cwd=other)
        git("push", "-q", "origin", "main", cwd=other)
        git("push", "-q", "origin", "--delete", "chore/base", cwd=root)
        (stub / "merged.txt").write_text(old_tip + "\n", encoding="utf-8")
    if pr_open:
        (stub / "open.txt").write_text("", encoding="utf-8")

    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["PATH"] = f"{stub / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub)
    environment["CARGO_TARGET_DIR"] = str(base / "target")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["POSTIO_CHECKS_GRACE"] = "1"
    environment["POSTIO_CHECKS_POLL"] = "1"
    result = patience.run(
        ["bash", "scripts/issue-land.sh", "--no-merge", "--title", "feat(dummy): stacked"],
        cwd=root, env=environment, capture_output=True, text=True, timeout=300,
    )
    calls = (stub / "calls").read_text(encoding="utf-8")
    return result, calls, root


def report(case: str, result, calls: str) -> str:
    return (f"[{case}] exit={result.returncode}\n--- stdout ---\n{result.stdout}\n"
            f"--- stderr ---\n{result.stderr}\n--- gh calls ---\n{calls}")


def main() -> int:
    prereq.require("cargo", present=prereq.have("cargo"))
    channel = pinned_channel()

    def case(**kw):
        with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
            result, calls, root = run_case(Path(directory), channel, **kw)
            gitdir = Path(git("rev-parse", "--absolute-git-dir", cwd=root))
            recorded = (gitdir / "postio-base").read_text().strip() if (gitdir / "postio-base").exists() else "main"
            ahead = git("log", "--format=%s", "origin/main..HEAD", cwd=root).splitlines()
            return result, calls, recorded, ahead

    # A
    r, calls, recorded, ahead = case(recorded="chore/base", base_merged=True, pr_open=True)
    if r.returncode != 0:
        FAILURES.append(f"A: landing failed\n{report('A', r, calls)}")
    if ahead != ["feat(dummy): the stacked work"]:
        FAILURES.append(f"A: branch is not exactly its own commit on main: {ahead}\n{report('A', r, calls)}")
    if recorded != "main":
        FAILURES.append(f"A: postio-base is {recorded!r}, not main\n{report('A', r, calls)}")
    if "pr edit --base main" not in calls:
        FAILURES.append(f"A: the open PR was not retargeted\n{report('A', r, calls)}")
    if "pr create" in calls:
        FAILURES.append(f"A: opened a second PR\n{report('A', r, calls)}")
    if "squash-merged" not in r.stdout:
        FAILURES.append(f"A: said nothing about the move\n{report('A', r, calls)}")

    # B
    r, calls, recorded, ahead = case(recorded="chore/base", base_merged=True, pr_open=False)
    if r.returncode != 0 or "pr create --base main" not in calls:
        FAILURES.append(f"B: the new PR was not opened against main\n{report('B', r, calls)}")
    if "pr edit --base main" in calls:
        FAILURES.append(f"B: retargeted a PR that does not exist\n{report('B', r, calls)}")
    if ahead != ["feat(dummy): the stacked work"]:
        FAILURES.append(f"B: base commits were replayed: {ahead}\n{report('B', r, calls)}")

    # C
    r, calls, recorded, ahead = case(recorded="chore/base", base_merged=False, pr_open=False)
    if r.returncode != 0 or "pr create --base chore/base" not in calls:
        FAILURES.append(f"C: the PR was not opened against the open base\n{report('C', r, calls)}")
    if recorded != "chore/base" or "retargeted" not in r.stdout:
        FAILURES.append(f"C: base changed or the retarget was not announced\n{report('C', r, calls)}")

    # D
    r, calls, recorded, ahead = case(recorded="main", base_merged=False, pr_open=False)
    if r.returncode != 0 or "pr list" in calls or "pr create --base main" not in calls:
        FAILURES.append(f"D: the default path changed\n{report('D', r, calls)}")

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("issue-land stacked check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
