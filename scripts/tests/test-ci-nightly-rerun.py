#!/usr/bin/env python3
"""Self-test for scripts/ci-nightly-rerun.py.

While the last nightly is red, `ci.yml` re-runs on every pull request only
what was red on it. Both directions of a wrong answer cost something: too
wide and every pull request pays the 46-minute path this exists to remove;
too narrow and a pull request clears `Nightly is green` without proving the
thing that failed. The jobs payloads below are the shape the jobs API
returned for real red nightlies (a Docs-only run, a one-test GTK run).

Usage: scripts/tests/test-ci-nightly-rerun.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "ci-nightly-rerun.py"

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def job(name: str, conclusion: str, red_step: str | None = None) -> dict:
    steps = [{"name": "Set up job", "conclusion": "success"}]
    if red_step:
        steps.append({"name": red_step, "conclusion": "failure"})
    return {"name": name, "conclusion": conclusion, "steps": steps}


GREEN = [
    job("Anything to test", "success"),
    job("Coverage floors", "success"),
    job("Docs", "success"),
    job("Full suite", "success"),
]


def with_red(*red: dict) -> list[dict]:
    names = {j["name"] for j in red}
    return [j for j in GREEN if j["name"] not in names] + list(red)


SUITE_STEP = "cargo nextest run --profile ci-full"
DOCTEST_STEP = "cargo test --doc"
SPEC = "postio-gtk::focus_suite row_menu::a_right_click_offers_its_verbs"


def scope(jobs: list[dict], failed: str | None = None) -> dict[str, str]:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "jobs.json"
        path.write_text(json.dumps({"total_count": len(jobs), "jobs": jobs}), encoding="utf-8")
        args = [sys.executable, str(SCRIPT), "scope", "--jobs", str(path)]
        if failed is not None:
            tests = Path(directory) / "failed.txt"
            tests.write_text(failed, encoding="utf-8")
            args += ["--failed-tests", str(tests)]
        out = subprocess.run(args, check=True, capture_output=True, text=True).stdout
    # Read it back the way the runner does: `key=value`, or a heredoc.
    result: dict[str, str] = {}
    lines = iter(out.splitlines())
    for line in lines:
        if "<<" in line:
            key, delimiter = line.split("<<", 1)
            body = []
            for inner in lines:
                if inner == delimiter:
                    break
                body.append(inner)
            result[key] = "\n".join(body)
        else:
            key, _, value = line.partition("=")
            result[key] = value
    return result


METADATA = {
    "workspace_members": ["a", "b", "c"],
    "packages": [
        {
            "id": "a",
            "name": "postio-gtk",
            "targets": [
                {"name": "postio_gtk", "kind": ["lib"]},
                {"name": "focus_suite", "kind": ["test"]},
                {"name": "widgets_suite", "kind": ["test"]},
            ],
        },
        {
            "id": "b",
            "name": "postio-model",
            "targets": [{"name": "postio_model", "kind": ["lib"]}],
        },
        {
            "id": "c",
            "name": "postio-tools",
            "targets": [{"name": "postio-dump", "kind": ["bin"]}],
        },
    ],
}


def plan(command: str, specs: str) -> list[str]:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "metadata.json"
        path.write_text(json.dumps(METADATA), encoding="utf-8")
        out = subprocess.run(
            [sys.executable, str(SCRIPT), command, "--metadata", str(path)],
            input=specs,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    return out.splitlines()


def main() -> int:
    # ── one job red: re-run that job and nothing else ────────────────
    out = scope(with_red(job("Docs", "failure", "cargo doc")))
    case(
        "a red Docs job re-runs Docs alone (5 of 7 red nightlies measured)",
        out.get("only") == "doc",
        f"{out}",
    )
    out = scope(with_red(job("Coverage floors", "failure", "cargo llvm-cov (gated crates)")))
    case("a red coverage job re-runs coverage alone", out.get("only") == "coverage", f"{out}")

    # ── a red suite: the tests that failed, under the profile that ran them
    out = scope(with_red(job("Full suite", "failure", SUITE_STEP)), failed=SPEC + "\n")
    case(
        "a red suite with its failures known re-runs only those tests",
        out.get("only") == "tests" and out.get("tests") == SPEC,
        f"{out}",
    )
    case(
        "under ci-full, so a failed measurement-tier test can still be selected",
        out.get("profile") == "ci-full",
        f"{out}",
    )
    out = scope(
        with_red(job("Full suite", "failure", SUITE_STEP)),
        failed=SPEC + "\npostio-model codec::tests::round_trip\n",
    )
    case(
        "every failed test is carried, one per line",
        out.get("tests", "").splitlines() == [SPEC, "postio-model codec::tests::round_trip"],
        f"{out}",
    )

    # ── a red suite whose failures are unknown: the fallback ─────────
    for label, failed in (("no artifact", None), ("an empty artifact", "")):
        out = scope(with_red(job("Full suite", "failure", SUITE_STEP)), failed=failed)
        case(
            f"with {label}, the whole suite minus the measurement tier",
            out.get("only") == "tests" and out.get("profile") == "ci" and out.get("tests") == "",
            f"{out}",
        )

    out = scope(with_red(job("Full suite", "failure", DOCTEST_STEP)))
    case("red doctests re-run the doctests alone", out.get("only") == "doctests", f"{out}")

    out = scope(with_red(job("Full suite", "failure", "Install cargo-nextest")), failed=SPEC)
    case(
        "a setup step that broke re-runs the suite and the doctests, unnarrowed",
        out.get("only") == "tests doctests" and out.get("profile") == "ci" and out.get("tests") == "",
        f"{out}",
    )

    out = scope(with_red(job("Full suite", "cancelled")))
    case(
        "a suite job with no red step (timed out, cancelled) is unknown too",
        out.get("only") == "tests doctests" and out.get("profile") == "ci",
        f"{out}",
    )

    # ── two jobs red: both ───────────────────────────────────────────
    out = scope(
        with_red(job("Docs", "failure", "cargo doc"), job("Full suite", "failure", SUITE_STEP)),
        failed=SPEC,
    )
    case(
        "two red jobs re-run both",
        out.get("only") == "doc tests" and out.get("tests") == SPEC,
        f"{out}",
    )

    # ── nothing to narrow to: everything ─────────────────────────────
    everything = "coverage doc tests doctests"
    out = scope(with_red(job("Anything to test", "failure", "Compare against the last run")))
    case(
        "a red `Anything to test` (nothing else ran) re-runs everything",
        out.get("only") == everything and out.get("profile") == "ci",
        f"{out}",
    )
    out = scope(with_red(job("Some job added later", "failure", "x")))
    case("a red job this does not know re-runs everything", out.get("only") == everything, f"{out}")
    out = scope(GREEN)
    case(
        "a red run whose jobs are all green re-runs everything",
        out.get("only") == everything,
        f"{out}",
    )
    case("and the fallback never names tests", out.get("tests") == "", f"{out}")

    # ── building only what the failures live in ─────────────────────
    specs = "\n".join(
        [
            "postio-gtk::focus_suite row_menu::a",
            "postio-model codec::tests::b",
            "postio-tools::bin/postio-dump tests::c",
            "postio-gtk::gone_suite x::d",
            "postio-gone y::e",
        ]
    )
    args = plan("cargo-args", specs)
    case(
        "cargo args select each package and only the failing targets",
        args
        == [
            "-p",
            "postio-gtk",
            "-p",
            "postio-model",
            "-p",
            "postio-tools",
            "--test=focus_suite",
            "--lib",
            "--bin=postio-dump",
        ],
        f"{args}",
    )
    case(
        "a test binary gone from this branch builds nothing (it was deleted: fixed)",
        "--test=gone_suite" not in args and "postio-gone" not in args,
        f"{args}",
    )
    filt = plan("filter", specs)
    case(
        "the filter names each surviving test exactly, by its binary",
        filt
        == [
            "(binary_id(postio-gtk::focus_suite) & test(=row_menu::a))"
            " | (binary_id(postio-model) & test(=codec::tests::b))"
            " | (binary_id(postio-tools::bin/postio-dump) & test(=tests::c))"
        ],
        f"{filt}",
    )
    case(
        "when every failed binary is gone there is nothing to run",
        plan("filter", "postio-gone y::e\n") == [] and plan("cargo-args", "postio-gone y::e\n") == [],
        "expected empty output",
    )

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.")
        return 1
    print("ci-nightly-rerun self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
