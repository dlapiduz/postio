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
    turned into `-`;
  * with no `--app`, the app is Focus, the one desktop app (ADR 0043,
    specs/007-postio-focus T265): its runner is postio-focus's.

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
echo "display ${POSTIO_TEST_DISPLAY:-}" >> "$FAKE_LOG"
echo "start $$" >> "$FAKE_LOG"
sleep "${RUNNER_SLEEP:-0}"
echo "end $$" >> "$FAKE_LOG"
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
    (catalogue / "gaps" / "focus.toml").write_text("")
    (catalogue / "README.md").write_text("")
    (repo / "crates" / "postio-focus").mkdir(parents=True)
    (repo / "crates" / "postio-focus" / "Cargo.toml").write_text("")
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

        print("case: run --only selects by path and builds once, Focus's runner by default")
        result = run(ctx, "run", "--only", "list/*", "--jobs", "1")
        lines = log_lines(ctx)
        builds = [l for l in lines if l.startswith("cargo build") and "--example storyboard" in l]
        expect("default-app", all("-p postio-focus" in l for l in builds) and builds,
               f"the default runner is Focus's: {builds}")
        runner_calls = [l for l in lines if l.startswith("runner run")]
        expect("only", result.returncode == 0, f"exit {result.returncode}: {result.stderr.strip()}")
        expect("only", len(builds) == 1, f"the runner is built once, saw {builds}")
        expect("only", len(runner_calls) == 1, f"one runner call, saw {runner_calls}")
        call = runner_calls[0] if runner_calls else ""
        expect("only", "list/archive-walks-down.toml" in call, call)
        expect("only", "list/launch-keyboard-on-first-row.toml" in call, call)
        expect("only", "escape-leaves-search" not in call, "search is not selected")
        expect("only", "--out" in call and "Design/review/feature-storyboards/runs" in call, call)

        print("case: a plain run never plays calibration storyboards or gap lists")
        run(ctx, "run", "--jobs", "1")
        call = next((l for l in log_lines(ctx) if l.startswith("runner run")), "")
        expect("calibration", "escape-leaves-search.toml" in call, call)
        expect("calibration", "calibration/" not in call and "gaps/" not in call, call)

        print("case: --calibration plays only the calibration set")
        run(ctx, "run", "--calibration", "--jobs", "1")
        call = next((l for l in log_lines(ctx) if l.startswith("runner run")), "")
        expect("calibration-only", "calibration/archive-returns-to-top.toml" in call, call)
        expect("calibration-only", "list/" not in call and "search/" not in call, call)

        print("case: exit codes pass through as 0, 1, 2")
        for code in ("1", "2"):
            result = run(ctx, "run", RUNNER_EXIT=code)
            expect("exit", result.returncode == int(code), f"runner {code} -> {result.returncode}")

        print("case: --jobs splits the storyboards across runners on their own displays")
        result = run(ctx, "run", "--jobs", "2", RUNNER_SLEEP="0.5")
        lines = log_lines(ctx)
        calls = [l for l in lines if l.startswith("runner run")]
        expect("jobs", result.returncode == 0, f"exit {result.returncode}: {result.stderr.strip()}")
        expect("jobs", len(calls) == 2, f"two runners, saw {calls}")
        played = sorted(
            word for call in calls for word in call.split() if word.endswith(".toml")
        )
        expect(
            "jobs",
            sorted(Path(p).name for p in played)
            == ["archive-walks-down.toml", "escape-leaves-search.toml", "launch-keyboard-on-first-row.toml"],
            f"every storyboard played once: {played}",
        )
        displays = {l.split()[-1] for l in lines if l.startswith("display ")}
        expect("jobs", len(displays) == 2, f"a compositor each: {displays}")
        order = [l.split()[0] for l in lines if l.split()[0] in ("start", "end")]
        expect("jobs", order[:2] == ["start", "start"], f"the runners overlap: {order}")
        result = run(ctx, "run", "--jobs", "2", RUNNER_EXIT="1")
        expect("jobs", result.returncode == 1, f"the worst shard's exit, got {result.returncode}")

        print("case: an --only that matches nothing says so and fails")
        result = run(ctx, "run", "--only", "nowhere/*")
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
        result = run(ctx, "key")
        call = next((l for l in log_lines(ctx) if l.startswith("tool key")), "")
        expect("key", result.returncode == 0, f"exit {result.returncode}: {result.stderr.strip()}")
        expect("key", f"--tree storyboards={tree}" in call, call)
        expect("key", "--tree crates/postio-widgets=absent" in call, call)
        expect("key", "crates/postio-focus=" in call and "crates/postio-focus=absent" not in call, call)
        expect("key", "postio-gtk" not in call and "postio-app" not in call,
               f"Focus's key names no classic crate: {call}")

        print("case: bundle points the tool at the runs, the catalogue and the design screens")
        acceptance = Path(ctx["repo"]) / "acceptance.md"
        acceptance.write_text("Escape returns to the row.\n")
        result = run(ctx, "bundle", "--acceptance", str(acceptance))
        call = next((l for l in log_lines(ctx) if l.startswith("tool bundle")), "")
        expect("bundle", result.returncode == 0, f"exit {result.returncode}: {result.stderr.strip()}")
        expect("bundle", "Design/review/feature-storyboards/runs" in call, call)
        expect("bundle", "--catalogue" in call and call.split("--catalogue ")[1].split()[0].endswith("storyboards"), call)
        expect("bundle", "--design-dir" in call and "Design/screens" in call, call)
        expect("bundle", "Design/review/feature-storyboards/bundle" in call, call)
        expect("bundle", "postio-focus-design" not in call, "the untracked Focus design folder is never offered")

        print("case: tool passes its arguments to postio-storyboard")
        run(ctx, "tool", "verdicts", "check", "/some/bundle")
        call = next((l for l in log_lines(ctx) if l.startswith("tool verdicts")), "")
        expect("tool", call == "tool verdicts check /some/bundle", call)

        print("case: base plays the branch's storyboards on the merge-base, cached")
        repo = Path(ctx["repo"])
        (repo / "crates" / "postio-focus" / "examples").mkdir(parents=True, exist_ok=True)
        (repo / "crates" / "postio-focus" / "examples" / "storyboard.rs").write_text("")
        gitdir = subprocess.run(["git", "-C", str(repo), "rev-parse", "--git-dir"],
                                capture_output=True, text=True, check=True).stdout.strip()
        gitdir = (repo / gitdir) if not gitdir.startswith("/") else Path(gitdir)
        subprocess.run(["git", "-C", str(repo), "add", "-A"], check=True)
        subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.com",
                        "commit", "-qm", "base", "--allow-empty"], check=True)
        subprocess.run(["git", "-C", str(repo), "branch", "-f", "main"], check=True)
        base_sha = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"],
                                  capture_output=True, text=True, check=True).stdout.strip()
        subprocess.run(["git", "-C", str(repo), "update-ref", "refs/remotes/origin/main", base_sha], check=True)
        (gitdir / "postio-base").write_text("main\n")
        (repo / "storyboards" / "list" / "archive-walks-down.toml").write_text(
            'source = { kind = "flow", ref = "changed on the branch" }\n')
        subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.com",
                        "commit", "-qam", "branch"], check=True)
        cache = Path(ctx["env"]["FAKE_TARGET"]).parent / "cache"
        result = run(ctx, "base", STORYBOARDS_CACHE=str(cache))
        lines = log_lines(ctx)
        expect("base", result.returncode == 0, f"exit {result.returncode}: {result.stderr.strip()}")
        tree = repo / "target" / "storyboard-base" / "tree"
        head = subprocess.run(["git", "-C", str(tree), "rev-parse", "HEAD"],
                              capture_output=True, text=True).stdout.strip()
        expect("base", head == base_sha, f"the base tree is at the merge-base: {head} vs {base_sha}")
        call = next((l for l in lines if l.startswith("runner run")), "")
        expect("base", str(repo / "storyboards" / "list" / "archive-walks-down.toml") in call,
               f"the branch's storyboard is played: {call}")
        expect("base", f"--out {cache / base_sha}" in call, f"into the cache: {call}")
        link = repo / "Design" / "review" / "feature-storyboards" / "base"
        expect("base", link.is_symlink() and link.resolve() == (cache / base_sha).resolve(),
               f"the review links the base: {link}")
        result = run(ctx, "base", STORYBOARDS_CACHE=str(cache))
        again = log_lines(ctx)
        expect("base-cache", not any(l.startswith("runner run") for l in again),
               f"a second call with nothing changed plays nothing: {again}")

        print("case: a base with no runner is reported, not failed")
        (repo / "crates" / "postio-focus" / "examples" / "storyboard.rs").unlink()
        subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.com",
                        "commit", "-qam", "drop the runner"], check=True)
        subprocess.run(["git", "-C", str(repo), "update-ref", "refs/remotes/origin/main", "HEAD"], check=True)
        result = run(ctx, "base", STORYBOARDS_CACHE=str(cache))
        expect("no-runner", result.returncode == 0, f"exit {result.returncode}")
        expect("no-runner", "predates the runner" in result.stdout, result.stdout)

        print("case: screens plays the screen storyboards in their variants and sheets them")
        (Path(ctx["repo"]) / "storyboards" / "screens").mkdir(parents=True, exist_ok=True)
        (Path(ctx["repo"]) / "storyboards" / "screens" / "inbox.toml").write_text(
            'source = { kind = "design", ref = "01" }\n')
        result = run(ctx, "screens")
        lines = log_lines(ctx)
        call = next((l for l in lines if l.startswith("runner run")), "")
        sheet = next((l for l in lines if l.startswith("tool sheet")), "")
        expect("screens", "screens/inbox.toml" in call and "list/" not in call, call)
        expect("screens", "--variants" in call and "screens/runs" in call, call)
        builds = [l for l in lines if l.startswith("cargo build") and "--example storyboard" in l]
        expect("screens", builds and all("-p postio-focus" in l for l in builds),
               f"the sweep films Focus: {builds}")
        expect("screens", "--design-dir" in sheet and "Design/screens" in sheet and "screens/index.html" in sheet, sheet)
        result = run(ctx, "screens", TOOL_EXIT="1")
        expect("screens", result.returncode == 1, f"a screen that failed to render fails the sweep: {result.returncode}")

    if FAILURES:
        print(f"\n{len(FAILURES)} self-test assertion(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print("\nall storyboards.sh self-tests passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
