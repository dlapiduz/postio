#!/usr/bin/env python3
"""Self-test for scripts/test-removal-check.sh.

A stub `cargo` on PATH stands in for cargo-mutants and replays a scenario:
the mutants Pass A (the suite without the cases) missed, and the ones
Pass B (with them) caught. So the verdict logic is judged without mutating
anything. One case per promise in
specs/012-test-suite-value/contracts/test-removal-check.md and research R4.

Usage: scripts/tests/test-test-removal-check.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE.parent / "test-removal-check.sh"
FAILURES: list[str] = []

CAUGHT_ONLY_WITH = "crates/postio-config/src/change.rs:71:13: replace || with && in ConfigChanged::any"
ALREADY_SURVIVES = "crates/postio-config/src/compose.rs:45:9: replace SignaturePlacement::as_str -> &'static str with \"\""
MISSED_EITHER_WAY = "crates/postio-config/src/change.rs:80:5: replace ConfigChanged::none -> bool with true"

# The stub records its arguments and writes the scenario's mutants.out for the pass it recognises:
# Pass A writes to `<work>/a` and Pass B to `<work>/b`.
STUB = r'''#!/usr/bin/env python3
import os, sys, pathlib
args = sys.argv[1:]
log = pathlib.Path(os.environ["STUB_LOG"])
log.write_text(log.read_text() + " ".join(args) + "\n" if log.exists() else " ".join(args) + "\n")
out = pathlib.Path(args[args.index("--output") + 1]) / "mutants.out"
out.mkdir(parents=True, exist_ok=True)
which = "B" if args[args.index("--output") + 1].endswith("/b") else "A"
for kind in ("missed", "caught"):
    lines = os.environ.get(f"STUB_{which}_{kind.upper()}", "")
    (out / f"{kind}.txt").write_text("".join(l + "\n" for l in lines.split("\n") if l))
'''


def run(tmp: Path, a_missed: list[str], b_caught: list[str], env_extra: dict[str, str] | None = None,
        scope: str | None = None, extra: list[str] | None = None):
    bin_dir = tmp / "bin"
    bin_dir.mkdir(exist_ok=True)
    stub = bin_dir / "cargo"
    stub.write_text(STUB)
    stub.chmod(0o755)
    baseline = tmp / "baseline.txt"
    baseline.write_text(ALREADY_SURVIVES + "\n")
    log = tmp / "cargo.log"
    log.unlink(missing_ok=True)
    env = {
        **os.environ,
        "PATH": f"{bin_dir}:{os.environ['PATH']}",
        "CI": "true",
        "STUB_LOG": str(log),
        "STUB_A_MISSED": "\n".join(a_missed),
        "STUB_B_CAUGHT": "\n".join(b_caught),
        "STUB_B_MISSED": "\n".join(m for m in a_missed if m not in b_caught),
        "MUTANTS_BASELINE": str(baseline),
        "TEST_AUDIT_OUT": str(tmp / "out"),
        **(env_extra or {}),
    }
    result = subprocess.run(
        ["bash", str(SCRIPT), *(["--scope", scope] if scope else []), *(extra or []), "crates/postio-config/src/change.rs", "--",
         "postio-config change::tests::changing_focus_touches_only_focus",
         "postio-config::config_suite schema::focus_filtering_is_on_unless_turned_off"],
        capture_output=True, text=True, env=env,
    )
    verdict_path = tmp / "out" / "verdict.json"
    verdict = json.loads(verdict_path.read_text()) if verdict_path.exists() else {}
    return result, verdict, log.read_text() if log.exists() else ""


def check(name: str, ok: bool, detail: str = "") -> None:
    if ok:
        print(f"ok  {name}")
    else:
        FAILURES.append(f"{name}\n{detail}")


with tempfile.TemporaryDirectory() as tmp:
    tmp = Path(tmp)

    result, verdict, calls = run(tmp, [CAUGHT_ONLY_WITH, ALREADY_SURVIVES, MISSED_EITHER_WAY], [CAUGHT_ONLY_WITH])
    out = result.stdout + result.stderr
    check("a mutant caught only with the cases is NOT SAFE, exit 1", result.returncode == 1 and "NOT SAFE" in out, out)
    check("the surviving mutant is named", CAUGHT_ONLY_WITH in out, out)
    check("the verdict records it", verdict.get("new_survivors") == [CAUGHT_ONLY_WITH] and verdict.get("result") == "not safe",
          json.dumps(verdict))
    check("the verdict names the cases and files",
          len(verdict.get("cases", [])) == 2 and verdict.get("files") == ["crates/postio-config/src/change.rs"], json.dumps(verdict))
    first = calls.splitlines()[0] if calls else ""
    check("Pass A runs the suite without the cases, through nextest",
          "--test-tool=nextest" in first and "-E" in first
          and "not ((binary_id(=postio-config) & test(=change::tests::changing_focus_touches_only_focus))" in first, calls)
    second = calls.splitlines()[1] if len(calls.splitlines()) > 1 else ""
    check("Pass B re-runs only A's misses that the baseline does not hold",
          "--re" in second and "ConfigChanged::any" in second and "ConfigChanged::none" in second
          and "SignaturePlacement" not in second and "-E" not in second, calls)

    result, verdict, calls = run(tmp, [ALREADY_SURVIVES, MISSED_EITHER_WAY], [])
    out = result.stdout + result.stderr
    check("no new survivor is safe, exit 0", result.returncode == 0 and out.startswith("safe:"), out)
    check("a safe verdict says so", verdict.get("result") == "safe" and verdict.get("new_survivors") == [], json.dumps(verdict))

    result, verdict, calls = run(tmp, [ALREADY_SURVIVES], [])
    check("nothing new missed skips Pass B", result.returncode == 0 and len(calls.splitlines()) == 1, calls)

    result, _, calls = run(tmp, [], [], {"CI": ""})
    out = result.stdout + result.stderr
    check("off CI it refuses, exit 2, and says why", result.returncode == 2 and "load average" in out and not calls, out)

    result, _, _ = run(tmp, [], [], {"CI": "", "POSTIO_ALLOW_MUTANTS_HERE": "1"})
    check("POSTIO_ALLOW_MUTANTS_HERE=1 lets it run off CI", result.returncode == 0, result.stdout + result.stderr)

    result, verdict, calls = run(tmp, [CAUGHT_ONLY_WITH], [CAUGHT_ONLY_WITH], scope="test(/^pickers::/)")
    lines = calls.splitlines()
    check("--scope narrows Pass A to the cluster, still without the cases",
          len(lines) == 2 and "(test(/^pickers::/)) & not (" in lines[0], calls)
    check("--scope narrows Pass B the same way, with the cases",
          len(lines) == 2 and "-E" in lines[1] and "test(/^pickers::/)" in lines[1], calls)
    check("the verdict records the scope", verdict.get("scope") == "test(/^pickers::/)", json.dumps(verdict))

    result, verdict, calls = run(tmp, [CAUGHT_ONLY_WITH], [CAUGHT_ONLY_WITH],
                                 extra=["--package", "postio-config", "--re", "ConfigChanged::any", "--re", "between"])
    lines = calls.splitlines()
    check("--package and --re narrow Pass A's mutants",
          len(lines) == 2 and "-p postio-config" in lines[0] and "--re ConfigChanged::any --re between" in lines[0], calls)
    check("--package reaches Pass B, whose own --re names the mutants",
          len(lines) == 2 and "-p postio-config" in lines[1] and "--re between" not in lines[1], calls)
    check("the verdict records the filters",
          verdict.get("package") == "postio-config" and verdict.get("re") == ["ConfigChanged::any", "between"], json.dumps(verdict))

    result, verdict, calls = run(tmp, [CAUGHT_ONLY_WITH], [CAUGHT_ONLY_WITH],
                                 extra=["--package", "postio-config", "--test-package", "postio-config",
                                        "--test-package", "postio-ffi"])
    lines = calls.splitlines()
    check("--test-package runs the removed cases' own crate in both passes",
          len(lines) == 2 and all("--test-package postio-config --test-package postio-ffi" in l for l in lines), calls)
    check("the verdict records the test packages", verdict.get("test_packages") == ["postio-config", "postio-ffi"],
          json.dumps(verdict))

    bad = subprocess.run(["bash", str(SCRIPT), "crates/x.rs"], capture_output=True, text=True, env={**os.environ, "CI": "true"})
    check("no -- and no cases is a usage error, exit 2", bad.returncode == 2, bad.stdout + bad.stderr)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("test-removal-check: all cases behaved")
