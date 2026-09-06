#!/usr/bin/env python3
"""Refuse a wall-clock deadline in a test that `POSTIO_TEST_PATIENCE` cannot reach.

#842 built the dial: `postio_test_support::patience` and `scaled` multiply
every deadline in the suite by `POSTIO_TEST_PATIENCE`, so making a loaded
machine more patient is one environment variable rather than a pull request
that enlarges a constant and slows every run forever.

**The dial only reaches the deadlines that go through that crate.** #957 is
what the gap costs: three `gtk_suite` cases flake on a busy workstation, one
a run, each passing alone every time -- and every one of them waits on a
deadline written by hand. `gtk_composer_toolbar` waits a hardcoded 20
seconds. Setting `POSTIO_TEST_PATIENCE=8` before a local full-suite run does
nothing for exactly the cases that need it, which is why "leave it, CI is the
arbiter" kept looking like the only option.

# The rule

In a test file, `Instant::now() + <expr>` must reach the dial: `<expr>` names
`scaled(...)`, `patience()`, or a `PATIENCE` constant.

# The exception, and why it needs a reason

Some deadlines *are* the subject. A debounce test asserts that nothing
happens for the debounce window; a negative assertion ("not marked read
yet") is only as strong as the time it waited. Scaling those changes what
the test proves rather than how patient it is. Those say so:

    // POSTIO-FIXED-DEADLINE: the debounce window is what this asserts.
    let deadline = Instant::now() + DEBOUNCE + Duration::from_millis(60);

The marker must carry a reason -- a bare marker is a silencer, and the next
person needs to know whether the number is load-bearing or was merely never
revisited.

# Scope

Two shapes, one rule.

**Rust**: `Instant::now() + ...`, which all three of #957's cases use.
`tokio::time::timeout` is the same hazard on the async side and is not covered
yet; see the issue.

**Python**: `subprocess.run(..., timeout=...)` under `scripts/tests`. The 81
self-tests there shell out to the scripts they cover, and their deadlines were
written by hand and reachable by nothing -- #1243 is one expiring on a runner
that builds 81 sandboxes four at a time, on a branch that touched nothing to
do with its subject. They go through `scripts/lib/patience.py` now, which
scales the deadline and reports an expiry as an expiry (#1249). This is what
stops the next one being written straight to `subprocess`.

# Exit status

0 clean, 1 a deadline the dial cannot reach, 2 the check could not run.
"""

from __future__ import annotations

import io
import re
import subprocess
import sys
import tokenize
from pathlib import Path

# Where `Instant::now()` is mentioned at all -- the cheap first pass.
MENTIONS_NOW = re.compile(r"Instant::now\s*\(\s*\)")
# The deadline shape itself, tested against the whole statement rather than
# the line: rustfmt breaks a long `let deadline = ...` wherever it likes, and
# a check the formatter can silence is not a check. `Instant::now() < deadline`
# in a loop condition has no `+` and is not a deadline being *set*.
DEADLINE = re.compile(r"(?:\w+::)*Instant::now\s*\(\s*\)\s*\+")
# What proves the expression answers to POSTIO_TEST_PATIENCE.
SCALED = re.compile(r"\bscaled\s*\(|\bpatience\s*\(\s*\)|\bPATIENCE\b")
# The escape, which must be followed by a reason rather than left bare.
MARKER = re.compile(r"POSTIO-FIXED-DEADLINE:\s*(?P<reason>\S.*)")
BARE_MARKER = re.compile(r"POSTIO-FIXED-DEADLINE:")

# How far above the deadline the marker may sit: a helper function documents
# its own fixed deadline in the doc comment, not on the `let` line.
LOOKBACK = 8

# `Instant::now() + limit`, where `limit` was bound above. One hop, same file:
# a deadline is often computed once and reused, and refusing that would push
# every call site into repeating the `scaled` call.
BARE_NAME = re.compile(r"^\s*\+?\s*(?P<name>[A-Za-z_]\w*)\s*;?\s*$")


class CheckError(Exception):
    """The check could not be run, as opposed to: the check failed."""


def tracked(pattern: str) -> list[Path]:
    try:
        listed = subprocess.run(
            ["git", "ls-files", pattern],
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise CheckError("git is not on PATH") from error
    except subprocess.CalledProcessError as error:
        raise CheckError(f"git ls-files failed: {error.stderr.strip()}") from error
    return [Path(line) for line in listed.stdout.splitlines() if line]


def statement(lines: list[str], start: int) -> str:
    """The statement beginning at `start`, joined and whitespace-collapsed."""
    joined = []
    for line in lines[start : start + 4]:
        joined.append(line)
        if ";" in line:
            break
    return " ".join(" ".join(joined).split())


def binding_of(name: str, lines: list[str]) -> str | None:
    """Where `name` was bound in this file, if it was bound exactly once."""
    binder = re.compile(
        rf"^\s*(?:let(?:\s+mut)?|const|static)\s+{re.escape(name)}\b"
    )
    matches = [line for line in lines if binder.match(line)]
    return matches[0] if len(matches) == 1 else None


def offenders(path: Path) -> list[tuple[int, str]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError):
        return []
    found = []
    for index, line in enumerate(lines):
        if not MENTIONS_NOW.search(line):
            continue
        expression = statement(lines, index)
        if not DEADLINE.search(expression):
            continue
        if SCALED.search(expression):
            continue
        # `Instant::now() + limit`: follow `limit` to where it was bound.
        addend = expression.split("+", 1)[1] if "+" in expression else ""
        named = BARE_NAME.match(addend)
        if named:
            bound = binding_of(named.group("name"), lines)
            if bound is not None and SCALED.search(bound):
                continue
        window = lines[max(0, index - LOOKBACK) : index + 1]
        if any(MARKER.search(near) for near in window):
            continue
        if any(BARE_MARKER.search(near) for near in window):
            found.append((index + 1, "POSTIO-FIXED-DEADLINE: with no reason after it"))
            continue
        found.append((index + 1, line.strip()))
    return found


def python_offenders(path: str) -> list[tuple[int, str]]:
    """`subprocess.run(..., timeout=...)` calls the dial cannot reach.

    Tokenized rather than matched line by line: several of these self-tests
    embed whole Python and shell fixtures in string literals, and a regex
    reads those as code. The codemod that introduced `patience.run` learned
    that the expensive way -- its first pass inserted an import into the middle
    of a fixture in three files, because they contain `import` at column zero
    inside a triple-quoted string.
    """
    source = Path(path).read_text(encoding="utf-8", errors="replace")
    if "subprocess.run" not in source:
        return []
    lines = source.splitlines()
    try:
        tokens = [
            t
            for t in tokenize.generate_tokens(io.StringIO(source).readline)
            if t.type not in (tokenize.COMMENT, tokenize.NL, tokenize.NEWLINE,
                              tokenize.INDENT, tokenize.DEDENT)
        ]
    except (tokenize.TokenError, IndentationError, SyntaxError):
        # Not this check's business to fail on a file that will not parse;
        # running it is what reports that.
        return []

    found = []
    for i, token in enumerate(tokens):
        if not (token.type == tokenize.NAME and token.string == "subprocess"):
            continue
        if i + 3 >= len(tokens):
            continue
        if not (
            tokens[i + 1].string == "."
            and tokens[i + 2].string == "run"
            and tokens[i + 3].string == "("
        ):
            continue
        depth = 0
        timed = False
        for j in range(i + 3, len(tokens)):
            text = tokens[j].string
            if text in "([{":
                depth += 1
            elif text in ")]}":
                depth -= 1
                if depth == 0:
                    break
            elif (
                tokens[j].type == tokenize.NAME
                and text == "timeout"
                and j + 1 < len(tokens)
                and tokens[j + 1].string == "="
            ):
                timed = True
        if not timed:
            continue
        line = token.start[0]
        window = "\n".join(lines[max(0, line - 1 - LOOKBACK) : line])
        if MARKER.search(window):
            continue
        if BARE_MARKER.search(window):
            found.append((line, "POSTIO-FIXED-DEADLINE with no reason"))
            continue
        found.append((line, "subprocess.run(timeout=...) -- use patience.run"))
    return found


def main() -> int:
    try:
        # git pathspec `*` matches `/` too, so one pattern reaches
        # `tests/suite/case.rs` as well as `tests/case.rs`.
        sources = sorted(set(tracked("crates/*/tests/*.rs")))
        self_tests = sorted(set(tracked("scripts/tests/*.py")))
    except CheckError as error:
        print(f"cannot run the check: {error}", file=sys.stderr)
        return 2

    problems = [
        f"{path}:{line}: {text}"
        for path in sources
        for line, text in offenders(path)
    ] + [
        f"{path}:{line}: {text}"
        for path in self_tests
        for line, text in python_offenders(path)
    ]

    if not problems:
        print(
            "test-deadlines-scale check passed "
            f"({len(sources)} test files, {len(self_tests)} self-tests)."
        )
        return 0

    print("test-deadlines-scale check FAILED\n", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"\n{len(problems)} deadline(s) POSTIO_TEST_PATIENCE cannot reach.\n\n"
        "A hand-rolled deadline measures the process it runs in. On a shared\n"
        "workstation that is a flake nobody can reproduce alone -- #957, one\n"
        "gtk_suite case a run -- and the dial #842 built to absorb it stops\n"
        "at the edge of postio_test_support.\n\n"
        "Wrap the duration so the dial reaches it:\n\n"
        "    use postio_test_support::scaled;\n"
        "    let deadline = Instant::now() + scaled(Duration::from_secs(20));\n\n"
        "Or, where the duration is what the test asserts -- a debounce, a\n"
        "grace period, a negative assertion whose strength is the time it\n"
        "waited -- say so, with the reason:\n\n"
        "    // POSTIO-FIXED-DEADLINE: the debounce window is the subject here.\n\n"
        "In a self-test under scripts/tests, the same rule reads:\n\n"
        "    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / \"lib\"))\n"
        "    import patience\n\n"
        "    patience.run([...], timeout=30)\n\n"
        "`timeout` stays the number that is right on an idle machine; the dial\n"
        "is what makes it right on a busy one, and an expiry then reports as an\n"
        "expiry rather than as the script answering wrongly.\n",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
