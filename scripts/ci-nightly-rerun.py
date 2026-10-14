#!/usr/bin/env python3
"""What a pull request re-runs while the last nightly is red.

`ci.yml`'s `Nightly is green` blocks every merge while the last nightly on
`main` is red, and clears for a pull request that proves it fixes it. It
used to prove that by re-running the *whole* nightly -- the suite with its
measurement tier, the coverage floors and rustdoc -- on every pull request,
which put 45% of pull-request runs on a 46-minute path instead of a
6-minute one. Five of the seven red nightlies measured were the Docs job
alone (one broken intra-doc link); the other two were one GTK test each.
Re-running everything to clear one link is 40 minutes of nothing.

So the proof is narrowed to **what was red**, which is the same claim: the
nightly passes on this branch exactly when what failed on it now passes.

* A red `Docs` job re-runs `Docs`; a red `Coverage floors` re-runs that.
* A red `Full suite` re-runs only the tests that failed twice there --
  `test-with-flake-retry.sh` writes them to an artifact,
  `nightly-failed-tests`, one `<binary_id> <test name>` per line, the shape
  nextest's own summary uses -- and the doctests only if *they* failed.
* When the red set cannot be known -- the artifact is missing or empty, a
  setup step failed, `Anything to test` failed, a job this does not know --
  it falls back to every job, with the suite on `profile.ci`: the whole
  workspace minus the measurement tier. Never narrower than it can justify.

Subcommands:

  scope --jobs JOBS.json [--failed-tests FILE]
      JOBS.json is `GET /repos/{repo}/actions/runs/{id}/jobs` for the red
      run. Prints `only=`, `profile=` and a multi-line `tests` in
      `$GITHUB_OUTPUT` syntax. `only` is a space-separated subset of
      `coverage doc tests doctests`; `tests` is the failed specs, empty to
      mean the profile's whole reach.

  cargo-args [--metadata METADATA.json]
  filter     [--metadata METADATA.json]
      Read specs on stdin. `cargo-args` prints the cargo target selection
      that builds only their binaries, one argument per line; `filter`
      prints the nextest filterset that runs only those tests. Both drop a
      spec whose binary no longer exists on this branch (deleting a broken
      test is a way of fixing it), and print nothing when nothing is left.
      Without --metadata they ask `cargo metadata`.

Selecting the binaries matters as much as the filter: nextest refuses a
`binary_id()` that names no built binary, and building the whole workspace
to run one test is most of the cost being removed.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ALL = ["coverage", "doc", "tests", "doctests"]

# Job names in `nightly.yml`, which is what the jobs API reports. A job
# renamed there without moving here falls back to everything, which is slow
# rather than wrong.
JOB_SCOPE = {"Coverage floors": ["coverage"], "Docs": ["doc"]}
SUITE_JOB = "Full suite"
SUITE_STEP_PREFIX = "cargo nextest run"
DOCTEST_STEP_PREFIX = "cargo test --doc"

RED = {"failure", "timed_out", "cancelled", "startup_failure"}

LIB_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}


def decide(jobs: list[dict], failed_tests: list[str]) -> dict[str, object]:
    """The re-run plan for a red nightly whose jobs are `jobs`."""
    fallback = {"only": list(ALL), "profile": "ci", "tests": []}
    red = [job for job in jobs if job.get("conclusion") in RED]
    if not red:
        # The run is red and no job says why: a workflow that failed to
        # start, say. Nothing to narrow to.
        return fallback

    only: list[str] = []
    tests: list[str] = []
    profile = "ci-full"
    for job in red:
        name = job.get("name", "")
        if name in JOB_SCOPE:
            only += JOB_SCOPE[name]
            continue
        if name != SUITE_JOB:
            return fallback
        red_steps = [
            step.get("name", "")
            for step in job.get("steps") or []
            if step.get("conclusion") in RED
        ]
        for step in red_steps:
            if step.startswith(SUITE_STEP_PREFIX):
                only.append("tests")
                if failed_tests:
                    tests = list(failed_tests)
                else:
                    profile = "ci"
            elif step.startswith(DOCTEST_STEP_PREFIX):
                only.append("doctests")
            else:
                # Setup broke before the suite said anything.
                only += ["tests", "doctests"]
                profile, tests = "ci", []
        if not red_steps:
            only += ["tests", "doctests"]
            profile, tests = "ci", []

    if profile == "ci":
        tests = []
    return {
        "only": [scope for scope in ALL if scope in only],
        "profile": profile,
        "tests": tests,
    }


def parse_spec(line: str) -> tuple[str, str] | None:
    line = line.strip()
    if " " not in line:
        return None
    binary_id, test = line.split(" ", 1)
    return binary_id, test.strip()


def targets(metadata: dict) -> dict[str, dict[str, set[str]]]:
    """package -> kind -> target names, for the workspace's own packages."""
    members = set(metadata.get("workspace_members") or [])
    out: dict[str, dict[str, set[str]]] = {}
    for package in metadata.get("packages", []):
        if members and package.get("id") not in members:
            continue
        kinds: dict[str, set[str]] = {}
        for target in package.get("targets", []):
            for kind in target.get("kind", []):
                kind = "lib" if kind in LIB_KINDS else kind
                kinds.setdefault(kind, set()).add(target["name"])
        out[package["name"]] = kinds
    return out


def selection(binary_id: str, known: dict[str, dict[str, set[str]]]):
    """(package, cargo target flag or None for the lib), or None if gone."""
    package, _, rest = binary_id.partition("::")
    kinds = known.get(package)
    if kinds is None:
        return None
    if not rest:
        return (package, None) if "lib" in kinds else None
    kind, _, name = rest.partition("/")
    if not name:
        kind, name = "test", rest
    if name not in kinds.get(kind, set()):
        return None
    return package, f"--{kind}={name}"


def surviving(specs: list[str], metadata: dict):
    known = targets(metadata)
    kept = []
    for line in specs:
        parsed = parse_spec(line)
        if parsed is None:
            continue
        chosen = selection(parsed[0], known)
        if chosen is None:
            print(f"gone on this branch, counts as fixed: {line.strip()}", file=sys.stderr)
            continue
        kept.append((parsed, chosen))
    return kept


def cargo_args(specs: list[str], metadata: dict) -> list[str]:
    kept = surviving(specs, metadata)
    packages, flags = [], []
    for _, (package, flag) in kept:
        if package not in packages:
            packages.append(package)
        flag = flag or "--lib"
        if flag not in flags:
            flags.append(flag)
    # Target flags apply across every selected package and match where they
    # can, so `-p a -p b --lib --test=x` builds a's lib tests and b's x.
    return [arg for package in packages for arg in ("-p", package)] + flags


def nextest_filter(specs: list[str], metadata: dict) -> str:
    kept = surviving(specs, metadata)
    return " | ".join(
        f"(binary_id({binary_id}) & test(={test}))" for (binary_id, test), _ in kept
    )


def load_metadata(path: str | None) -> dict:
    if path:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def github_output(plan: dict[str, object]) -> str:
    tests = "\n".join(plan["tests"])  # type: ignore[arg-type]
    delimiter = "NIGHTLY_TESTS_EOF"
    return (
        f"only={' '.join(plan['only'])}\n"  # type: ignore[arg-type]
        f"profile={plan['profile']}\n"
        f"tests<<{delimiter}\n{tests}\n{delimiter}\n"
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    scope = sub.add_parser("scope")
    scope.add_argument("--jobs", required=True)
    scope.add_argument("--failed-tests")
    for name in ("cargo-args", "filter"):
        sub.add_parser(name).add_argument("--metadata")
    args = parser.parse_args(argv)

    if args.command == "scope":
        jobs = json.loads(Path(args.jobs).read_text(encoding="utf-8")).get("jobs", [])
        failed: list[str] = []
        if args.failed_tests and Path(args.failed_tests).is_file():
            text = Path(args.failed_tests).read_text(encoding="utf-8")
            failed = [line.strip() for line in text.splitlines() if parse_spec(line)]
        sys.stdout.write(github_output(decide(jobs, failed)))
        return 0

    specs = sys.stdin.read().splitlines()
    metadata = load_metadata(args.metadata)
    if args.command == "cargo-args":
        for arg in cargo_args(specs, metadata):
            print(arg)
    else:
        text = nextest_filter(specs, metadata)
        if text:
            print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
