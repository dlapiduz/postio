"""Self-test for #290: claiming and landing against a base that is not main.

Multi-account (#1) is worked as an initiative branch, so a worktree cut from
`feature/multi-account` has to land back onto it. Both scripts hardcoded
`main` in five places, and every one of them is correct-looking — a branch cut
from an initiative branch and landed with `--base main` puts the work straight
onto `main`, silently, which is the single thing an initiative branch exists
to prevent.

The base is therefore **recorded by the claim and read by the landing**, in
the worktree's own private git dir, rather than retyped on every command. A
flag the operator has to remember is a flag the operator eventually forgets,
and forgetting this one is a merge to `main`.

Only `gh` is stubbed. `git` is real throughout, against a bare local remote,
so what is asserted is where the branch actually sits and what base the PR was
actually opened against — not a stubbed opinion about either.

Usage: scripts/tests/test-issue-base-branch.py
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
CLAIM = HERE / "issue-claim.sh"
LAND = HERE / "issue-land.sh"

FAILURES: list[str] = []

# Answers the handful of questions the scripts ask, and logs every call so the
# test can assert on the base `gh pr create` was actually given.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/issues.json"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
    cat "$STUB_DIR/issue.json"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    if printf '%s' "$*" | grep -q -- "--json number"; then exit 1; fi
    if printf '%s' "$*" | grep -q -- "--json url"; then
        echo "https://example.com/pull/1"; exit 0
    fi
    if printf '%s' "$*" | grep -q -- "--json baseRefName"; then
        cat "$STUB_DIR/prbase.txt" 2>/dev/null || echo '{"baseRefName":"main"}'
        exit 0
    fi
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then echo "[]"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    # Actually move the base, as a real rebase-merge does -- the #312
    # verification in issue-land.sh checks that the work reached the base, so
    # a stub that says "Merged" without merging fails that very check. The
    # base is read the same way the land script reads it.
    BASE=$(cat "$(git rev-parse --git-dir)/postio-base" 2>/dev/null || echo main)
    git push -q origin "HEAD:refs/heads/$BASE" || exit 1
    echo "Merged"
    exit 0
fi
exit 0
"""

FIXTURE_CI_YML = "name: CI\non:\n  workflow_dispatch:\n"

STUB_CHECKS = [
    "check-crate-boundaries.py",
    "check-no-personal-data.py",
    "check-no-silent-tracking.py",
    "check-toolchain-pinned.py",
    "check-no-gtk-init-in-unit-tests.py",
    "check-runtime-crossings.py",
]


def git(*args: str, cwd: Path, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=check,
        capture_output=True,
        text=True,
    )


def pinned_channel() -> str:
    for line in (HERE.parent / "rust-toolchain.toml").read_text(encoding="utf-8").splitlines():
        if line.strip().startswith("channel"):
            return line.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("no channel pinned")


def build_repo(root: Path, channel: str) -> None:
    (root / "rust-toolchain.toml").write_text(
        f'[toolchain]\nchannel = "{channel}"\nprofile = "minimal"\n', encoding="utf-8"
    )
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["dummy"]\nresolver = "2"\n', encoding="utf-8"
    )
    (root / "dummy" / "src").mkdir(parents=True)
    (root / "dummy" / "Cargo.toml").write_text(
        '[package]\nname = "dummy"\nversion = "0.1.0"\nedition = "2021"\n', encoding="utf-8"
    )
    (root / "dummy" / "src" / "lib.rs").write_text("pub fn x() {}\n", encoding="utf-8")
    workflows = root / ".github" / "workflows"
    workflows.mkdir(parents=True)
    (workflows / "ci.yml").write_text(FIXTURE_CI_YML, encoding="utf-8")

    scripts = root / "scripts"
    scripts.mkdir()
    (scripts / "checks").mkdir()
    shutil.copy(HERE / "check.sh", scripts / "check.sh")
    (scripts / "check.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", scripts / "lib")
    for source in (CLAIM, LAND, HERE / "wait-for-checks.sh", HERE / "checks" / "ci-expected-workflows.py"):
        into = scripts / "checks" if source.parent.name == "checks" else scripts
        shutil.copy(source, into / source.name)
        (into / source.name).chmod(0o755)
    for name in STUB_CHECKS:
        (scripts / "checks" / name).write_text("#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8")
        (scripts / "checks" / name).chmod(0o755)


def world(base: Path, channel: str, issue: int) -> tuple[Path, Path, Path]:
    """A bare remote with `main` and `feature/initiative`, plus a checkout."""
    origin = base / "origin.git"
    root = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    gh = stub_dir / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub_dir / "calls").write_text("", encoding="utf-8")
    (stub_dir / "issues.json").write_text(
        f'[{{"number":{issue},"title":"do the thing","labels":[{{"name":"ready"}}],'
        f'"assignees":[],"milestone":null,"blockedBy":[]}}]',
        encoding="utf-8",
    )
    (stub_dir / "issue.json").write_text(
        f'{{"number":{issue},"title":"do the thing","labels":[{{"name":"ready"}}],'
        f'"assignees":[],"milestone":null,"blockedBy":[]}}',
        encoding="utf-8",
    )

    subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True, capture_output=True)
    root.mkdir()
    build_repo(root, channel)
    git("init", "-q", "-b", "main", cwd=root)
    # In the repo's own config, not via `-c`: `issue-land.sh` runs a plain
    # `git commit`, and worktrees inherit this.
    git("config", "user.email", "test@example.com", cwd=root)
    git("config", "user.name", "Test", cwd=root)
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)

    # The initiative branch, with a commit of its own so "cut from here" is
    # distinguishable from "cut from main".
    git("checkout", "-q", "-b", "feature/initiative", cwd=root)
    (root / "INITIATIVE").write_text("the initiative lives here\n", encoding="utf-8")
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "chore: start the initiative", cwd=root)
    git("push", "-q", "origin", "feature/initiative", cwd=root)
    git("checkout", "-q", "main", cwd=root)

    return origin, root, stub_dir


def env_for(root: Path, base: Path, stub_dir: Path) -> dict[str, str]:
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    environment["CARGO_TARGET_DIR"] = str(base / "target")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["POSTIO_CHECKS_GRACE"] = "1"
    environment["POSTIO_CHECKS_POLL"] = "1"
    return environment


def run(script: str, args: list[str], cwd: Path, environment: dict[str, str], timeout: int = 180):
    return subprocess.run(
        ["bash", str(cwd / "scripts" / script) if (cwd / "scripts" / script).exists() else script, *args],
        cwd=cwd,
        env=environment,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def report(name: str, result, calls: str) -> str:
    return (
        f"[{name}] exit={result.returncode}\n--- stdout ---\n{result.stdout}\n"
        f"--- stderr ---\n{result.stderr}\n--- gh calls ---\n{calls}"
    )


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0
    channel = pinned_channel()

    # --- A: a worktree claimed with --base lands back onto that base --------
    with tempfile.TemporaryDirectory(dir=HERE.parent) as directory:
        base = Path(directory)
        _origin, root, stub_dir = world(base, channel, 7)
        environment = env_for(root, base, stub_dir)

        claimed = run("issue-claim.sh", ["--base", "feature/initiative", "7"], root, environment)
        tree = base / "worktrees" / "issue-7"
        if claimed.returncode != 0 or not tree.exists():
            FAILURES.append(f"A: the claim did not produce a worktree\n{report('A', claimed, '')}")
        else:
            # Cut from the initiative branch, not from main: its marker file
            # is the difference.
            if not (tree / "INITIATIVE").exists():
                FAILURES.append(
                    "A: the worktree was cut from main, so the initiative's own "
                    "commits are missing from it and landing would replay the "
                    f"branch onto the wrong base\n{report('A', claimed, '')}"
                )

            (tree / "dummy" / "src" / "extra.rs").write_text("// work\n", encoding="utf-8")
            landed = run(
                "issue-land.sh", ["-m", "feat(dummy): add a file"], tree, environment, timeout=240
            )
            calls = (stub_dir / "calls").read_text(encoding="utf-8")
            if landed.returncode != 0:
                FAILURES.append(f"A: landing failed\n{report('A', landed, calls)}")
            if "--base feature/initiative" not in calls:
                FAILURES.append(
                    "A: the PR was opened against the wrong base -- initiative work "
                    f"would merge straight into main\n{report('A', landed, calls)}"
                )
            if "--base main" in calls:
                FAILURES.append(f"A: the PR named main as its base\n{report('A', landed, calls)}")

    # --- B: no base given behaves exactly as it does today ------------------
    with tempfile.TemporaryDirectory(dir=HERE.parent) as directory:
        base = Path(directory)
        _origin, root, stub_dir = world(base, channel, 8)
        environment = env_for(root, base, stub_dir)

        claimed = run("issue-claim.sh", ["8"], root, environment)
        tree = base / "worktrees" / "issue-8"
        if claimed.returncode != 0 or not tree.exists():
            FAILURES.append(f"B: the claim failed\n{report('B', claimed, '')}")
        else:
            if (tree / "INITIATIVE").exists():
                FAILURES.append("B: a claim with no --base must still come from main")
            (tree / "dummy" / "src" / "extra.rs").write_text("// work\n", encoding="utf-8")
            landed = run(
                "issue-land.sh", ["-m", "feat(dummy): add a file"], tree, environment, timeout=240
            )
            calls = (stub_dir / "calls").read_text(encoding="utf-8")
            if landed.returncode != 0:
                FAILURES.append(f"B: landing failed\n{report('B', landed, calls)}")
            if "--base main" not in calls:
                FAILURES.append(
                    f"B: the default stopped being main\n{report('B', landed, calls)}"
                )

    # --- C: a base that does not exist is refused ---------------------------
    with tempfile.TemporaryDirectory(dir=HERE.parent) as directory:
        base = Path(directory)
        _origin, root, stub_dir = world(base, channel, 9)
        environment = env_for(root, base, stub_dir)

        claimed = run("issue-claim.sh", ["--base", "feature/typo", "9"], root, environment)
        if claimed.returncode == 0:
            FAILURES.append(
                "C: a base that does not exist on the remote was accepted, so the "
                f"work would be cut from nowhere\n{report('C', claimed, '')}"
            )
        if "feature/typo" not in (claimed.stdout + claimed.stderr):
            FAILURES.append(f"C: the refusal did not name the base\n{report('C', claimed, '')}")

        # The refusal has to come *before* anything is claimed. Letting it fall
        # through to `git fetch` also fails, and looks the same in the exit
        # code -- but by then the claim directory exists, so a typo locks the
        # issue behind a claim no session holds and no release will sweep.
        claims = base / "claims"
        leaked = sorted(p.name for p in claims.iterdir()) if claims.exists() else []
        if leaked:
            FAILURES.append(
                "C: a refused base left a claim behind, which locks the issue for "
                f"every other session: {leaked}\n{report('C', claimed, '')}"
            )
        if (base / "worktrees" / "issue-9").exists():
            FAILURES.append(f"C: a refused base still made a worktree\n{report('C', claimed, '')}")

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("issue base-branch check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
