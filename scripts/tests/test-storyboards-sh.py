#!/usr/bin/env python3
"""Self-test for scripts/storyboards.sh (specs/008-storyboards T043).

The script is the one command people and skills use to play storyboards,
so what it promises is tested here against stand-ins rather than a real
build: a fake `cargo` on PATH that records every invocation and answers
`cargo metadata` with a temporary target directory, and a stub runner that
records what it was asked to play and exits with whatever the case wants.

What is asserted:
  * `run --only <glob>` selects storyboards by their path under the
    catalogue, without `.toml`, and never plays calibration storyboards;
  * the runner is built once per invocation, not once per storyboard;
  * exit codes: 0 when every run passed, 1 when one failed, 2 when the
    runner could not run;
  * `lint` hands the catalogue to `postio-storyboard lint`;
  * runs go under Design/review/<branch>/runs, with the branch's `/`
    turned into `-`.

No network, no display, and the real repository is never written to.

Usage: scripts/tests/test-storyboards-sh.py
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
SCRIPT = HERE / "storyboards.sh"

FAILURES: list[str] = []


def expect(case: str, cond: bool, detail: str) -> None:
    print(f"  {'ok  ' if cond else 'FAIL'} {case}: {detail}")
    if not cond:
        FAILURES.append(f"{case}: {detail}")


FAKE_CARGO = """#!/usr/bin/env bash
echo "cargo $*" >> "$FAKE_LOG"
case "$1" in
  metadata) printf '{"target_directory":"%s"}\\n' "$FAKE_TARGET" ;;
  build)
    mkdir -p "$FAKE_TARGET/debug/examples"
    cp "$FAKE_RUNNER" "$FAKE_TARGET/debug/examples/storyboard"
    cp "$FAKE_TOOL" "$FAKE_TARGET/debug/postio-storyboard"
    ;;
esac
"""

# Records its arguments; exits with $RUNNER_EXIT; writes a run.json per
# storyboard so `page` has something to read.
FAKE_RUNNER = """#!/usr/bin/env bash
echo "runner $*" >> "$FAKE_LOG"
exit "${RUNNER_EXIT:-0}"
"""

FAKE_TOOL = """#!/usr/bin/env bash
echo "tool $*" >> "$FAKE_LOG"
exit "${TOOL_EXIT:-0}"
"""


def setup(tmp: Path) -> dict[str, str]:
    repo = tmp / "repo"
    (repo / "scripts").mkdir(parents=True)
    shutil.copy(SCRIPT, repo / "scripts" / "storyboards.sh")
    # The script runs the headless wrapper; a pass-through stands in.
    wrap = repo / "scripts" / "test-headless.sh"
    wrap.write_text('#!/usr/bin/env bash\nexec "$@"\n')
    wrap.chmod(0o755)
    catalogue = repo / "storyboards"
    for path in (
        "list/archive-walks-down.toml",
        "list/launch-keyboard-on-first-row.toml",
        "search/escape-leaves-search.toml",
        "calibration/archive-returns-to-top.toml",
    ):
        (catalogue / path).parent.mkdir(parents=True, exist_ok=True)
        (catalogue / path).write_text('source = { kind = "flow", ref = "x" }\n')
    (catalogue / "gaps").mkdir()
    (catalogue / "gaps" / "classic.toml").write_text("")
    (catalogue / "README.md").write_text("")
    (repo / "crates" / "postio-app").mkdir(parents=True)
    (repo / "crates" / "postio-app" / "Cargo.toml").write_text("")
    subprocess.run(["git", "init", "-q", "-b", "feature/storyboards", str(repo)], check=True)

    bin_dir = tmp / "bin"
    bin_dir.mkdir()
    for name, body in (("cargo", FAKE_CARGO),):
        path = bin_dir / name
        path.write_text(body)
        path.chmod(0o755)
    runner = tmp / "runner"
    runner.write_text(FAKE_RUNNER)
    runner.chmod(0o755)
    tool = tmp / "tool"
    tool.write_text(FAKE_TOOL)
    tool.chmod(0o755)
    log = tmp / "log"
    log.write_text("")
    env = dict(os.environ)
    env.update(
        PATH=f"{bin_dir}:{env['PATH']}",
        FAKE_LOG=str(log),
        FAKE_TARGET=str(tmp / "target"),
        FAKE_RUNNER=str(runner),
        FAKE_TOOL=str(tool),
    )
    env.pop("CARGO_TARGET_DIR", None)
    return {"repo": str(repo), "log": str(log), **{"env": env}}  # type: ignore[dict-item]


def run(ctx: dict, *args: str, **extra_env: str) -> subprocess.CompletedProcess:
    env = dict(ctx["env"])
    env.update(extra_env)
    Path(ctx["log"]).write_text("")
    return subprocess.run(
        ["bash", "scripts/storyboards.sh", *args],
        cwd=ctx["repo"],
        env=env,
        capture_output=True,
        text=True,
    )


def log_lines(ctx: dict) -> list[str]:
    return Path(ctx["log"]).read_text().splitlines()


def main() -> int:
    if not SCRIPT.exists():
        print(f"FAIL: {SCRIPT} does not exist", file=sys.stderr)
        return 1
    with tempfile.TemporaryDirectory() as tmp_name:
        ctx = setup(Path(tmp_name))

        print("case: run --only selects by path and builds once")
        result = run(ctx, "run", "--app", "classic", "--only", "list/*")
        lines = log_lines(ctx)
        builds = [l for l in lines if l.startswith("cargo build")]
        runner_calls = [l for l in lines if l.startswith("runner run")]
        expect("only", result.returncode == 0, f"exit {result.returncode}: {result.stderr.strip()}")
        expect("only", len(builds) == 1, f"one build, saw {builds}")
        expect("only", len(runner_calls) == 1, f"one runner call, saw {runner_calls}")
        call = runner_calls[0] if runner_calls else ""
        expect("only", "list/archive-walks-down.toml" in call, call)
        expect("only", "list/launch-keyboard-on-first-row.toml" in call, call)
        expect("only", "escape-leaves-search" not in call, "search is not selected")
        expect("only", "--out" in call and "Design/review/feature-storyboards/runs" in call, call)

        print("case: a plain run never plays calibration storyboards or gap lists")
        run(ctx, "run", "--app", "classic")
        call = next((l for l in log_lines(ctx) if l.startswith("runner run")), "")
        expect("calibration", "escape-leaves-search.toml" in call, call)
        expect("calibration", "calibration/" not in call and "gaps/" not in call, call)

        print("case: --calibration plays only the calibration set")
        run(ctx, "run", "--app", "classic", "--calibration")
        call = next((l for l in log_lines(ctx) if l.startswith("runner run")), "")
        expect("calibration-only", "calibration/archive-returns-to-top.toml" in call, call)
        expect("calibration-only", "list/" not in call and "search/" not in call, call)

        print("case: exit codes pass through as 0, 1, 2")
        for code in ("1", "2"):
            result = run(ctx, "run", "--app", "classic", RUNNER_EXIT=code)
            expect("exit", result.returncode == int(code), f"runner {code} -> {result.returncode}")

        print("case: an --only that matches nothing says so and fails")
        result = run(ctx, "run", "--app", "classic", "--only", "nowhere/*")
        expect("nothing", result.returncode == 2, f"exit {result.returncode}")
        expect("nothing", "no storyboard" in result.stderr, result.stderr.strip())

        print("case: lint hands the catalogue to postio-storyboard")
        result = run(ctx, "lint")
        call = next((l for l in log_lines(ctx) if l.startswith("tool lint")), "")
        expect("lint", result.returncode == 0, f"exit {result.returncode}")
        expect("lint", call.endswith("storyboards"), call)
        result = run(ctx, "lint", TOOL_EXIT="1")
        expect("lint", result.returncode == 1, f"a dirty catalogue exits 1, got {result.returncode}")

        print("case: page builds index.html from the runs")
        result = run(ctx, "page")
        call = next((l for l in log_lines(ctx) if l.startswith("tool page")), "")
        expect("page", result.returncode == 0, f"exit {result.returncode}: {result.stderr.strip()}")
        expect("page", "--runs" in call and "Design/review/feature-storyboards/runs" in call, call)
        expect("page", "Design/review/feature-storyboards/index.html" in call, call)

        print("case: key passes the tree ids of the app's crates and the catalogue")
        repo = ctx["repo"]
        subprocess.run(["git", "-C", repo, "add", "-A"], check=True)
        subprocess.run(
            ["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@example.com",
             "commit", "-qm", "seed"],
            check=True,
        )
        tree = subprocess.run(
            ["git", "-C", repo, "rev-parse", "HEAD:storyboards"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        result = run(ctx, "key", "--app", "classic")
        call = next((l for l in log_lines(ctx) if l.startswith("tool key")), "")
        expect("key", result.returncode == 0, f"exit {result.returncode}: {result.stderr.strip()}")
        expect("key", f"--tree storyboards={tree}" in call, call)
        expect("key", "--tree crates/postio-gtk=absent" in call, call)
        expect("key", "crates/postio-app=" in call and "crates/postio-app=absent" not in call, call)

    if FAILURES:
        print(f"\n{len(FAILURES)} self-test assertion(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print("\nall storyboards.sh self-tests passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
