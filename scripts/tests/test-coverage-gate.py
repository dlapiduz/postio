#!/usr/bin/env python3
"""Self-test for scripts/coverage.sh.

Mirrors scripts/tests/test-fuzz-missing-tool.py's shape for the same reason:
a tool this script wraps might not be installed, and the failure has to name
the tool and the fix before doing any real work, not surface as whatever
error `command not found` produces three directories down.

The second half is the logic that actually matters here and has never run
before this file: the floor comparison. A stubbed `cargo-llvm-cov` lets this
assert both directions -- a crate above its floor passes, one below fails and
says which crate and by how much -- without spending the minutes an
instrumented build actually costs.

Usage: scripts/tests/test-coverage-gate.py
Exit status: 0 the script behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
COVERAGE = HERE / "coverage.sh"

FAILURES: list[str] = []

CARGO_STUB_MISSING_TOOL = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/cargo-calls"
echo "error: no such command: \\`llvm-cov\\`" >&2
exit 101
"""


def summary_json(percent: float) -> str:
    return json.dumps(
        {
            "data": [
                {
                    "totals": {
                        "lines": {"count": 100, "covered": int(percent), "percent": percent}
                    }
                }
            ]
        }
    )


def cargo_llvm_cov_stub(percent: float) -> str:
    # Only the one subcommand this script calls needs to do anything; every
    # other invocation (there are none, today) would fall through and fail
    # loudly rather than silently pretending to succeed.
    return f"""#!/usr/bin/env bash
if [ "$1" = "llvm-cov" ]; then
    echo '{summary_json(percent)}'
    exit 0
fi
echo "unexpected cargo subcommand in stub: $*" >&2
exit 99
"""


def run(env_extra: dict[str, str], args: list[str]) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment.update(env_extra)
    return subprocess.run(
        ["bash", str(COVERAGE), *args],
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def test_missing_tool_fails_before_touching_cargo() -> None:
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = Path(directory)
        binaries = stub_dir / "bin"
        binaries.mkdir()
        cargo = binaries / "cargo"
        cargo.write_text(CARGO_STUB_MISSING_TOOL, encoding="utf-8")
        cargo.chmod(0o755)
        (stub_dir / "cargo-calls").write_text("", encoding="utf-8")

        # Deliberately without ~/.cargo/bin, where a real cargo-llvm-cov
        # would live -- `command -v cargo-llvm-cov` must find nothing here,
        # the same as it would find nothing on a machine that never
        # installed it.
        result = run(
            {"PATH": f"{binaries}:/usr/bin:/bin", "STUB_DIR": str(stub_dir)},
            [],
        )
        calls = (stub_dir / "cargo-calls").read_text(encoding="utf-8")
        report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"

        if result.returncode == 0:
            FAILURES.append(f"a missing cargo-llvm-cov must not look like success:\n{report}")
        output = result.stdout + result.stderr
        if "cargo install cargo-llvm-cov" not in output:
            FAILURES.append(
                f"the script did not say how to install the missing tool:\n{report}"
            )
        if calls.strip():
            FAILURES.append(
                f"cargo was invoked before the tool check ran:\n{report}"
            )


def with_stubbed_tool(percent: float, args: list[str]) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = Path(directory)
        binaries = stub_dir / "bin"
        binaries.mkdir()
        stub = binaries / "cargo-llvm-cov"
        stub.write_text("#!/usr/bin/env bash\necho placeholder\n", encoding="utf-8")
        stub.chmod(0o755)
        cargo = binaries / "cargo"
        cargo.write_text(cargo_llvm_cov_stub(percent), encoding="utf-8")
        cargo.chmod(0o755)
        return run({"PATH": f"{binaries}:/usr/bin:/bin"}, args)


def test_a_crate_above_its_floor_passes() -> None:
    # 99.9% rather than a value near postio-search's real floor: this test
    # asserts the comparison direction, not today's measured number, and
    # must keep passing as that number moves.
    result = with_stubbed_tool(99.9, ["postio-search"])
    report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"
    if result.returncode != 0:
        FAILURES.append(f"99.9% is above any real floor and must pass:\n{report}")
    if "ok:" not in result.stdout:
        FAILURES.append(f"a passing crate should be reported as ok:\n{report}")


def test_a_crate_below_its_floor_fails_and_names_it() -> None:
    result = with_stubbed_tool(1.0, ["postio-search"])
    report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"
    if result.returncode == 0:
        FAILURES.append(f"1.0% is below any real floor and must fail:\n{report}")
    output = result.stdout + result.stderr
    if "postio-search" not in output:
        FAILURES.append(f"a failure must name the crate that dropped:\n{report}")
    if "FAILED" not in output:
        FAILURES.append(f"a failure should say so plainly:\n{report}")


def test_an_unrecorded_crate_is_a_clear_error_not_a_crash() -> None:
    result = with_stubbed_tool(99.0, ["not-a-real-crate"])
    report = f"exit={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}"
    if result.returncode == 0:
        FAILURES.append(f"a crate with no recorded floor must not pass silently:\n{report}")
    if "Traceback" in result.stderr:
        FAILURES.append(
            f"a missing floor should be a clear message, not a Python traceback:\n{report}"
        )
    if "no floor recorded" not in result.stderr:
        FAILURES.append(f"the error should say a floor is missing:\n{report}")


def main() -> int:
    test_missing_tool_fails_before_touching_cargo()
    test_a_crate_above_its_floor_passes()
    test_a_crate_below_its_floor_fails_and_names_it()
    test_an_unrecorded_crate_is_a_clear_error_not_a_crash()

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("coverage.sh self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
