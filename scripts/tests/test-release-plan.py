#!/usr/bin/env python3
"""Self-test for scripts/release-plan.sh.

`release.yml` runs on every push to `main` (#1714) and its first job asks
this script whether that push is a release. The answer has to be "no" for
almost every push and cost nothing to give, "yes" exactly once per version,
and never "yes" for a version that is already out -- a second run for v0.5.0
would rebuild and re-upload over a published release.

It also decides whether the full suite runs. A release does not ship without
it, but the nightly runs the identical suite, so a green nightly on the
*exact* commit is the same answer already given; anything less than the
exact commit is not.

A real git repository per case (tags are the record of what was released),
and `gh` stubbed to answer the nightly question.

Usage: scripts/tests/test-release-plan.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "release-plan.sh"

FAILURES: list[str] = []

# Answers "how many successful nightly runs at this SHA" from a file, and
# records what it was asked. No file: the API failed.
GH_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/gh-args"
[ -f "$STUB_DIR/nightly-count" ] || { echo "HTTP 502" >&2; exit 1; }
cat "$STUB_DIR/nightly-count"
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=repo, check=True, capture_output=True, text=True,
        env={**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null"},
    ).stdout.strip()


def repo(base: Path, *, version: str, tags: list[str]) -> Path:
    root = base / "repo"
    root.mkdir()
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.email", "ci@example.com")
    git(root, "config", "user.name", "CI")
    (root / "Cargo.toml").write_text(
        f'[workspace]\nmembers = []\n\n[workspace.package]\nversion = "{version}"\nrust-version = "1.98"\n',
        encoding="utf-8",
    )
    git(root, "add", "Cargo.toml")
    git(root, "commit", "-q", "-m", "a commit")
    for tag in tags:
        git(root, "tag", tag)
    # The release commit itself, after the tags.
    (root / "README.md").write_text("x\n", encoding="utf-8")
    git(root, "add", "README.md")
    git(root, "commit", "-q", "-m", "the commit being pushed")
    return root


def plan(
    base: Path,
    *,
    version: str,
    tags: list[str],
    event: str = "push",
    nightly: str | None = "0",
    extra: tuple[str, ...] = (),
) -> tuple[dict[str, str], subprocess.CompletedProcess]:
    root = repo(base, version=version, tags=tags)
    stub = base / "stub"
    (stub / "bin").mkdir(parents=True)
    (stub / "bin" / "gh").write_text(GH_STUB, encoding="utf-8")
    (stub / "bin" / "gh").chmod(0o755)
    if nightly is not None:
        (stub / "nightly-count").write_text(nightly + "\n", encoding="utf-8")
    env = dict(os.environ)
    env.update(
        PATH=f"{stub / 'bin'}:{env['PATH']}",
        STUB_DIR=str(stub),
        GITHUB_EVENT_NAME=event,
        GITHUB_SHA=git(root, "rev-parse", "HEAD"),
        GITHUB_REPOSITORY="example/postio",
        GIT_CONFIG_GLOBAL="/dev/null",
    )
    proc = patience.run(
        ["bash", str(SCRIPT), *extra], cwd=root, env=env, capture_output=True, text=True, timeout=30,
    )
    outputs = {}
    for line in proc.stdout.splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            outputs[key] = value
    return outputs, proc


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    # ── the ordinary push: nothing to release ─────────────────────────
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.4.2", tags=["v0.4.1", "v0.4.2"])
        case(
            "a push whose version is already tagged builds nothing",
            proc.returncode == 0 and out.get("build") == "false" and out.get("publish") == "false",
            f"{out} / {proc.stderr}",
        )

    # main said 0.3.0 while v0.4.2 existed; the version it names is already
    # out, and it is not the newest either.
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.3.0", tags=["v0.3.0", "v0.4.2"])
        case("a push behind the newest tag builds nothing", out.get("build") == "false", f"{out} / {proc.stderr}")

    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.4.0", tags=["v0.4.2"])
        case(
            "an untagged version older than the newest tag is refused, not released",
            out.get("build") == "false" and "0.4.2" in proc.stderr,
            f"{out} / {proc.stderr}",
        )

    # ── the release PR's merge ───────────────────────────────────────
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.5.0", tags=["v0.4.2"])
        case(
            "a new version on main is a release",
            out.get("build") == "true" and out.get("publish") == "true",
            f"{out} / {proc.stderr}",
        )
        case("...tagged v<version>", out.get("tag") == "v0.5.0", str(out))
        case("...versioned as the workspace says", out.get("version") == "0.5.0", str(out))
        case("...and a 0.x is a pre-release", out.get("prerelease") == "true", str(out))
        case("...and with no green nightly on this commit, the suite runs", out.get("suite") == "run", str(out))

    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="1.0.0", tags=["v0.4.2"])
        case("a 1.x is not a pre-release", out.get("prerelease") == "false", str(out))

    # Tags compare as versions, not strings: v0.10.0 is newer than v0.9.0.
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.10.0", tags=["v0.9.0"])
        case("0.10.0 is newer than v0.9.0", out.get("publish") == "true", f"{out} / {proc.stderr}")

    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.5.0", tags=[])
        case("a first release with no tags at all is a release", out.get("publish") == "true", f"{out} / {proc.stderr}")

    # ── the suite ────────────────────────────────────────────────────
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.5.0", tags=["v0.4.2"], nightly="1")
        case("a green nightly on this exact commit skips the suite", out.get("suite") == "skip", f"{out} / {proc.stderr}")
        asked = (Path(d) / "stub" / "gh-args").read_text(encoding="utf-8")
        case(
            "...asking about this commit's nightly runs that succeeded",
            "nightly.yml" in asked and "head_sha=" in asked and "status=success" in asked,
            asked,
        )
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.5.0", tags=["v0.4.2"], nightly=None)
        case("no answer about the nightly runs the suite", out.get("suite") == "run", f"{out} / {proc.stderr}")

    # ── by hand ─────────────────────────────────────────────────────
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.4.2", tags=["v0.4.2"], event="workflow_dispatch")
        case(
            "a dispatch builds every package and publishes nothing",
            out.get("build") == "true" and out.get("publish") == "false",
            f"{out} / {proc.stderr}",
        )
        case(
            "...versioned so it cannot be mistaken for the release",
            out.get("version", "").startswith("0.4.2-dev.") and len(out.get("version", "")) > len("0.4.2-dev."),
            str(out),
        )
        case("...with no tag", out.get("tag", "") == "", str(out))
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.4.2", tags=["v0.4.2"], event="workflow_dispatch", extra=("--skip-suite",))
        case("a dispatch may skip the suite when asked", out.get("suite") == "skip", str(out))
    with tempfile.TemporaryDirectory() as d:
        out, proc = plan(Path(d), version="0.5.0", tags=["v0.4.2"], extra=("--skip-suite",))
        case("a real release never skips the suite on request", out.get("suite") == "run", str(out))

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.")
        return 1
    print("release-plan self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
