#!/usr/bin/env python3
"""Self-test for #559: the issue workflow must run on BSD userland too.

Three scripts used GNU-only syntax, which made the whole workflow unusable on
macOS -- you could not claim an issue, so you could not do anything else:

  * `issue-claim.sh` built the branch slug with `sed 's/[^a-z0-9]\\+/-/g'`.
    BSD sed has no `\\+`, so the substitution matched nothing, the title passed
    through with its spaces and colons, and git refused the ref:
        fatal: 'issue-552-issue-claim.sh: claim from a label other' is not a
        valid branch name
  * `issue-land.sh` extracted the issue number with the same `\\+`. On BSD it
    yielded the empty string, and the guard below it reported "not an issue
    branch" -- true-sounding, and about the wrong thing entirely.
  * `issue-release.sh` aged claims with GNU `date -d`, which BSD date rejects.

Two kinds of case, because they fail in different places:

  * a *behavioural* case, which is red on BSD and green on GNU. It runs the
    real script and asserts the branch name, so it fails exactly where a
    contributor does.
  * a *source* case, which is red on both. `\\+` in a sed expression is wrong
    on GNU too -- it happens to work there -- so the constraint is stated once
    and enforced everywhere, rather than being a thing only Mac owners find.

The source case covers every script except the three that are Linux-only by
their nature -- they drive mutter, or install into the XDG hicolor layout.
Those are listed with a reason in LINUX_ONLY_SCRIPTS; a script is not exempt
because it happens to fail, it is exempt because a Mac will never run it.

Usage: scripts/tests/test-scripts-bsd-portable.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
ISSUE_CLAIM = SCRIPTS / "issue-claim.sh"

# A title with the punctuation that actually broke it: a dot inside a word, a
# colon-then-space run, and ordinary spaces. This is #552's real title.
FIXTURE_TITLE = "issue-claim.sh: claim from a label other than ready"

FIXTURE_ISSUES = [
    {
        "number": 552,
        "title": FIXTURE_TITLE,
        "labels": [{"name": "ready"}, {"name": "p0"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
]

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/issues.json"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "stub issue"; exit 0; fi
exit 1
"""

# The GNU-only constructs that have bitten this repository, each with the
# portable spelling that replaces it. Kept as a table rather than one regex so
# a failure can name the fix, the way every check in scripts/checks/ does.
GNU_ONLY = [
    (
        re.compile(r"\\\+"),
        r"`\+` is a GNU BRE extension; BSD sed matches it literally",
        r"use `[x][x]*` instead of `[x]\+`",
    ),
    (
        re.compile(r"\bdate\s+-d\b"),
        "`date -d` is GNU; BSD date rejects it",
        "use `date -j -f`, or compute the age in python3",
    ),
    (
        re.compile(r"\bgrep\s+-[a-zA-Z]*P\b"),
        "`grep -P` is GNU; BSD grep has no PCRE mode",
        "use a POSIX ERE with `grep -E`",
    ),
]

# The scripts that are Linux-only by their nature, and the reason each one is.
# Per-file and spelled out, not a pattern: the point of the source case is that
# a *new* script has to be argued into this list, and a glob would quietly
# accept the next one. The same reasoning as the allowlist in
# check-no-silent-tracking.py.
LINUX_ONLY_SCRIPTS = {
    "headless-runner.sh": "drives `mutter --headless` and reads /proc",
    "test-headless.sh": "the same compositor, started by hand",
    "install-local.sh": "XDG hicolor layout, .desktop files, gtk-update-icon-cache",
}

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def expected_slug(title: str) -> str:
    """What the slug must be, derived independently of the shell.

    Mirrors issue-claim.sh's pipeline in order -- lowercase, collapse runs of
    non-alphanumerics, strip one leading and one trailing dash, then truncate
    -- rather than asserting a hardcoded string, so this stays honest if the
    fixture title changes.
    """
    slug = re.sub(r"[^a-z0-9]+", "-", title.lower())
    slug = re.sub(r"^-", "", slug)
    slug = re.sub(r"-$", "", slug)
    return slug[:40]


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def world(base: Path) -> tuple[Path, Path]:
    """A fixture repo with a local bare origin, and a stubbed gh on PATH."""
    repo = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (repo / "scripts").mkdir(parents=True)
    shutil.copy(ISSUE_CLAIM, repo / "scripts" / "issue-claim.sh")
    (repo / "scripts" / "issue-claim.sh").chmod(0o755)
    shutil.copytree(SCRIPTS / "lib", repo / "scripts" / "lib")

    gh = stub_dir / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub_dir / "issues.json").write_text(json.dumps(FIXTURE_ISSUES), encoding="utf-8")

    git("init", "-q", "-b", "main", cwd=repo)
    (repo / "README.md").write_text("fixture repo\n", encoding="utf-8")
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=repo)
    git("push", "-q", "origin", "main", cwd=repo)
    return repo, stub_dir


def run_claim(repo: Path, stub_dir: Path, base: Path, *args: str):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
        cwd=repo,
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def behaviour() -> None:
    """The claim script, run for real, on a title full of punctuation."""
    want = expected_slug(FIXTURE_TITLE)
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        repo, stub_dir = world(base)

        result = run_claim(repo, stub_dir, base, "--dry-run")
        out = result.stdout + result.stderr
        match = re.search(r"^\s*branch:[ \t]*(.+?)[ \t]*$", result.stdout, re.MULTILINE)

        case(
            "the dry run reports a branch at all",
            match is not None,
            f"no 'branch:' line in output:\n{out}",
        )
        if match is None:
            return

        got = match.group(1)
        case(
            "punctuation in the title becomes dashes in the branch",
            got == f"issue-552-{want}",
            f"got {got!r}, want {'issue-552-' + want!r}",
        )
        case(
            "the branch name is one git will accept",
            subprocess.run(
                ["git", "check-ref-format", "--branch", got],
                capture_output=True,
            ).returncode
            == 0,
            f"git rejects the ref {got!r} -- this is the #559 failure exactly",
        )
        # The slug is the branch, and the branch is a path component of the
        # worktree directory. A space here is not only an invalid ref, it is a
        # directory name every later command has to quote correctly.
        case(
            "the branch carries no shell-hostile characters",
            re.fullmatch(r"[a-z0-9-]+", got) is not None,
            f"branch {got!r} is not restricted to [a-z0-9-]",
        )


def sources() -> None:
    """No GNU-only construct in any shell script, on any platform."""
    every = sorted(SCRIPTS.glob("*.sh"))
    scripts = [f for f in every if f.name not in LINUX_ONLY_SCRIPTS]
    case("there are shell scripts to check", bool(scripts), "no *.sh found")

    # An exemption for a script that no longer exists is an exemption nobody
    # will notice has stopped meaning anything.
    names = {f.name for f in every}
    stale = sorted(set(LINUX_ONLY_SCRIPTS) - names)
    case(
        "no exemption names a script that is gone",
        not stale,
        f"LINUX_ONLY_SCRIPTS still lists {stale}",
    )

    for pattern, why, fix in GNU_ONLY:
        hits = []
        for script in scripts:
            for number, line in enumerate(
                script.read_text(encoding="utf-8").splitlines(), 1
            ):
                if line.lstrip().startswith("#"):
                    continue
                if pattern.search(line):
                    hits.append(f"{script.relative_to(SCRIPTS.parent)}:{number}: {line.strip()}")
        case(
            f"no GNU-only construct: {why}",
            not hits,
            "\n      " + "\n      ".join(hits) + f"\n      fix: {fix}",
        )


def main() -> int:
    behaviour()
    sources()

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        print(
            "\nscripts/ runs on BSD userland (macOS) as well as GNU (Linux).",
            file=sys.stderr,
        )
        return 1
    print("scripts BSD-portability check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
