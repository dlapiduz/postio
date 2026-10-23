#!/usr/bin/env python3
"""One row per test case in the workspace: where it is, what it costs, what it cites.

The first of spec 012's three questions about the suite (specs/
012-test-suite-value, US1): *what is there?* It reads the runner's own list
of cases -- `cargo nextest list`, which includes the GTK and widgets suites'
harness-free cases a grep for `#[test]` misses -- and joins each to its
file, its layer (ADR 0049), its last recorded duration, the spec items and
issues it cites, and the smells research R6 names.

The summary groups cases by the spec item they cite and by the surface
their file is about (`pickers.rs`, `focus_pickers.rs`). One asserted in two
or more behaviour layers is a *parity cluster*: the shape ADR 0049 says to
reduce, and the order spec 012's cleanup batches take.

Nothing leaves the machine. Durations come from a nextest JUnit report --
by default the newest under `target/nextest/` -- and a case the report
does not mention is `unknown`, never 0.

    scripts/test-inventory.py                    # the whole workspace
    scripts/test-inventory.py -p postio-config   # one crate
    scripts/test-inventory.py --junit junit.xml  # durations from a CI artifact
    scripts/test-inventory.py $(scripts/unbuildable-crates.sh | sed 's/^/--exclude /')

Writes `target/test-audit/cases.csv` and `summary.json`.
Exit status: 0 written; 1 the rows disagree with the runner's count (a
parsing bug, never the tree's fault); 2 bad arguments or no runner.
"""

from __future__ import annotations

import argparse
import csv
import json
import subprocess
import sys
import xml.etree.ElementTree as ET
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))

import test_audit  # noqa: E402  -- enabled by the sys.path line above

REPO = Path(__file__).resolve().parent.parent
COLUMNS = ["id", "crate", "suite", "file", "layer", "cost_rank", "display", "ignored", "duration_s", "refs", "smells"]
LAYER_ORDER = ["rule", "controller", "frontend"]
BEHAVIOUR_LAYERS = set(LAYER_ORDER)  # meta and measurement assert no behaviour
CLUSTER_LAYERS = 2
FRONTEND_PREFIXES = ("focus_", "mac_", "gtk_", "tui_", "e2e_")
TOP = 20


def listing(args: argparse.Namespace) -> dict:
    if args.list_json:
        return json.loads(Path(args.list_json).read_text())
    command = ["cargo", "nextest", "list", "--message-format", "json"]
    command += [arg for p in args.package for arg in ("-p", p)] or ["--workspace"]
    if not args.package:
        command += [arg for c in args.exclude for arg in ("--exclude", c)]
    try:
        result = subprocess.run(command, cwd=args.root, capture_output=True, text=True)
    except FileNotFoundError:
        print("cargo is not on PATH.", file=sys.stderr)
        sys.exit(2)
    if result.returncode != 0:
        print(result.stderr, file=sys.stderr)
        print("`cargo nextest list` failed; the inventory needs a tree that builds.", file=sys.stderr)
        sys.exit(2)
    return json.loads(result.stdout)


def newest_junit(root: Path) -> Path | None:
    found = sorted((root / "target" / "nextest").glob("*/junit.xml"), key=lambda p: p.stat().st_mtime)
    return found[-1] if found else None


def durations(report: Path | None) -> dict[str, float]:
    """`<binary-id> <test name>` -> seconds, for every case the report ran."""
    if report is None:
        return {}
    times = {}
    for case in ET.parse(report).iter("testcase"):
        if case.find("skipped") is not None:
            continue
        times[f"{case.get('classname', '')} {case.get('name', '')}"] = float(case.get("time", "0"))
    return times


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("-p", "--package", action="append", default=[], help="restrict to this package (repeatable)")
    parser.add_argument(
        "--exclude", action="append", default=[], help="leave out a package this host cannot build (repeatable)"
    )
    parser.add_argument("--junit", help="a nextest JUnit report for durations (default: newest under target/nextest/)")
    parser.add_argument("--out", help="output directory (default: target/test-audit/)")
    parser.add_argument("--root", default=str(REPO), help=argparse.SUPPRESS)
    parser.add_argument("--list-json", help=argparse.SUPPRESS)  # a recorded listing, for the self-test
    args = parser.parse_args()
    root = Path(args.root).resolve()
    out = Path(args.out) if args.out else root / "target" / "test-audit"
    out.mkdir(parents=True, exist_ok=True)

    data = listing(args)
    cases = test_audit.parse_nextest_list(data, root)
    report = Path(args.junit) if args.junit else (None if args.list_json else newest_junit(root))
    times = durations(report)

    rows = []
    for case in cases:
        path = test_audit.case_file(case, root)
        body = test_audit.fn_body(path, case.fn_name)
        refs = test_audit.parse_refs(path, case.fn_name)
        layer, rank, display = test_audit.layer(case, path, body)
        rows.append(
            {
                "id": case.id,
                "crate": case.package,
                "suite": case.binary_name if case.kind == "test" else case.kind,
                "file": str(path.relative_to(root)) if isinstance(path, Path) and path.is_relative_to(root) else str(path),
                "layer": layer,
                "cost_rank": rank,
                "display": str(display).lower(),
                "ignored": str(case.ignored).lower(),
                "duration_s": times.get(case.id, "unknown"),
                "refs": ";".join(refs),
                "smells": ";".join(sorted(test_audit.smells(body, refs))),
            }
        )

    with (out / "cases.csv").open("w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=COLUMNS)
        writer.writeheader()
        writer.writerows(rows)

    summary = summarise(rows, root)
    (out / "clusters.json").write_text(json.dumps(summary.pop("_all_clusters"), indent=1) + "\n")
    (out / "summary.json").write_text(json.dumps(summary, indent=1, sort_keys=True) + "\n")
    report_to(summary, out, report)

    expected = data.get("test-count", len(cases))
    if len(rows) != expected:
        print(
            f"\nThe runner listed {expected} case(s) but {len(rows)} row(s) were written: a bug in"
            " scripts/lib/test_audit.py's parsing, not in the tree. Compare the listing with cases.csv.",
            file=sys.stderr,
        )
        return 1
    return 0


def surface_of(file: str) -> str | None:
    """The surface an integration test file is about: `focus_pickers.rs` and `pickers.rs` are one.

    Each layer cites its own task ids, so spec items rarely join a
    controller case to its frontend twins; the file names do.
    """
    if file == "ambiguous" or "/tests/" not in file:
        return None
    path = Path(file)
    stem = path.parent.name if path.stem == "mod" else path.stem
    if stem in ("main", "support", "lib") or stem.startswith("support_"):
        return None
    for prefix in FRONTEND_PREFIXES:
        if stem.startswith(prefix):
            return stem[len(prefix) :]
    return stem


def summarise(rows: list[dict], root: Path) -> dict:
    by_layer = Counter(r["layer"] for r in rows)
    cpu_by_layer: dict[str, float] = defaultdict(float)
    for r in rows:
        if r["duration_s"] != "unknown":
            cpu_by_layer[r["layer"]] += float(r["duration_s"])

    groups: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        if r["layer"] not in BEHAVIOUR_LAYERS:
            continue
        for ref in r["refs"].split(";"):
            if "/" in ref and not ref.startswith("?/"):
                groups[ref].append(r)
        surface = surface_of(r["file"])
        if surface:
            groups[f"surface:{surface}"].append(r)
    clusters = []
    for item, cited in groups.items():
        suites = {r["id"].split(" ", 1)[0] for r in cited}
        layers: dict[str, list[str]] = defaultdict(list)
        for r in cited:
            layers[r["layer"]].append(r["id"])
        if len(layers) >= CLUSTER_LAYERS:
            cpu = sum(float(r["duration_s"]) for r in cited if r["duration_s"] != "unknown")
            cheapest = min(layers, key=LAYER_ORDER.index)
            above = sum(len(ids) for name, ids in layers.items() if name != cheapest)
            clusters.append(
                {"item": item, "suites": len(suites), "cases_by_layer": dict(layers), "cpu_s": round(cpu, 6), "above_cheapest": above}
            )
    # The cases above a cluster's cheapest layer are the ones ADR 0049 would reduce.
    clusters.sort(key=lambda c: (-c["cpu_s"], -c["above_cheapest"], c["item"]))

    smells = Counter(s for r in rows for s in r["smells"].split(";") if s)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=root, capture_output=True, text=True)
    return {
        "cases": len(rows),
        "cases_by_layer": dict(by_layer),
        "cpu_s_by_layer": {k: round(v, 3) for k, v in cpu_by_layer.items()},
        "parity_clusters": {"count": len(clusters), "top": clusters[:TOP]},
        "_all_clusters": clusters,
        "smells_by_kind": dict(smells),
        "untraced": smells.get("untraced", 0),
        "ambiguous": sum(1 for r in rows if r["file"] == "ambiguous"),
        "commit": commit.stdout.strip(),
    }


def report_to(summary: dict, out: Path, junit: Path | None) -> None:
    print(f"{summary['cases']} cases at {summary['commit'] or 'an unknown commit'}; written to {out}/cases.csv")
    print()
    print("By layer (cases, CPU-seconds from " + (str(junit) if junit else "no JUnit report: pass --junit") + "):")
    for layer, n in sorted(summary["cases_by_layer"].items(), key=lambda kv: -kv[1]):
        print(f"  {layer:<12} {n:>6}  {summary['cpu_s_by_layer'].get(layer, 0):>10.1f} s")
    print()
    clusters = summary["parity_clusters"]
    print(f"Parity clusters (a spec item or surface in >= {CLUSTER_LAYERS} behaviour layers): {clusters['count']}")
    for c in clusters["top"][:10]:
        layers = ", ".join(f"{k} {len(v)}" for k, v in sorted(c["cases_by_layer"].items()))
        print(f"  {c['item']:<28} {c['suites']} suites  {c['cpu_s']:>8.1f} s  ({layers})")
    print(f"  All of them, largest first: {out}/clusters.json")
    print("  ADR 0049: each keeps its branches in one layer. Read one with:")
    print(f"    grep '<item>' {out}/cases.csv")
    print()
    print("Smells: " + (", ".join(f"{k} {v}" for k, v in sorted(summary["smells_by_kind"].items())) or "none"))
    print(f"Files not resolved: {summary['ambiguous']} (the `file` column says `ambiguous`)")
    print()
    print("Next: which of these run nothing another case does not --")
    print("    scripts/test-coverage-sets.py -p <crate>")


if __name__ == "__main__":
    sys.exit(main())
