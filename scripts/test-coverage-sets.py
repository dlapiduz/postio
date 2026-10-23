#!/usr/bin/env python3
"""Which test cases run nothing that a case at least as cheap does not.

Spec 012's second question (specs/012-test-suite-value, US2). Each case is
run alone under coverage instrumentation, and what it executed becomes a
set of regions. A case is a *candidate* when its set is a subset of another
case's that costs no more to run (research R3), or when it executes nothing
in the crate it belongs to.

A candidate is a proposal, never a verdict. Two cases that run the same
lines can assert different things. Whether one can go is
`scripts/test-removal-check.sh`'s question, and only its "safe" licenses a
removal (FR-007).

How each case gets its own profile: nextest runs every case in its own
process and tells it its name (`NEXTEST_BINARY_ID`, `NEXTEST_TEST_NAME`).
The `per-test-coverage` profile in `.config/nextest.toml` wraps each one in
`scripts/per-test-profile.sh`, which points `LLVM_PROFILE_FILE` at a
directory of that case's own (research R2). The instrumentation flags come
from `cargo llvm-cov show-env`, so this measures what
`scripts/coverage.sh` measures. Everything stays on this machine.

    scripts/test-inventory.py -p postio-config        # first: the cases and their costs
    scripts/test-coverage-sets.py -p postio-config
    scripts/test-coverage-sets.py -p postio-config --reuse   # re-report without re-running

Writes `target/test-audit/candidates.csv`.
Exit status: 0 report written (with or without candidates); 1 a case left
no profile; 2 bad arguments or a missing tool.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
NOT_JUDGED = {"measurement"}


# --- the report: pure, and what the self-test judges ----------------------


def load_cases(path: Path) -> dict[str, dict]:
    return {row["id"]: row for row in csv.DictReader(path.open())}


def judged(row: dict) -> bool:
    return row["ignored"] != "true" and row["layer"] not in NOT_JUDGED


def keeper_order(row: dict) -> tuple:
    """Cheapest first, then the case that cites a spec item, then by name: deterministic."""
    cites_spec = any("/" in ref for ref in row["refs"].split(";") if ref)
    return (int(row["cost_rank"]), not cites_spec, row["id"])


def candidates(cases: dict[str, dict], regions: dict[str, list[str]]) -> list[dict]:
    ids = sorted(cid for cid in regions if cid in cases and judged(cases[cid]))
    sets = {cid: frozenset(regions[cid]) for cid in ids}
    found = []
    for cid in ids:
        row = cases[cid]
        own = f"crates/{row['crate']}/"
        if not any(region.startswith(own) for region in sets[cid]):
            found.append({"case": cid, "kind": "no-own-crate-coverage", "subsumed_by": "", "regions": len(sets[cid])})
            continue
        supersets = [
            other
            for other in ids
            if other != cid
            and int(cases[other]["cost_rank"]) <= int(row["cost_rank"])
            and sets[cid] <= sets[other]
            # Of two equal sets only one goes; the keeper is the first in keeper_order.
            and not (sets[cid] == sets[other] and keeper_order(row) < keeper_order(cases[other]))
        ]
        if supersets:
            best = min(supersets, key=lambda other: keeper_order(cases[other]))
            found.append({"case": cid, "kind": "subset", "subsumed_by": best, "regions": len(sets[cid])})

    def cpu(cid: str) -> float:
        d = cases[cid]["duration_s"]
        return float(d) if d not in ("", "unknown") else 0.0

    found.sort(key=lambda c: (c["kind"] != "subset", -cpu(c["case"]), c["case"]))
    return found


def write_report(found: list[dict], cases: dict[str, dict], out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)
    with (out / "candidates.csv").open("w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=["case", "kind", "subsumed_by", "regions"], lineterminator="\n")
        writer.writeheader()
        writer.writerows(found)
    subsets = [c for c in found if c["kind"] == "subset"]
    print(f"{len(subsets)} subset candidate(s), {len(found) - len(subsets)} with no coverage of their own crate;")
    print(f"written to {out}/candidates.csv. These are proposals, not verdicts (FR-007).")
    if found:
        files = sorted({cases[c["case"]]["file"] for c in found if cases[c["case"]]["file"] != "ambiguous"})
        print()
        print("Next: is removing them safe? On CI (it mutates code; see the script's header):")
        print(f"    scripts/test-removal-check.sh <the source files they cover> -- {found[0]['case']!r} ...")
        print(f"  ({len(files)} test file(s) hold these candidates)")
    else:
        print("Next: nothing to check -- scripts/test-removal-check.sh has no candidates from this crate.")


# --- the run: per-case profiles -------------------------------------------


def tool(name: str) -> Path:
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True).stdout.strip()
    found = sorted(Path(sysroot).glob(f"lib/rustlib/*/bin/{name}"))
    if not found:
        print(f"{name} is not installed, so per-case coverage cannot be read.\n", file=sys.stderr)
        print("    rustup component add llvm-tools-preview", file=sys.stderr)
        print("    cargo install cargo-llvm-cov --locked", file=sys.stderr)
        sys.exit(2)
    return found[0]


def coverage_env() -> dict[str, str]:
    if shutil.which("cargo-llvm-cov") is None:
        print("cargo-llvm-cov is not installed, so coverage cannot be measured.\n", file=sys.stderr)
        print("    rustup component add llvm-tools-preview", file=sys.stderr)
        print("    cargo install cargo-llvm-cov --locked", file=sys.stderr)
        sys.exit(2)
    shown = subprocess.run(
        ["cargo", "llvm-cov", "show-env", "--export-prefix"], cwd=REPO, capture_output=True, text=True, check=True
    ).stdout
    env = dict(os.environ)
    for line in shown.splitlines():
        if line.startswith("export "):
            key, _, value = line[len("export ") :].partition("=")
            env[key] = shlex.split(value)[0] if value else ""
    return env


def run_cases(packages: list[str], env: dict[str, str], profiles: Path) -> dict[str, Path]:
    """Run every case under the per-test-coverage profile; binary id -> its test binary."""
    shutil.rmtree(profiles, ignore_errors=True)
    package_args = [arg for p in packages for arg in ("-p", p)]
    listed = subprocess.run(
        ["cargo", "nextest", "list", "--message-format", "json", *package_args],
        cwd=REPO, env=env, capture_output=True, text=True, check=True,
    )
    binaries = {s["binary-id"]: Path(s["binary-path"]) for s in json.loads(listed.stdout)["rust-suites"].values()}
    # A failing case still leaves a profile, and its coverage is still what it ran.
    subprocess.run(
        ["cargo", "nextest", "run", "--profile", "per-test-coverage", "--no-fail-fast", *package_args],
        cwd=REPO, env=env,
    )
    return binaries


def case_dir(profiles: Path, binary_id: str, name: str) -> Path:
    """Where scripts/per-test-profile.sh put a case's profiles: keep the two in step."""
    return profiles / binary_id / hashlib.sha1(name.encode()).hexdigest()[:16]


def regions_of(case_profiles: Path, binary: Path, profdata: Path, llvm_cov: Path) -> list[str]:
    raws = sorted(case_profiles.glob("*.profraw"))
    merged = case_profiles / "case.profdata"
    subprocess.run([profdata, "merge", "-sparse", *raws, "-o", merged], check=True)
    exported = subprocess.run(
        [llvm_cov, "export", "--format=text", "--skip-functions", f"--instr-profile={merged}", binary],
        capture_output=True, text=True, check=True,
    ).stdout
    found = set()
    for f in json.loads(exported)["data"][0]["files"]:
        try:
            name = str(Path(f["filename"]).resolve().relative_to(REPO))
        except ValueError:
            continue  # the registry, the standard library: not ours to judge
        if not name.startswith("crates/"):
            continue
        for line, col, count, has_count, is_entry, *_ in f["segments"]:
            if has_count and is_entry and count > 0:
                found.add(f"{name}:{line}:{col}")
    return sorted(found)


def collect(cases: dict[str, dict], packages: list[str], profiles: Path, reuse: bool) -> dict[str, list[str]]:
    env = coverage_env()
    binaries = run_cases(packages, env, profiles) if not reuse else None
    if binaries is None:
        listed = subprocess.run(
            ["cargo", "nextest", "list", "--message-format", "json", *[a for p in packages for a in ("-p", p)]],
            cwd=REPO, env=env, capture_output=True, text=True, check=True,
        )
        binaries = {s["binary-id"]: Path(s["binary-path"]) for s in json.loads(listed.stdout)["rust-suites"].values()}
    profdata, llvm_cov = tool("llvm-profdata"), tool("llvm-cov")
    regions, missing = {}, []
    for cid, row in sorted(cases.items()):
        if not judged(row) or row["crate"] not in packages:
            continue
        binary_id, name = cid.split(" ", 1)
        where = case_dir(profiles, binary_id, name)
        if not any(where.glob("*.profraw")):
            missing.append(cid)
            continue
        regions[cid] = regions_of(where, binaries[binary_id], profdata, llvm_cov)
    if missing:
        print(f"{len(missing)} case(s) left no profile, first {missing[0]!r}.", file=sys.stderr)
        print("The wrapper did not run for them: see research R2 and the per-test-coverage profile.", file=sys.stderr)
        sys.exit(1)
    return regions


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("-p", "--package", action="append", default=[], help="a package to measure (repeatable)")
    parser.add_argument("--cases", help="the inventory's cases.csv (default: target/test-audit/cases.csv)")
    parser.add_argument("--out", help="output directory (default: target/test-audit/)")
    parser.add_argument("--reuse", action="store_true", help="report from the profiles already collected")
    parser.add_argument("--regions", help=argparse.SUPPRESS)  # recorded region sets, for the self-test
    args = parser.parse_args()
    out = Path(args.out) if args.out else REPO / "target" / "test-audit"
    cases_path = Path(args.cases) if args.cases else REPO / "target" / "test-audit" / "cases.csv"
    if not cases_path.exists():
        print(f"No {cases_path}: run the inventory for the same packages first --", file=sys.stderr)
        print("    scripts/test-inventory.py " + " ".join(f"-p {p}" for p in args.package), file=sys.stderr)
        return 2
    cases = load_cases(cases_path)
    if args.regions:
        regions = json.loads(Path(args.regions).read_text())
    elif not args.package:
        print("Name the packages to measure with -p: per-case coverage of the whole workspace is a CI job.", file=sys.stderr)
        return 2
    else:
        regions = collect(cases, args.package, REPO / "target" / "test-audit" / "profiles", args.reuse)
        out.mkdir(parents=True, exist_ok=True)
        (out / "regions.json").write_text(json.dumps(regions, sort_keys=True))
    write_report(candidates(cases, regions), cases, out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
