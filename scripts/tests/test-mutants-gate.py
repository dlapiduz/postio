#!/usr/bin/env python3
"""Self-test for scripts/mutants.sh.

Same shape as test-coverage-gate.py: a stubbed `cargo mutants` stands in for
the real, hours-long thing, so the baseline-diff logic -- the part that has
never run before this file -- gets to be red-then-green like anything else.

Usage: scripts/tests/test-mutants-gate.py
Exit status: 0 the script behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
MUTANTS = HERE / "mutants.sh"

FAILURES: list[str] = []

CARGO_STUB_MISSING_TOOL = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/cargo-calls"
echo "error: no such command: \\`mutants\\`" >&2
exit 101
"""


def cargo_mutants_stub(survivors: list[str]) -> str:
    survivors_text = "\\n".join(survivors)
    # Finds `--output <dir>` in "$@" and writes mutants.out/missed.txt under
    # it, exactly where the real tool puts its report.
    return f"""#!/usr/bin/env bash
if [ "$1" = "mutants" ]; then
    args=("$@")
    outdir=""
    for i in "${{!args[@]}}"; do
        if [ "${{args[$i]}}" = "--output" ]; then
            outdir="${{args[$((i+1))]}}"
        fi
    done
    mkdir -p "$outdir/mutants.out"
    printf '{survivors_text}\\n' > "$outdir/mutants.out/missed.txt"
    exit 0
fi
echo "unexpected cargo subcommand in stub: $*" >&2
exit 99
"""


def run(
    env_extra: dict[str, str], args: list[str], repo: Path
) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment.update(env_extra)
    # mutants.sh finds its repo root from *its own* path
    # (`dirname "${BASH_SOURCE[0]}"`), not from the caller's cwd -- correct
    # for the real script, but it means invoking the real
    # scripts/mutants.sh here would read and write this repository's own
    # docs/mutants-baseline.txt. Running the sandboxed copy under
    # repo/scripts/ instead makes it resolve entirely inside the sandbox.
    return subprocess.run(
        ["bash", str(repo / "scripts" / "mutants.sh"), *args],
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def fake_repo(root: Path) -> Path:
    """A throwaway git repo with its own copy of mutants.sh, so the script's
    baseline reads and writes land inside the sandbox rather than touching
    this repository's own docs/mutants-baseline.txt."""
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    (root / "docs").mkdir()
    (root / "scripts").mkdir()
    (root / "scripts" / "mutants.sh").write_text(
        MUTANTS.read_text(encoding="utf-8"), encoding="utf-8"
    )
    (root / "scripts" / "mutants.sh").chmod(0o755)
    return root


def test_missing_tool_fails_before_touching_cargo() -> None:
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = Path(directory)
        binaries = stub_dir / "bin"
        binaries.mkdir()
        cargo = binaries / "cargo"
        cargo.write_text(CARGO_STUB_MISSING_TOOL, encoding="utf-8")
        cargo.chmod(0o755)
        (stub_dir / "cargo-calls").write_text("", encoding="utf-8")
        repo = fake_repo(stub_dir / "repo")

        result = run(
            {"PATH": f"{binaries}:/usr/bin:/bin", "STUB_DIR": str(stub_dir)},
            [],
            repo,
        )
        calls = (stub_dir / "cargo-calls").read_text(encoding="utf-8")
        report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"

        if result.returncode == 0:
            FAILURES.append(f"a missing cargo-mutants must not look like success:\n{report}")
        output = result.stdout + result.stderr
        if "cargo install cargo-mutants" not in output:
            FAILURES.append(f"the script did not say how to install the missing tool:\n{report}")
        if calls.strip():
            FAILURES.append(f"cargo was invoked before the tool check ran:\n{report}")


def with_stubbed_tool(
    survivors: list[str], repo: Path, args: list[str], env_extra: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    binaries = repo.parent / "bin"
    binaries.mkdir(exist_ok=True)
    stub = binaries / "cargo-mutants"
    stub.write_text("#!/usr/bin/env bash\necho placeholder\n", encoding="utf-8")
    stub.chmod(0o755)
    cargo = binaries / "cargo"
    cargo.write_text(cargo_mutants_stub(survivors), encoding="utf-8")
    cargo.chmod(0o755)
    environment = {"PATH": f"{binaries}:/usr/bin:/bin"}
    environment.update(env_extra or {})
    return run(environment, args, repo)


def test_no_baseline_yet_reports_survivors_and_the_seed_command() -> None:
    with tempfile.TemporaryDirectory() as directory:
        repo = fake_repo(Path(directory) / "repo")
        result = with_stubbed_tool(["src/a.rs:1: replace foo -> bool with true"], repo, [])
        report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"
        if result.returncode == 0:
            FAILURES.append(f"no baseline yet must not look like success:\n{report}")
        output = result.stdout + result.stderr
        if "MUTANTS_UPDATE_BASELINE=1" not in output:
            FAILURES.append(f"the script should say how to seed the baseline:\n{report}")


def test_update_baseline_writes_exactly_what_survived() -> None:
    with tempfile.TemporaryDirectory() as directory:
        repo = fake_repo(Path(directory) / "repo")
        survivors = [
            "src/b.rs:2: replace bar -> bool with false",
            "src/a.rs:1: replace foo -> bool with true",
        ]
        result = with_stubbed_tool(
            survivors, repo, [], env_extra={"MUTANTS_UPDATE_BASELINE": "1"}
        )
        report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"
        if result.returncode != 0:
            FAILURES.append(f"seeding the baseline should succeed:\n{report}")
        baseline = repo / "docs" / "mutants-baseline.txt"
        if not baseline.exists():
            FAILURES.append(f"the baseline file was not written:\n{report}")
            return
        recorded = sorted(baseline.read_text(encoding="utf-8").splitlines())
        if recorded != sorted(survivors):
            FAILURES.append(
                f"the baseline should be exactly what survived, sorted; got {recorded}:\n{report}"
            )


def test_a_new_survivor_past_the_baseline_fails_and_names_it() -> None:
    with tempfile.TemporaryDirectory() as directory:
        repo = fake_repo(Path(directory) / "repo")
        known = "src/a.rs:1: replace foo -> bool with true"
        (repo / "docs" / "mutants-baseline.txt").write_text(known + "\n", encoding="utf-8")
        new = "src/b.rs:2: replace bar -> bool with false"

        result = with_stubbed_tool([known, new], repo, [])
        report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"
        if result.returncode == 0:
            FAILURES.append(f"a new survivor past the baseline must fail the run:\n{report}")
        output = result.stdout + result.stderr
        if new not in output:
            FAILURES.append(f"the new survivor should be named:\n{report}")
        if known in output.replace(new, ""):
            FAILURES.append(
                f"a survivor already in the baseline is not new and should not be reported:\n{report}"
            )


def test_no_new_survivors_passes() -> None:
    with tempfile.TemporaryDirectory() as directory:
        repo = fake_repo(Path(directory) / "repo")
        known = "src/a.rs:1: replace foo -> bool with true"
        (repo / "docs" / "mutants-baseline.txt").write_text(known + "\n", encoding="utf-8")

        result = with_stubbed_tool([known], repo, [])
        report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"
        if result.returncode != 0:
            FAILURES.append(f"no new survivors past the baseline should pass:\n{report}")


def main() -> int:
    test_missing_tool_fails_before_touching_cargo()
    test_no_baseline_yet_reports_survivors_and_the_seed_command()
    test_update_baseline_writes_exactly_what_survived()
    test_a_new_survivor_past_the_baseline_fails_and_names_it()
    test_no_new_survivors_passes()

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("mutants.sh self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
