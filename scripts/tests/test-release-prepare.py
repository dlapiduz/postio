#!/usr/bin/env python3
"""Self-test for scripts/release-prepare.sh.

The version used to move inside the release workflow, on a copy of the tree
nobody committed, so `main` said 0.3.0 while v0.4.2 was out (#1714). Now it
moves in a pull request, and this script is what makes that pull request:
a branch off `origin/main`, the notes, the bump, one commit. Merging it is
the release, so the cases that matter are the refusals -- a version that is
not newer than the newest tag, a dirty tree, and the shared checkout, where
switching branches would pull the floor out from under other sessions.

A real repository with a bare `origin`, the real release-bump.py copied in,
and `cargo` stubbed (the script only asks it to refresh Cargo.lock).

Usage: scripts/tests/test-release-prepare.py
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

SCRIPTS = Path(__file__).resolve().parent.parent
REPO_ROOT = SCRIPTS.parent
SCRIPT = SCRIPTS / "release-prepare.sh"
# Under target/, which git ignores; see scripts/checks/check-test-sandboxes.py.
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

FAILURES: list[str] = []

GIT_ENV = {"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null"}

METAINFO = """\
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <releases>
    <release version="0.4.2" date="2026-09-20" type="development">
      <description><p>x</p></description>
    </release>
  </releases>
</component>
"""
PLIST = """\
<plist><dict>
\t<key>CFBundleShortVersionString</key>
\t<string>0.4.2</string>
\t<key>CFBundleVersion</key>
\t<string>0.4.2</string>
</dict></plist>
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True,
        env={**os.environ, **GIT_ENV},
    ).stdout.strip()


def world(base: Path) -> tuple[Path, Path]:
    """An origin with a tagged release and two commits since, a shared
    checkout of it, and a linked worktree to run the script from."""
    origin = base / "origin.git"
    git(base, "init", "-q", "--bare", "-b", "main", str(origin))
    main = base / "main"
    git(base, "clone", "-q", str(origin), str(main))
    git(main, "config", "user.email", "dev@example.com")
    git(main, "config", "user.name", "Dev")
    (main / "scripts").mkdir()
    shutil.copy(SCRIPTS / "release-bump.py", main / "scripts" / "release-bump.py")
    shutil.copy(SCRIPT, main / "scripts" / "release-prepare.sh")
    (main / "scripts" / "lib").mkdir()
    shutil.copy(SCRIPTS / "lib" / "release.sh", main / "scripts" / "lib" / "release.sh")
    (main / "Cargo.toml").write_text(
        '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.4.2"\nrust-version = "1.98"\n',
        encoding="utf-8",
    )
    (main / "crates" / "postio-gtk" / "data").mkdir(parents=True)
    (main / "crates" / "postio-gtk" / "data" / "dev.postio.Postio.metainfo.xml").write_text(METAINFO, encoding="utf-8")
    (main / "macos" / "Resources").mkdir(parents=True)
    (main / "macos" / "Resources" / "Info.plist").write_text(PLIST, encoding="utf-8")
    git(main, "add", "-A")
    git(main, "commit", "-q", "-m", "chore(release): v0.4.2")
    git(main, "tag", "v0.4.2")
    for subject in ("feat(gtk): a thing people will notice", "fix(sync): a thing that was broken"):
        (main / "README.md").write_text(subject + "\n", encoding="utf-8")
        git(main, "add", "README.md")
        git(main, "commit", "-q", "-m", subject)
    git(main, "push", "-q", "origin", "main", "--tags")
    tree = base / "tree"
    git(main, "worktree", "add", "-q", "-b", "scratch", str(tree), "origin/main")
    return main, tree


def run(cwd: Path, *args: str):
    stub = cwd.parent / "stub"
    (stub).mkdir(exist_ok=True)
    cargo = stub / "cargo"
    cargo.write_text('#!/usr/bin/env bash\necho "$*" >> "$(dirname "$0")/cargo-calls"\n', encoding="utf-8")
    cargo.chmod(0o755)
    env = {**os.environ, **GIT_ENV, "PATH": f"{stub}:{os.environ['PATH']}",
           "GIT_AUTHOR_NAME": "Dev", "GIT_AUTHOR_EMAIL": "dev@example.com",
           "GIT_COMMITTER_NAME": "Dev", "GIT_COMMITTER_EMAIL": "dev@example.com"}
    return patience.run(
        ["bash", "scripts/release-prepare.sh", *args],
        cwd=cwd, env=env, capture_output=True, text=True, timeout=60,
    )


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    # ── the release PR ───────────────────────────────────────────────
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as d:
        _, tree = world(Path(d))
        r = run(tree, "0.5.0")
        out = r.stdout + r.stderr
        case("prepares 0.5.0 from a clean worktree", r.returncode == 0, out)
        case(
            "on a branch of its own, named for the release",
            git(tree, "rev-parse", "--abbrev-ref", "HEAD") == "chore/release-v0.5.0",
            git(tree, "rev-parse", "--abbrev-ref", "HEAD"),
        )
        case(
            "one commit on origin/main, subject chore(release): v0.5.0",
            git(tree, "log", "--format=%s", "origin/main..HEAD") == "chore(release): v0.5.0",
            git(tree, "log", "--oneline", "origin/main..HEAD"),
        )
        case("the tree is clean afterwards -- everything is in the commit", git(tree, "status", "--porcelain") == "", git(tree, "status", "--short"))
        cargo = (tree / "Cargo.toml").read_text(encoding="utf-8")
        case("the workspace version is bumped", 'version = "0.5.0"' in cargo, cargo)
        case("Info.plist is bumped with it", "<string>0.5.0</string>" in (tree / "macos/Resources/Info.plist").read_text(encoding="utf-8"), "")
        notes = tree / "docs" / "releases" / "0.5.0.md"
        text = notes.read_text(encoding="utf-8") if notes.exists() else ""
        case(
            "the notes are drafted from the commits since the last tag",
            "a thing people will notice" in text and "a thing that was broken" in text,
            text or "no docs/releases/0.5.0.md",
        )
        case("the previous release commit is not a note", "chore(release)" not in text, text)
        case(
            "the notes are in the commit, so the PR is where they are edited",
            "docs/releases/0.5.0.md" in git(tree, "show", "--name-only", "--format=", "HEAD"),
            git(tree, "show", "--stat", "HEAD"),
        )
        case(
            "Cargo.lock is refreshed through cargo",
            "metadata" in (tree.parent / "stub" / "cargo-calls").read_text(encoding="utf-8")
            if (tree.parent / "stub" / "cargo-calls").exists() else False,
            "cargo was never asked",
        )
        case("it says what to do next", "issue-land.sh" in out, out)

    # Hand-written notes win, as they always have.
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as d:
        main, tree = world(Path(d))
        (main / "docs" / "releases").mkdir(parents=True)
        (main / "docs" / "releases" / "0.5.0.md").write_text("Written by a person.\n", encoding="utf-8")
        git(main, "add", "-A")
        git(main, "commit", "-q", "-m", "docs: notes for 0.5.0")
        git(main, "push", "-q", "origin", "main")
        r = run(tree, "0.5.0")
        text = (tree / "docs" / "releases" / "0.5.0.md").read_text(encoding="utf-8")
        case("existing notes are kept as written", r.returncode == 0 and text == "Written by a person.\n", r.stdout + r.stderr + text)

    # ── refusals ─────────────────────────────────────────────────────
    for bad, why in (("0.4.2", "the version that is already out"), ("0.4.1", "a version behind the newest tag"), ("v0.5.0", "a v-prefixed version"), ("0.5", "a non-semver version")):
        with tempfile.TemporaryDirectory(dir=SANDBOXES) as d:
            _, tree = world(Path(d))
            before = git(tree, "rev-parse", "HEAD")
            r = run(tree, bad)
            case(f"refuses {why} ({bad})", r.returncode != 0, r.stdout + r.stderr)
            case(f"...and leaves the tree as it was ({bad})",
                 git(tree, "rev-parse", "HEAD") == before and git(tree, "status", "--porcelain") == ""
                 and git(tree, "rev-parse", "--abbrev-ref", "HEAD") == "scratch",
                 git(tree, "status", "--short"))

    with tempfile.TemporaryDirectory(dir=SANDBOXES) as d:
        _, tree = world(Path(d))
        (tree / "stray.txt").write_text("uncommitted\n", encoding="utf-8")
        r = run(tree, "0.5.0")
        case("refuses a dirty tree", r.returncode != 0 and git(tree, "rev-parse", "--abbrev-ref", "HEAD") == "scratch", r.stdout + r.stderr)

    with tempfile.TemporaryDirectory(dir=SANDBOXES) as d:
        main, _ = world(Path(d))
        r = run(main, "0.5.0")
        case(
            "refuses the shared checkout, where switching branches moves everyone",
            r.returncode != 0 and git(main, "rev-parse", "--abbrev-ref", "HEAD") == "main",
            r.stdout + r.stderr,
        )
        case("...and says to use a worktree", "worktree" in (r.stdout + r.stderr), r.stdout + r.stderr)

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.")
        return 1
    print("release-prepare self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
