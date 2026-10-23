#!/usr/bin/env python3
"""Self-test for scripts/test-inventory.py.

Replays the fixture tree's recorded `nextest list` and JUnit report
(scripts/tests/fixtures/test-audit/), so no cargo runs. One case per
promise in specs/012-test-suite-value/contracts/test-inventory.md.

Usage: scripts/tests/test-test-inventory.py
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
SCRIPT = HERE.parent / "test-inventory.py"
FIXTURE = HERE / "fixtures" / "test-audit"
LISTING = FIXTURE / "nextest-list.json"
FAILURES: list[str] = []

COLUMNS = ["id", "crate", "suite", "file", "layer", "cost_rank", "display", "ignored", "duration_s", "refs", "smells"]


def run(out: Path, listing: Path = LISTING, junit: Path | None = FIXTURE / "junit.xml") -> subprocess.CompletedProcess:
    args = [sys.executable, str(SCRIPT), "--root", str(FIXTURE), "--list-json", str(listing), "--out", str(out)]
    if junit is not None:
        args += ["--junit", str(junit)]
    return subprocess.run(args, capture_output=True, text=True)


def check(name: str, ok: bool, detail: str = "") -> None:
    if ok:
        print(f"ok  {name}")
    else:
        FAILURES.append(f"{name}\n{detail}")


with tempfile.TemporaryDirectory() as tmp:
    out = Path(tmp)
    result = run(out)
    check("exits 0 on a consistent listing", result.returncode == 0, result.stdout + result.stderr)
    rows = list(csv.DictReader((out / "cases.csv").open())) if (out / "cases.csv").exists() else []
    check("one row per listed case", len(rows) == 21, f"{len(rows)} rows")
    header = (out / "cases.csv").read_text().splitlines()[0].split(",") if rows else []
    check("columns are data-model Case, in order", header == COLUMNS, str(header))
    by_id = {r["id"]: r for r in rows}

    gtk = by_id.get("postio-gtk::focus_suite pickers::s_then_2_snoozes", {})
    check("a timed case carries its duration", gtk.get("duration_s") == "3.5", str(gtk))
    check("a frontend display case is rank 3", gtk.get("layer") == "frontend" and gtk.get("cost_rank") == "3", str(gtk))
    check("refs are ;-joined", gtk.get("refs") == "009/FR-021", str(gtk))
    untimed = by_id.get("postio-ffi::ffi_suite focus_pickers::opening_succeeds", {})
    check("an untimed case is unknown, not 0", untimed.get("duration_s") == "unknown", str(untimed))
    check("smells are recorded", untimed.get("smells") == "is-ok-only", str(untimed))
    check("an unresolvable case's file is ambiguous",
          by_id.get("postio-storage generated::dup", {}).get("file") == "ambiguous")
    check("files are relative to the root",
          untimed.get("file") == "crates/postio-ffi/tests/ffi_suite/focus_pickers.rs", str(untimed))

    summary = json.loads((out / "summary.json").read_text()) if (out / "summary.json").exists() else {}
    check("summary counts the cases", summary.get("cases") == 21, str(summary.get("cases")))
    clusters = {c["item"]: c for c in summary.get("parity_clusters", {}).get("top", [])}
    fr021 = clusters.get("009/FR-021", {})
    check("an item cited in two behaviour layers is a parity cluster",
          fr021.get("suites") == 3 and sorted(fr021.get("cases_by_layer", {})) == ["controller", "frontend"], str(clusters))
    pickers = clusters.get("surface:pickers", {})
    check("a surface tested in the controller and the frontends is a parity cluster",
          pickers.get("suites") == 3 and sorted(pickers.get("cases_by_layer", {})) == ["controller", "frontend"], str(clusters))
    check("a frontend prefix does not hide a surface",
          "postio-ffi::ffi_suite focus_pickers::opening_succeeds" in pickers.get("cases_by_layer", {}).get("frontend", []),
          str(pickers))
    check("an item cited in one layer only is not a cluster", "007/SC-002" not in clusters, str(clusters))
    check("meta and measurement do not count as layers",
          not any(k.startswith("surface:docs") or k.startswith("surface:slow") for k in clusters), str(clusters))
    check("a cluster sums its known CPU", abs(fr021.get("cpu_s", 0) - 3.91) < 1e-9, str(fr021))
    check("the summary counts smells", summary.get("smells_by_kind", {}).get("is-ok-only") == 2,
          str(summary.get("smells_by_kind")))
    check("the summary counts ambiguous files", summary.get("ambiguous") == 1, str(summary.get("ambiguous")))
    check("stdout ends with what to run next", "scripts/test-coverage-sets.py" in result.stdout, result.stdout)

    result = run(Path(tmp) / "nojunit", junit=None)
    rows = list(csv.DictReader((Path(tmp) / "nojunit" / "cases.csv").open())) if result.returncode == 0 else []
    check("with no JUnit report every duration is unknown",
          rows and all(r["duration_s"] == "unknown" for r in rows), result.stdout + result.stderr)

    broken = Path(tmp) / "broken.json"
    data = json.loads(LISTING.read_text())
    data["test-count"] = 22
    broken.write_text(json.dumps(data))
    result = run(Path(tmp) / "broken", listing=broken)
    check("a row count that disagrees with the runner exits 1 and names both numbers",
          result.returncode == 1 and "22" in result.stdout + result.stderr and "21" in result.stdout + result.stderr,
          f"exit {result.returncode}\n{result.stdout}{result.stderr}")

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("test-inventory: all cases behaved")
