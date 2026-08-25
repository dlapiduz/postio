#!/usr/bin/env python3
"""Self-test for issue #160: issue-land.sh rebases the tree it lives in.

While landing #50, `issue-land.sh` merged a 1016-line, three-crate change
without waiting for CI. The fix that would have stopped it -- the machinery
c377dfc put in `scripts/` -- arrived in that run's own rebase, and the run
kept executing the copy bash had already opened. A script that rebases the
tree containing itself is invisible to the first run that pulls its own fix
in, and that run is exactly the one with the least reason to expect the old
behaviour.

The fix is a handover: when the rebase brings in anything under `scripts/`,
the run `exec`s the version it just pulled in, from the top, so the gates and
the merge decision are both the new machinery's and both ran against the tree
CI will actually see. Bounded, because a handover that keeps repeating is a
loop, and a loop that merges is worse than one that stops.

Four cases, each a real ordering rather than an opinion about one: `git` is
real throughout against a bare local remote, and only `gh` is stubbed.

  A  the rebase rewrites issue-land.sh itself   -> the new copy governs
  B  the rebase changes a script it *calls*     -> the gates re-run and the
                                                   merge is refused
  C  the rebase touches nothing under scripts/  -> no handover, normal merge
  D  the handover has already happened twice    -> refuse to merge, say why

Case B is the one that proves the third acceptance criterion: the gates and
the merge decision have to agree about which tree they are talking about.

Usage: scripts/test-issue-land-rebase-handover.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"
WAIT_FOR_CHECKS = HERE / "wait-for-checks.sh"
CI_EXPECTED_WORKFLOWS = HERE / "ci-expected-workflows.py"

STUB_CHECKS = [
    "check-crate-boundaries.py",
    "check-no-personal-data.py",
    "check-no-silent-tracking.py",
    "check-toolchain-pinned.py",
    "check-no-gtk-init-in-unit-tests.py",
    "check-runtime-crossings.py",
]

# Printed by the copy of issue-land.sh that main gained, and by no other. Its
# absence from a run's output means the pre-rebase copy decided the merge.
MARKER = "NEW_MACHINERY_RAN"

# Printed by the version of a *called* check that main gained. The stub the
# sandbox starts with passes, so this text can only come from the post-rebase
# tree -- and only if the gates ran again after the rebase.
CALLED_MARKER = "NEW_BOUNDARY_RULE_VIOLATED"

# The handover's own announcement, asserted present in A and absent in C.
HANDOVER = "handing over to the landing machinery"

REEXEC_DEPTH_ENV = "POSTIO_LAND_REEXEC_DEPTH"

FAILURES: list[str] = []

GH_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    if printf '%s' "$*" | grep -q -- "--json number"; then
        exit 1
    fi
    if printf '%s' "$*" | grep -q -- "--json url"; then
        echo "https://example.com/pull/1"
        exit 0
    fi
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then
    echo "[]"
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    echo "Merged"
    exit 0
fi
exit 0
"""

# No `pull_request` trigger, so ci-expected-workflows.py predicts nothing and
# wait-for-checks.sh takes its short grace path. A fixture of our own, so this
# does not start testing the other branch of that logic the day this repo's
# real triggers change for operational reasons.
FIXTURE_CI_YML = "name: CI\non:\n  workflow_dispatch:\n"


def pinned_channel() -> str:
    text = (REPO_ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("channel"):
            return line.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("rust-toolchain.toml names no channel")


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


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

    workflows = root / ".github" / "workflows"
    workflows.mkdir(parents=True)
    (workflows / "ci.yml").write_text(FIXTURE_CI_YML, encoding="utf-8")

    scripts = root / "scripts"
    scripts.mkdir()
    for source in (ISSUE_LAND, WAIT_FOR_CHECKS, CI_EXPECTED_WORKFLOWS):
        shutil.copy(source, scripts / source.name)
        (scripts / source.name).chmod(0o755)
    for name in STUB_CHECKS:
        (scripts / name).write_text(
            "#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8"
        )
        (scripts / name).chmod(0o755)


def marked_issue_land() -> str:
    """issue-land.sh as main would have it: same logic, plus a marker.

    The marker goes in the step the real incident got wrong -- the one that
    decides whether to wait for a check -- and a long comment goes in above
    it, so the bytes of everything after the rebase point move. That second
    part matters: bash reads a script by byte offset as it goes, so a run that
    keeps executing a file rewritten underneath it is not simply running the
    old version.
    """
    text = ISSUE_LAND.read_text(encoding="utf-8")
    anchor = 'echo "--- waiting for checks ---"'
    if anchor not in text:
        raise RuntimeError(f"issue-land.sh no longer contains {anchor!r}")
    padding = "\n".join(f"# {'shift the byte offsets along':<60}" for _ in range(40))
    text = text.replace(
        anchor, f"{padding}\n{anchor}\necho {MARKER}", 1
    )
    return text


def failing_boundary_check() -> str:
    return (
        "#!/usr/bin/env python3\n"
        f'print("{CALLED_MARKER}")\n'
        "raise SystemExit(1)\n"
    )


def advance_main(origin: Path, work: Path, files: dict[str, str], message: str) -> None:
    """Land a commit on origin/main from a clone of its own, as another session would."""
    subprocess.run(
        ["git", "clone", "-q", str(origin), str(work)], check=True, capture_output=True
    )
    for name, body in files.items():
        path = work / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)
    git("add", "-A", cwd=work)
    git("commit", "-q", "-m", message, cwd=work)
    git("push", "-q", "origin", "main", cwd=work)


def land(
    root: Path, target: Path, stub_dir: Path, extra_env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment.pop(REEXEC_DEPTH_ENV, None)
    environment["CARGO_TARGET_DIR"] = str(target)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_CHECKS_GRACE"] = "1"
    environment["POSTIO_CHECKS_POLL"] = "1"
    environment.update(extra_env or {})
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "-m", "feat(dummy): add a file"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=300,
    )


def run_case(
    base: Path,
    channel: str,
    name: str,
    incoming: dict[str, str],
    extra_env: dict[str, str] | None = None,
) -> tuple[subprocess.CompletedProcess[str], str]:
    """Set a whole world up, move main under the branch, and land."""
    root = base / name / "repo"
    origin = base / name / "origin.git"
    stub_dir = base / name / "stub"
    target = base / "target"
    (stub_dir / "bin").mkdir(parents=True)
    gh = stub_dir / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub_dir / "calls").write_text("", encoding="utf-8")

    subprocess.run(
        ["git", "init", "-q", "--bare", str(origin)], check=True, capture_output=True
    )
    root.mkdir(parents=True)
    build_sandbox(root, channel)
    git("init", "-q", "-b", "main", cwd=root)
    git("config", "user.email", "test@example.com", cwd=root)
    git("config", "user.name", "Test", cwd=root)
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)
    git("checkout", "-q", "-b", "issue-1-x", cwd=root)

    # main moves while the branch is being worked, exactly as it does here.
    advance_main(origin, base / name / "bump", incoming, "chore: main moves")

    (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")
    result = land(root, target, stub_dir, extra_env)
    return result, (stub_dir / "calls").read_text(encoding="utf-8")


def report(case: str, result: subprocess.CompletedProcess[str], calls: str) -> str:
    return (
        f"[{case}] exit={result.returncode}\n--- stdout ---\n{result.stdout}\n"
        f"--- stderr ---\n{result.stderr}\n--- gh calls ---\n{calls}"
    )


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0

    channel = pinned_channel()

    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)

        # A: the rebase rewrites issue-land.sh itself.
        result, calls = run_case(
            base, channel, "a", {"scripts/issue-land.sh": marked_issue_land()}
        )
        if MARKER not in result.stdout:
            FAILURES.append(
                "A: the run kept executing its pre-rebase self -- the copy the "
                "rebase brought in never got to decide the merge. This is #160 "
                f"verbatim.\n{report('A', result, calls)}"
            )
        if HANDOVER not in result.stdout:
            FAILURES.append(
                "A: nothing announced the handover, so a session reading the log "
                f"cannot tell which copy merged its work.\n{report('A', result, calls)}"
            )
        if result.returncode != 0 or "pr merge" not in calls:
            FAILURES.append(
                f"A: the handover was supposed to land the branch, not stop it.\n"
                f"{report('A', result, calls)}"
            )

        # B: the rebase changes a script issue-land.sh *calls*.
        result, calls = run_case(
            base,
            channel,
            "b",
            {"scripts/check-crate-boundaries.py": failing_boundary_check()},
        )
        if "pr merge" in calls:
            FAILURES.append(
                "B: main tightened an invariant check and this merged anyway -- "
                "the gates ran the pre-rebase copy and the merge decision ran "
                f"against a tree nothing had checked.\n{report('B', result, calls)}"
            )
        if result.returncode == 0:
            FAILURES.append(
                f"B: a failing invariant check must fail the run.\n"
                f"{report('B', result, calls)}"
            )
        if CALLED_MARKER not in (result.stdout + result.stderr):
            FAILURES.append(
                "B: the post-rebase invariant check never ran, so the gates and "
                f"the merge decision disagree about the tree.\n{report('B', result, calls)}"
            )

        # C: an ordinary rebase, nothing under scripts/.
        result, calls = run_case(
            base, channel, "c", {"docs/notes.md": "main moved, harmlessly.\n"}
        )
        if HANDOVER in result.stdout:
            FAILURES.append(
                "C: a rebase that changed no machinery still handed over, which "
                f"doubles every landing for nothing.\n{report('C', result, calls)}"
            )
        if result.returncode != 0 or "pr merge" not in calls:
            FAILURES.append(
                f"C: an ordinary rebase must still land.\n{report('C', result, calls)}"
            )

        # D: the handover has already happened as often as it may.
        result, calls = run_case(
            base,
            channel,
            "d",
            {"scripts/issue-land.sh": marked_issue_land()},
            extra_env={REEXEC_DEPTH_ENV: "2"},
        )
        if "pr merge" in calls:
            FAILURES.append(
                "D: the handover kept repeating and merged anyway. A run that "
                "cannot say which copy of itself it is must not be the one that "
                f"puts a commit on main.\n{report('D', result, calls)}"
            )
        if result.returncode == 0:
            FAILURES.append(
                f"D: an unbounded handover must stop the run.\n{report('D', result, calls)}"
            )
        if "#160" not in (result.stdout + result.stderr):
            FAILURES.append(
                "D: it refused without saying why, which leaves the session with "
                f"nothing to act on.\n{report('D', result, calls)}"
            )

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print(f"issue-land rebase-handover check passed ({channel}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
