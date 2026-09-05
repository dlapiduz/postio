#!/usr/bin/env python3
"""Self-test for scripts/ci-changes.sh, which decides what a diff obliges CI
to build (#1127).

39% of the last 14 days' commits on main touched no crate, manifest, cargo
config, toolchain, nextest config, fuzz target or build-affecting script --
and unless they were pure prose, CI ran the whole workspace suite for them:
two tooling PRs waited ~20 minutes each for tests they could not affect.

The classifier answers three questions per changed-file list, `rust=`,
`docs=` and `macos=`, as `key=value` lines for $GITHUB_OUTPUT. The third is
#666's: a macOS runner is the only thing that compiles the Swift half and
proves the link, and it must run when either half of that changes -- a
`postio-ffi` edit regenerates the bindings the Swift compiles against. Both directions matter:
`yes` for a docs change costs twenty minutes of runner on the critical
path; `no` for a Rust change merges code nothing compiled. So it fails
safe: anything it cannot place is `yes`, an empty list is `yes`, and any
event that is not a pull request or a push with a known range is `yes`.

Usage: scripts/tests/test-ci-changes.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "ci-changes.sh"
FAILURES: list[str] = []


def classify(event: str, files: list[str]) -> dict[str, str]:
    result = subprocess.run(
        ["bash", str(SCRIPT), event],
        input="\n".join(files) + ("\n" if files else ""),
        capture_output=True, text=True, timeout=30,
    )
    if result.returncode != 0:
        FAILURES.append(f"exit {result.returncode} for {event} {files}: {result.stderr}")
        return {}
    return dict(line.split("=", 1) for line in result.stdout.split() if "=" in line)


def case(
    name: str, event: str, files: list[str], rust: str, docs: str, macos: str
) -> None:
    got = classify(event, files)
    ok = (
        got.get("rust") == rust
        and got.get("docs") == docs
        and got.get("macos") == macos
    )
    print(f"{'ok   ' if ok else 'FAIL '} {name}")
    if not ok:
        FAILURES.append(
            f"{name}: want rust={rust} docs={docs} macos={macos}, got {got}"
        )


def main() -> int:
    # Anything Rust-shaped builds.
    case("a crate source file", "pull_request", ["crates/postio-core/src/lib.rs"], "yes", "no", "yes")
    case("a crate's Cargo.toml", "pull_request", ["crates/postio-body/Cargo.toml"], "yes", "no", "yes")
    case("the root manifest", "pull_request", ["Cargo.toml"], "yes", "no", "yes")
    case("Cargo.lock alone", "pull_request", ["Cargo.lock"], "yes", "no", "yes")
    case("cargo config", "pull_request", [".cargo/config.toml"], "yes", "no", "yes")
    case("the toolchain pin", "pull_request", ["rust-toolchain.toml"], "yes", "no", "yes")
    case("nextest config", "pull_request", [".config/nextest.toml"], "yes", "no", "yes")
    case("a fuzz target", "pull_request", ["fuzz/fuzz_targets/parse.rs"], "yes", "no", "yes")
    case("cargo-deny policy", "pull_request", ["deny.toml"], "yes", "no", "yes")
    case("the CI workflow itself", "pull_request", [".github/workflows/ci.yml"], "yes", "yes", "yes")
    case("the shared CI action", "pull_request", [".github/actions/rust-workspace/action.yml"], "yes", "no", "yes")
    for script in ("linker.sh", "cc-wrapper.sh", "rustc-wrapper.sh", "headless-runner.sh",
                   "install-shims.sh", "install-nextest.sh", "jobserver.sh",
                   "ci-drop-workspace-artifacts.sh", "lib/drop-workspace-artifacts.sh"):
        case(f"a build-affecting script: {script}", "pull_request", [f"scripts/{script}"], "yes", "no", "yes")
    # A test fixture is Rust-shaped too: the corpus is read by tests.
    case("a corpus fixture", "pull_request", ["crates/postio-model/tests/corpus/x.eml"], "yes", "no", "yes")

    # Nothing Rust-shaped does not.
    case("a tooling script", "pull_request", ["scripts/issue-land.sh"], "no", "no", "no")
    case("a tooling self-test", "pull_request", ["scripts/tests/test-issue-land-merge.py"], "no", "no", "no")
    case("a repository check", "pull_request", ["scripts/checks/check-no-personal-data.py"], "no", "no", "no")
    case("a skill", "pull_request", [".claude/skills/issue/SKILL.md"], "no", "no", "no")
    case("the guard hook", "pull_request", [".claude/hooks/guard-shared-tree.py"], "no", "no", "no")
    case("CLAUDE.md", "pull_request", ["CLAUDE.md"], "no", "no", "no")
    case("the README", "pull_request", ["README.md"], "no", "yes", "no")
    case("an engineering note", "pull_request", ["docs/notes/2026-09-04-waiting.md"], "no", "yes", "no")
    case("the notes index", "pull_request", ["docs/engineering-notes.md"], "no", "yes", "no")
    case("an ADR", "pull_request", ["docs/decisions/0026-x.md"], "no", "yes", "no")
    case("the book", "pull_request", ["docs/book/src/index.md"], "no", "yes", "no")
    case("a design canvas", "pull_request", ["Design/Mail Client.dc.html"], "no", "no", "no")
    case("another workflow", "pull_request", [".github/workflows/hooks.yml"], "no", "no", "no")
    case("mise pins", "pull_request", ["mise.toml"], "no", "no", "no")

    # The macOS half. Swift cannot reach a Rust build, and a Rust change
    # regenerates the bindings the Swift compiles against -- so the two keys
    # are not the same question and neither implies the other's `no`.
    case("Swift source", "pull_request", ["macos/Sources/Postio/Shell.swift"], "no", "no", "yes")
    case("a Swift test", "pull_request", ["macos/Tests/PostioKitTests/ReaderTests.swift"],
         "no", "no", "yes")
    case("the Swift manifest", "pull_request", ["macos/Package.swift"], "no", "no", "yes")
    case("the bundle's Info.plist", "pull_request", ["macos/Resources/Info.plist"],
         "no", "no", "yes")
    case("macos/CLAUDE.md is prose", "pull_request", ["macos/CLAUDE.md"], "no", "no", "no")
    for script in ("macos-build.sh", "macos-test.sh", "macos-bundle.sh", "ffi-bindgen.sh"):
        case(f"a macOS build script: {script}", "pull_request", [f"scripts/{script}"],
             "no", "no", "yes")
    # The boundary crate is where the two halves meet: its Rust is tested on a
    # cheap Linux runner (ADR 0019 Q7) and its bindings are what the Swift
    # links against, so it obliges both.
    case("the boundary crate", "pull_request", ["crates/postio-ffi/src/session.rs"],
         "yes", "no", "yes")

    # Mixed: the Rust file decides.
    case("docs plus a crate", "pull_request",
         ["docs/PRODUCT.md", "crates/postio-core/src/lib.rs"], "yes", "yes", "yes")
    case("Swift plus a crate", "pull_request",
         ["macos/Sources/Postio/Shell.swift", "crates/postio-core/src/lib.rs"],
         "yes", "no", "yes")

    # The safe direction.
    case("an empty list", "pull_request", [], "yes", "yes", "yes")
    case("a path nobody classified", "pull_request", ["something/new.txt"], "yes", "yes", "yes")
    case("a push with a range is classified like a PR", "push", ["docs/PRODUCT.md"], "no", "yes", "no")
    case("workflow_dispatch builds everything", "workflow_dispatch", ["docs/PRODUCT.md"], "yes", "yes", "yes")
    case("a schedule builds everything", "schedule", [], "yes", "yes", "yes")

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("ci-changes self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
