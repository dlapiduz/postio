#!/usr/bin/env python3
"""Self-test for scripts/test-coverage-sets.py's candidate computation.

Hand-written per-case region sets and a `cases.csv`, so no LLVM and no
cargo: the report half is judged on its own (`--regions`), the half that
collects profiles is proven on a real crate (quickstart §2). One case per
promise in specs/012-test-suite-value/contracts/test-coverage-sets.md and
research R3.

Usage: scripts/tests/test-test-coverage-sets.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import csv
import json
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE.parent / "test-coverage-sets.py"
FAILURES: list[str] = []

OWN = "crates/postio-config/src/change.rs"
FRONT = "crates/postio-ffi/src/bridge.rs"
OTHER = "crates/postio-model/src/lib.rs"
TESTS = "crates/postio-config/tests/config_suite/change.rs"

# id: (crate, layer, cost_rank, ignored, duration, refs, regions)
CASES = {
    "postio-config a_spec_cited": ("postio-config", "rule", 0, "false", "0.01", "012/FR-001", [f"{OWN}:1:1", f"{OWN}:2:1"]),
    # Each case also runs its own body, which no other case does; test code is not what a case protects.
    "postio-config b_copy": ("postio-config", "rule", 0, "false", "0.02", "", [f"{OWN}:1:1", f"{OWN}:2:1", f"{TESTS}:40:5"]),
    "postio-ffi c_through_the_bridge": ("postio-ffi", "frontend", 2, "false", "0.5", "", [f"{OWN}:1:1", f"{OWN}:2:1", f"{FRONT}:9:1"]),
    "postio-ffi d_one_line_of_it": ("postio-ffi", "frontend", 3, "false", "3.0", "", [f"{OWN}:1:1", f"{FRONT}:9:1"]),
    "postio-config e_live_server": ("postio-config", "rule", 0, "true", "unknown", "", [f"{OWN}:1:1"]),
    "postio-config m_measures": ("postio-config", "measurement", 0, "false", "40", "", [f"{OWN}:1:1"]),
    "postio-config z_tests_someone_else": ("postio-config", "rule", 0, "false", "0.1", "", [f"{OTHER}:5:1"]),
}


def write_inputs(tmp: Path) -> tuple[Path, Path]:
    cases = tmp / "cases.csv"
    with cases.open("w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["id", "crate", "suite", "file", "layer", "cost_rank", "display", "ignored", "duration_s", "refs", "smells"])
        for cid, (crate, layer, rank, ignored, dur, refs, _) in CASES.items():
            w.writerow([cid, crate, "lib", "x.rs", layer, rank, "false", ignored, dur, refs, ""])
    regions = tmp / "regions.json"
    regions.write_text(json.dumps({cid: c[6] for cid, c in CASES.items()}))
    return cases, regions


def run(tmp: Path, out: Path) -> tuple[subprocess.CompletedProcess, dict[str, dict]]:
    cases, regions = write_inputs(tmp)
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--cases", str(cases), "--regions", str(regions), "--out", str(out)],
        capture_output=True,
        text=True,
    )
    path = out / "candidates.csv"
    rows = {r["case"]: r for r in csv.DictReader(path.open())} if path.exists() else {}
    return result, rows


def check(name: str, ok: bool, detail: str = "") -> None:
    if ok:
        print(f"ok  {name}")
    else:
        FAILURES.append(f"{name}\n{detail}")


with tempfile.TemporaryDirectory() as tmp:
    tmp = Path(tmp)
    result, rows = run(tmp, tmp / "one")
    detail = result.stdout + result.stderr + json.dumps(rows, indent=1)
    check("exits 0 with candidates", result.returncode == 0, detail)
    check("of two equal cases, the one citing a spec is kept",
          rows.get("postio-config b_copy", {}).get("subsumed_by") == "postio-config a_spec_cited"
          and "postio-config a_spec_cited" not in rows, detail)
    check("a rule case is never subsumed by a frontend case (FR-006)",
          "postio-config a_spec_cited" not in rows, detail)
    check("a frontend case that adds its own regions is not a candidate",
          "postio-ffi c_through_the_bridge" not in rows, detail)
    check("a strict subset names a superset that costs no more",
          rows.get("postio-ffi d_one_line_of_it", {}).get("subsumed_by") == "postio-ffi c_through_the_bridge", detail)
    check("an ignored case is never listed", "postio-config e_live_server" not in rows, detail)
    check("a measurement case is never listed", "postio-config m_measures" not in rows, detail)
    check("a case with no region in its own crate says so",
          rows.get("postio-config z_tests_someone_else", {}).get("kind") == "no-own-crate-coverage", detail)
    check("subset rows carry their region count", rows.get("postio-config b_copy", {}).get("regions") == "2", detail)
    check("stdout names the next step", "scripts/test-removal-check.sh" in result.stdout, result.stdout)

    _, again = run(tmp, tmp / "two")
    first, second = tmp / "one" / "candidates.csv", tmp / "two" / "candidates.csv"
    check("two runs on the same input are byte-identical",
          first.exists() and second.exists() and first.read_bytes() == second.read_bytes())

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("test-coverage-sets: all cases behaved")
