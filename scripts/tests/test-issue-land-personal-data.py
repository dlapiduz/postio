#!/usr/bin/env python3
"""Self-test for issue #270: a new file is invisible to `issue-land.sh`'s
personal-data gate on the very landing that adds it.

`check-no-personal-data.py` reads `git ls-files` -- tracked files only.
`issue-land.sh` used to run it as part of "repository invariants" *before*
`git add -A`, so a brand-new file this session is adding was untracked at
scan time and the check never saw it: not on this landing, and -- with
per-PR CI paused -- not afterward either, unless some later, unrelated
branch happened to run the check while the file was already on `main`. This
is how #269's real (if harmless) false positive got through unscanned in the
first place, and would have done the same for a real leak.

The fix moves `git add -A` before the invariants, so a new file is staged --
and therefore visible to `git ls-files` -- by the time they run.

Only `check-no-personal-data.py` is real here; the other five invariant
checks are stubbed, exactly as in `test-issue-land-commit-guard.py`, because
this test is about staging order, not about them. `--wip` is used throughout
so the script never touches `gh`.

Usage: scripts/tests/test-issue-land-personal-data.py
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

sys.path.insert(0, str(HERE / "lib"))

import fixture_toolchain  # noqa: E402  -- enabled by the sys.path line above
ISSUE_LAND = HERE / "issue-land.sh"
PERSONAL_DATA_CHECK = HERE / "checks" / "check-no-personal-data.py"
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

STUB_CHECKS = [
    "check-crate-boundaries.py",
    "check-no-silent-tracking.py",
    "check-toolchain-pinned.py",
    "check-no-gtk-init-in-unit-tests.py",
    "check-runtime-crossings.py",
]

FAILURES: list[str] = []


def build_sandbox(root: Path) -> None:
    fixture_toolchain.write(root)
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
    shutil.copy(ISSUE_LAND, scripts / ISSUE_LAND.name)
    (scripts / ISSUE_LAND.name).chmod(0o755)
    shutil.copy(PERSONAL_DATA_CHECK, scripts / "checks" / PERSONAL_DATA_CHECK.name)
    (scripts / "checks" / PERSONAL_DATA_CHECK.name).chmod(0o755)
    for name in STUB_CHECKS:
        (scripts / "checks" / name).write_text(
            "#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8"
        )
        (scripts / "checks" / name).chmod(0o755)


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def land(root: Path, target: Path, *args: str) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["CARGO_TARGET_DIR"] = str(target)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["POSTIO_DENY_NAMES"] = "Nobody In Particular"
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "--wip", *args],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
    )


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0


    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        base = Path(directory)
        target = base / "target"
        root = base / "repo"
        origin = base / "origin.git"

        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
        root.mkdir()
        build_sandbox(root)
        git("init", "-q", "-b", "main", cwd=root)
        git("config", "user.email", "test@example.com", cwd=root)
        git("config", "user.name", "Test", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "issue-1-x", cwd=root)

        (root / "dummy" / "src" / "leak.rs").write_text(
            "// contact: quinn.harlow@northwind-traders.co.uk\n",
            encoding="utf-8",
        )

        result = land(root, target, "-m", "feat(dummy): add a file")

        if result.returncode == 0:
            FAILURES.append(
                "issue-land.sh landed a new file with a real email address "
                "on it -- the personal-data gate never saw it because it ran "
                f"before the file was staged:\n--- stdout ---\n{result.stdout}\n"
                f"--- stderr ---\n{result.stderr}"
            )
        elif "personal-data check FAILED" not in (result.stdout + result.stderr):
            FAILURES.append(
                "issue-land.sh failed, but not for the reason expected -- the "
                "personal-data check itself should be the one that says so:\n"
                f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
            )

        log = subprocess.run(
            ["git", "log", "-1", "--format=%s"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        if log != "init":
            FAILURES.append(
                "a landing the personal-data check should have refused still "
                f"produced a commit: HEAD subject is {log!r}"
            )

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("issue-land personal-data staging-order check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
