#!/usr/bin/env python3
"""Refuse a floating rustc: CI and every session must agree on one compiler.

CI used to run ``rustup default stable`` while the repository pinned nothing,
so the compiler that decided whether a PR was green was whatever shipped most
recently, and every session here built with whatever it happened to have
installed. That is not a hypothetical: rustc 1.98.0 (2026-08-18) tightened
``unused_imports`` for redundant glob imports and flagged ``use adw::prelude::*``
in ``compose::tests``. Sessions on 1.97.1 saw nothing, every local gate passed,
and the clippy job failed on ``-D warnings`` after the push -- a failure that
was unreproducible locally *by construction*. Issue #38.

It recurs on every rustc release, always the same way: green locally, red in
CI, on a lint nobody wrote.

# The rules

1. ``rust-toolchain.toml`` exists and names an **exact version**, not a
   channel. ``stable`` in that file floats exactly like ``rustup default
   stable`` did; the point of the pin is that a compiler change becomes a
   commit somebody made on purpose.
2. No GitHub workflow selects a toolchain of its own. A ``rustup default
   stable`` anywhere in ``.github/workflows`` silently wins over the file for
   that job, which is the bug this check exists to prevent -- reintroduced in
   one line, in a file nobody re-reads.
3. ``rust-version`` in the workspace manifest does not claim an MSRV older
   than the pinned compiler. The manifest said ``1.90`` while the pin said
   ``1.98``, so nothing had ever built this workspace on 1.90 and the claim
   was evidence-free -- it would have become false the first time anyone used
   a 1.91 feature, and nobody would have found out. An MSRV is a promise to
   somebody; while the only consumer is this repository, the honest promise is
   the compiler that is actually used.

4. The repository's ``mise.toml`` does not name ``rust``. mise exports
   ``RUSTUP_TOOLCHAIN`` for every shell in the repository, which beats this
   file (below), so a ``rust`` entry there is a second pin that wins. One
   arrived as ``rust = "latest"`` in an unrelated commit (2026-09-05),
   resolved to the 1.98 that was installed, and kept every session on 1.98
   after the 1.99 bump. mise.toml's own header already said it must not.

5. CI runs the repository's Python from ``mise.toml``. The checks are Python
   scripts and ``mise.toml`` pins the interpreter, but a workflow that calls
   ``python3`` gets whatever the runner image ships, so local and CI could
   disagree about the language the gates are written in -- the compiler bug
   again, one layer up. Every job that runs a repository script or ``python``
   must use ``./.github/actions/python`` after its checkout; that composite
   action reads the version from ``mise.toml`` and is the only place allowed
   to name ``actions/setup-python``, so the version is written once.

# What this check cannot see, and what to do about it

``RUSTUP_TOOLCHAIN`` in the environment **overrides rust-toolchain.toml**.
rustup's precedence is env var, then ``rustup override``, then the file. So a
machine that exports it -- this project's own workstation does, from
``~/.config/mise/config.toml`` -- ignores the pin while looking pinned, which
is the original bug wearing the fix's clothes.

A repository check cannot police anyone's shell, so this reports the skew
rather than failing on it: an exit status keyed to a developer's environment
would make CI's answer depend on the runner's, which is the thing being
fixed. ``--strict`` turns the warning into a failure for anyone who wants it
enforced locally. See ``docs/archive/engineering-notes.md``.

# Exit status

0 clean, 1 the pin is missing or a workflow overrides it, 2 could not run.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

# `rustup default <channel>` / `rustup toolchain install <channel>` where the
# channel is a floating name rather than a version. `1.98.0` is fine; `stable`
# is the bug.
#
# A *dated* nightly -- `nightly-2026-08-24` -- is not floating and is not the
# bug: it names one compiler as exactly as `1.98.0` does, and it is the only
# way to spell what libFuzzer needs, since `-Z sanitizer=address` does not
# exist on stable at any version. `fuzz/` is built with one, on a schedule,
# and never decides whether a PR is green. The rule this file is about is
# "the compiler must be a decision somebody made", not "the compiler must be
# stable" -- so the date is what makes it acceptable, and a bare `nightly`
# stays refused. See #147.
#
# The optional `run:` allows for the one-line form. Without it this matched
# only the `run: |` block spelling, so `run: rustup default stable` on a single
# line -- the shorter and likelier way to reintroduce the bug -- went straight
# past the guard.
FLOATING = re.compile(
    r"^\s*(?:-\s*)?(?:run:\s*)?rustup\s+(?:default|toolchain\s+install)\s+"
    r"(stable|beta|nightly)(?!-\d{4}-\d{2}-\d{2})\b",
    re.MULTILINE,
)

# `channel = "1.98.0"` in rust-toolchain.toml. Deliberately strict: a channel
# name here floats just as hard as it did in the workflow.
EXACT = re.compile(r'^\s*channel\s*=\s*"(\d+\.\d+(?:\.\d+)?)"\s*$', re.MULTILINE)


# `rust-version = "1.90"` in `[workspace.package]`. Two or three components.
RUST_VERSION = re.compile(
    r'^\s*rust-version\s*=\s*"(\d+\.\d+(?:\.\d+)?)"\s*$', re.MULTILINE
)


def declared_msrv(root: Path) -> str | None:
    """`rust-version` from the workspace manifest, if it declares one."""
    try:
        text = (root / "Cargo.toml").read_text()
    except OSError:
        return None
    found = RUST_VERSION.search(text)
    return found.group(1) if found else None


def as_tuple(version: str) -> tuple[int, ...]:
    """`"1.98.0"` -> `(1, 98, 0)`, so versions compare as numbers rather than
    as strings -- `"1.9" > "1.10"` is true for strings and wrong."""
    return tuple(int(part) for part in version.split("."))


def repository_root() -> Path:
    """The checkout this script lives in."""
    try:
        top = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
            cwd=Path(__file__).resolve().parent,
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError) as error:
        print(f"cannot find the repository root: {error}", file=sys.stderr)
        raise SystemExit(2) from error
    return Path(top)


def pinned_version(root: Path) -> str | None:
    """The exact version `rust-toolchain.toml` names, if it names one."""
    path = root / "rust-toolchain.toml"
    if not path.is_file():
        return None
    found = EXACT.search(path.read_text(encoding="utf-8"))
    return found.group(1) if found else None


MISE_RUST = re.compile(r'^\s*rust\s*=', re.MULTILINE)


def mise_pins_rust(root: Path) -> int | None:
    """The line of mise.toml that names `rust`, if any."""
    path = root / "mise.toml"
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8")
    match = MISE_RUST.search(text)
    return None if match is None else text[: match.start()].count("\n") + 1


def floating_workflows(root: Path) -> list[str]:
    """Workflow lines that pick a toolchain instead of honouring the file."""
    offenders: list[str] = []
    workflows = root / ".github" / "workflows"
    for path in sorted(workflows.glob("*.yml")) + sorted(workflows.glob("*.yaml")):
        text = path.read_text(encoding="utf-8")
        for match in FLOATING.finditer(text):
            line = text[: match.start()].count("\n") + 1
            offenders.append(f"{path.relative_to(root)}:{line}: {match.group(0).strip()}")
    return offenders


PYTHON_ACTION = "./.github/actions/python"

# What running the repository's Python looks like inside a `run:` body: the
# interpreter, a `.py` script, or any script under `scripts/` (most of which
# call python3). A cache key or `with:` value is not a run body.
RUNS_PYTHON = re.compile(
    r"\bpython3?\b|\.py\b|(?<![\w./-])scripts/\w|\.claude/hooks"
)
JOB_HEAD = re.compile(r"^  ([A-Za-z0-9_-]+):\s*$")
RUN_LINE = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")


def jobs_of(text: str) -> list[tuple[str, int, list[str]]]:
    """`(name, first line number, lines)` for each job in a workflow."""
    lines = text.splitlines()
    jobs: list[tuple[str, int, list[str]]] = []
    in_jobs = False
    for number, line in enumerate(lines, start=1):
        if line.startswith("jobs:"):
            in_jobs = True
        elif in_jobs and line and not line.startswith((" ", "#")):
            in_jobs = False
        head = JOB_HEAD.match(line) if in_jobs else None
        if head:
            jobs.append((head.group(1), number, []))
        elif jobs and in_jobs:
            jobs[-1][2].append(line)
    return jobs


def run_bodies(lines: list[str]) -> str:
    """The text of every `run:` step in a job, comment lines dropped."""
    bodies: list[str] = []
    index = 0
    while index < len(lines):
        found = RUN_LINE.match(lines[index])
        index += 1
        if not found:
            continue
        indent, rest = len(found.group(1)), found.group(2)
        if rest.strip() and rest.strip()[0] not in "|>":
            bodies.append(rest)
            continue
        while index < len(lines) and (
            not lines[index].strip() or len(lines[index]) - len(lines[index].lstrip()) > indent
        ):
            bodies.append(lines[index])
            index += 1
    return "\n".join(b for b in bodies if not b.lstrip().startswith("#"))


def python_problems(root: Path) -> list[str]:
    """Rule 5: workflow jobs that run Python must set it up from mise.toml."""
    problems: list[str] = []
    workflows = root / ".github" / "workflows"
    uses_action = False
    for path in sorted(workflows.glob("*.yml")) + sorted(workflows.glob("*.yaml")):
        rel = path.relative_to(root)
        text = path.read_text(encoding="utf-8")
        for match in re.finditer(r"^\s*(?:-\s+)?uses:\s*actions/setup-python@", text, re.MULTILINE):
            line = text[: match.start()].count("\n") + 1
            problems.append(
                f"{rel}:{line}: names actions/setup-python; use {PYTHON_ACTION}, "
                f"which reads the version from mise.toml"
            )
        for name, first, lines in jobs_of(text):
            body = "\n".join(lines)
            has_action = PYTHON_ACTION in body
            uses_action = uses_action or has_action
            if has_action:
                at = body.index(PYTHON_ACTION)
                checkout = body.find("actions/checkout@")
                if checkout > at:
                    problems.append(
                        f"{rel}: job `{name}` sets Python up before its checkout, "
                        f"so mise.toml is not there to read"
                    )
            elif RUNS_PYTHON.search(run_bodies(lines)):
                problems.append(
                    f"{rel}:{first}: job `{name}` runs Python from the runner image; "
                    f"add `- uses: {PYTHON_ACTION}` after its checkout"
                )
    action = root / PYTHON_ACTION[2:] / "action.yml"
    if uses_action and not action.is_file():
        problems.append(f"{action.relative_to(root)} is missing")
    elif action.is_file():
        text = action.read_text(encoding="utf-8")
        if "mise.toml" not in text or re.search(r"python-version:\s*[\"']?\d", text):
            problems.append(
                f"{action.relative_to(root)} must read the version from mise.toml, "
                f"not carry one of its own"
            )
    return problems


def report_environment_skew(pinned: str) -> None:
    """Say so when this shell will ignore the pin. Never fatal -- see the docs."""
    override = os.environ.get("RUSTUP_TOOLCHAIN")
    if override and not override.startswith(pinned):
        print(
            f"warning: RUSTUP_TOOLCHAIN={override} overrides rust-toolchain.toml "
            f"({pinned}), so this shell is not building with the pinned compiler.\n"
            f"         rustup reads the environment before the file. On this "
            f"project's workstation the export comes from ~/.config/mise/config.toml.",
            file=sys.stderr,
        )


def main() -> int:
    strict = "--strict" in sys.argv[1:]
    root = repository_root()

    pinned = pinned_version(root)
    if pinned is None:
        path = root / "rust-toolchain.toml"
        why = "does not name an exact version" if path.is_file() else "is missing"
        print(
            f"toolchain check FAILED: rust-toolchain.toml {why}.\n"
            f"  CI and every session must agree on one compiler, or a lint that "
            f"fires in one and not the other turns main red on a change nobody "
            f"made. Add:\n\n"
            f'    [toolchain]\n    channel = "1.98.0"\n',
            file=sys.stderr,
        )
        return 1

    offenders = floating_workflows(root)
    if offenders:
        print(
            f"toolchain check FAILED: {len(offenders)} workflow step(s) select a "
            f"floating toolchain, which wins over rust-toolchain.toml ({pinned}):",
            file=sys.stderr,
        )
        for offender in offenders:
            print(f"  {offender}", file=sys.stderr)
        print(
            "  Drop the selection and let the file decide; `rustup show` installs "
            "what it names.",
            file=sys.stderr,
        )
        return 1

    msrv = declared_msrv(root)
    if msrv is not None and as_tuple(msrv) < as_tuple(pinned)[: len(as_tuple(msrv))]:
        print(
            f"toolchain check FAILED: Cargo.toml claims rust-version = "
            f'"{msrv}" but rust-toolchain.toml pins {pinned}.\n'
            f"  Nothing ever builds this workspace on {msrv}, so the claim is "
            f"untested -- it becomes false the first time anyone uses a newer\n"
            f"  feature, and no gate here would notice. Either set rust-version "
            f"to match the pin, or add a CI job that checks on {msrv}.",
            file=sys.stderr,
        )
        return 1

    mise_line = mise_pins_rust(root)
    if mise_line is not None:
        print(
            f"toolchain check FAILED: mise.toml:{mise_line} names rust, and mise "
            f"exports RUSTUP_TOOLCHAIN from it,\n"
            f"  which beats rust-toolchain.toml ({pinned}) in every shell in this "
            f"repository. Delete the line: the pin is rust-toolchain.toml's.",
            file=sys.stderr,
        )
        return 1

    python = python_problems(root)
    if python:
        print(
            f"toolchain check FAILED: {len(python)} workflow Python problem(s). "
            f"mise.toml pins the interpreter the checks run on, and CI must "
            f"use it:",
            file=sys.stderr,
        )
        for problem in python:
            print(f"  {problem}", file=sys.stderr)
        return 1

    override = os.environ.get("RUSTUP_TOOLCHAIN")
    skewed = bool(override) and not override.startswith(pinned)
    if skewed:
        report_environment_skew(pinned)
        if strict:
            return 1

    print(f"toolchain check passed (pinned to {pinned}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
