#!/usr/bin/env python3
"""Self-test for scripts/report-advisory-failure.sh.

The scheduled advisories audit (audit.yml) is only worth having if a failure
actually leaves a trace someone will see without watching the Actions tab.
This exercises the two shapes that trace can take -- a fresh issue, naming
the advisory in its title, and a comment added to one already open, rather
than a second issue piling up for the same advisory every day it stays
unfixed -- plus the case where cargo-deny's output does not contain a
RUSTSEC id at all (a licence or ban failure would not, if this script is
ever pointed at a wider `cargo deny check` in the future).

`gh` is stubbed on PATH: `issue list` replays canned JSON, and `issue
create`/`issue comment` are recorded to a log file instead of actually
calling GitHub, so this runs with no token and no network.

Usage: scripts/tests/test-report-advisory-failure.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "report-advisory-failure.sh"

FAILURES: list[str] = []

# Records every call it was given, then answers `issue list` from a fixture
# and no-ops everything else -- which is all `create`/`comment` need, since
# the test reads back the call log rather than anything `gh` would return.
GH_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/existing.json"
fi
exit 0
"""


def run(*, output: str, existing_json: str) -> tuple[subprocess.CompletedProcess, Path]:
    tmp = tempfile.TemporaryDirectory()
    stub_dir = Path(tmp.name)
    bin_dir = stub_dir / "bin"
    bin_dir.mkdir()
    gh = bin_dir / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub_dir / "existing.json").write_text(existing_json, encoding="utf-8")
    (stub_dir / "calls").write_text("", encoding="utf-8")

    output_file = stub_dir / "audit-output.txt"
    output_file.write_text(output, encoding="utf-8")

    env = dict(os.environ)
    env["PATH"] = f"{bin_dir}:{env['PATH']}"
    env["STUB_DIR"] = str(stub_dir)

    proc = subprocess.run(
        ["bash", str(SCRIPT), str(output_file), "https://example.com/run/1"],
        capture_output=True,
        text=True,
        env=env,
        stdin=subprocess.DEVNULL,
        timeout=30,
    )
    proc._tmp = tmp  # keep the tempdir alive until the caller is done with it
    return proc, stub_dir / "calls"


def calls(calls_path: Path) -> list[str]:
    if not calls_path.exists():
        return []
    return [line for line in calls_path.read_text(encoding="utf-8").splitlines() if line]


def case_files_a_fresh_issue_naming_the_advisory() -> None:
    label = "a fresh advisory files a new issue naming it"
    proc, calls_path = run(
        output=(
            "error[vulnerability]: potential problem\n"
            "  ID:       RUSTSEC-2024-0421\n"
            "  Advisory: https://rustsec.org/advisories/RUSTSEC-2024-0421\n"
        ),
        existing_json="[]",
    )
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    logged = calls(calls_path)
    created = [line for line in logged if line.startswith("issue create")]
    if len(created) != 1:
        FAILURES.append(f"{label}: expected one 'issue create' call, got {logged}")
        return
    if "RUSTSEC-2024-0421" not in created[0]:
        FAILURES.append(f"{label}: the created issue's title did not name the advisory: {created[0]}")
    if any(line.startswith("issue comment") for line in logged):
        FAILURES.append(f"{label}: commented as well as creating -- should be one or the other")


def case_an_open_issue_for_the_same_advisory_gets_a_comment_not_a_duplicate() -> None:
    label = "an already-open issue is commented on, not duplicated"
    proc, calls_path = run(
        output="ID:       RUSTSEC-2024-0421\n",
        existing_json='[{"number": 42, "title": "cargo-deny: RUSTSEC-2024-0421 failed the scheduled audit"}]',
    )
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    logged = calls(calls_path)
    if any(line.startswith("issue create") for line in logged):
        FAILURES.append(f"{label}: filed a new issue instead of reusing #42: {logged}")
    commented = [line for line in logged if line.startswith("issue comment")]
    if len(commented) != 1 or not commented[0].startswith("issue comment 42"):
        FAILURES.append(f"{label}: expected exactly one comment on #42, got {logged}")


def case_an_open_issue_for_a_different_advisory_does_not_absorb_this_one() -> None:
    label = "a different advisory's open issue is left alone"
    proc, calls_path = run(
        output="ID:       RUSTSEC-2024-0421\n",
        existing_json='[{"number": 7, "title": "cargo-deny: RUSTSEC-2019-0001 failed the scheduled audit"}]',
    )
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    logged = calls(calls_path)
    created = [line for line in logged if line.startswith("issue create")]
    if len(created) != 1:
        FAILURES.append(f"{label}: expected a new issue for the new advisory, got {logged}")
    if any(line.startswith("issue comment") for line in logged):
        FAILURES.append(f"{label}: commented on the unrelated issue #7 instead of filing a new one: {logged}")


def case_no_advisory_id_still_files_a_generic_issue() -> None:
    label = "output with no RUSTSEC id still gets reported"
    proc, calls_path = run(
        output="error[license-not-encountered]: a licence rule failed\n",
        existing_json="[]",
    )
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    logged = calls(calls_path)
    created = [line for line in logged if line.startswith("issue create")]
    if len(created) != 1:
        FAILURES.append(f"{label}: expected exactly one issue filed, got {logged}")
        return
    if "RUSTSEC" in created[0]:
        FAILURES.append(f"{label}: claimed an advisory id that was never in the output: {created[0]}")


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing: {SCRIPT}", file=sys.stderr)
        return 1
    case_files_a_fresh_issue_naming_the_advisory()
    case_an_open_issue_for_the_same_advisory_gets_a_comment_not_a_duplicate()
    case_an_open_issue_for_a_different_advisory_does_not_absorb_this_one()
    case_no_advisory_id_still_files_a_generic_issue()

    if FAILURES:
        for failure in FAILURES:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("report-advisory-failure self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
