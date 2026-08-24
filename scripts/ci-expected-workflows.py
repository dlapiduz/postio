#!/usr/bin/env python3
"""Name the workflows a set of changed paths will schedule on a pull request.

`issue-land.sh` used to ask `gh pr checks` whether anything was going to run
and merge if the answer was "no checks reported". That answer means two
different things -- "this branch schedules no workflow" and "GitHub has not
registered one yet" -- and the script could not tell them apart. Lost one
way it cost a re-run; lost the other it merged a five-crate change before CI
started (#135, #139, #131).

The branch's own diff is what knows, so this decides from the diff. The
workflow files are the authority: their `on.pull_request` filters say
exactly which changes schedule them, and reading those is not a guess.

    scripts/ci-expected-workflows.py crates/postio-core/src/lib.rs
    git diff --name-only origin/main...HEAD | scripts/ci-expected-workflows.py

Prints one workflow name per line, in filename order.

Exit status:
  0  at least one workflow will run -- the caller must wait for a check
  1  nothing will run
  2  the workflows could not be read; the caller should assume the worst

No network, no `gh`, no git. Given the same paths it answers the same way
every time, which is the whole point: the old heuristic did not.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# --------------------------------------------------------------------------
# A very small YAML reader.
#
# PyYAML is not installed on this box or on the CI runner's default image,
# and adding a dependency to make one script work is a poor trade against
# ~120 lines that read the subset GitHub workflow triggers actually use:
# nested mappings, scalar and flow-sequence values, block sequences of
# scalars, and -- the part that matters -- anchors and aliases, because the
# real ci.yml defines its prose ignore list once as `&prose` and reuses it
# for `pull_request` as `*prose`. A reader that skipped the alias would see
# an unfiltered trigger and claim every prose PR runs CI.
# --------------------------------------------------------------------------


class YamlError(Exception):
    """The subset reader met something it will not guess about."""


def _strip_comment(text: str) -> str:
    """Drop a trailing `# comment`, respecting quotes."""
    out: list[str] = []
    quote: str | None = None
    for i, ch in enumerate(text):
        if quote:
            if ch == quote:
                quote = None
        elif ch in "'\"":
            quote = ch
        elif ch == "#" and (i == 0 or text[i - 1] in " \t"):
            break
        out.append(ch)
    return "".join(out).strip()


def _scalar(text: str) -> str:
    text = text.strip()
    if len(text) >= 2 and text[0] == text[-1] and text[0] in "'\"":
        return text[1:-1]
    return text


def _flow_sequence(text: str) -> list[str]:
    """`[main, release]` -> ['main', 'release']."""
    inner = text.strip()[1:-1].strip()
    if not inner:
        return []
    return [_scalar(part) for part in inner.split(",")]


_KEY = re.compile(r"^([^\s:][^:]*):(?:\s+(.*))?$")


class _Reader:
    def __init__(self, text: str) -> None:
        self.lines: list[tuple[int, str]] = []
        for raw in text.splitlines():
            stripped = _strip_comment(raw)
            if not stripped:
                continue
            self.lines.append((len(raw) - len(raw.lstrip(" ")), stripped))
        self.pos = 0
        self.anchors: dict[str, object] = {}

    def _peek(self) -> tuple[int, str] | None:
        return self.lines[self.pos] if self.pos < len(self.lines) else None

    def parse(self, indent: int) -> object:
        """Read the block at `indent` or deeper. Mapping, list, or None."""
        head = self._peek()
        if head is None or head[0] < indent:
            return None
        if head[1].startswith("- "):
            return self._sequence(head[0])
        return self._mapping(head[0])

    def _sequence(self, indent: int) -> list[str]:
        items: list[str] = []
        while True:
            head = self._peek()
            if head is None or head[0] != indent or not head[1].startswith("- "):
                break
            self.pos += 1
            items.append(_scalar(head[1][2:]))
        return items

    def _mapping(self, indent: int) -> dict[str, object]:
        node: dict[str, object] = {}
        while True:
            head = self._peek()
            if head is None or head[0] < indent:
                break
            if head[0] > indent:
                raise YamlError(f"unexpected indent at {head[1]!r}")
            match = _KEY.match(head[1])
            if match is None:
                break
            self.pos += 1
            key = _scalar(match.group(1))
            rest = (match.group(2) or "").strip()
            node[key] = self._value(rest, indent)
        return node

    def _value(self, rest: str, indent: int) -> object:
        anchor: str | None = None
        if rest.startswith("&"):
            name, _, remainder = rest[1:].partition(" ")
            anchor, rest = name, remainder.strip()
        if rest.startswith("*"):
            name = rest[1:].strip()
            if name not in self.anchors:
                raise YamlError(f"alias *{name} has no anchor")
            value: object = self.anchors[name]
        elif rest.startswith("["):
            value = _flow_sequence(rest)
        elif rest:
            value = _scalar(rest)
        else:
            value = self.parse(indent + 1)
        if anchor is not None:
            self.anchors[anchor] = value
        return value


    def document(self, wanted: set[str]) -> dict[str, object]:
        """Top-level keys, descending into `wanted` and stepping over the rest.

        `jobs:` is most of a workflow file and none of this script's business:
        it holds block scalars, sequences of mappings and expression syntax,
        all of which a reader this size would have to either implement or
        choke on. Skipping it wholesale is not a shortcut -- it is the reason
        the reader can stay small enough to be worth trusting.
        """
        node: dict[str, object] = {}
        while True:
            head = self._peek()
            if head is None:
                break
            if head[0] != 0:
                self.pos += 1
                continue
            match = _KEY.match(head[1])
            if match is None:
                self.pos += 1
                continue
            self.pos += 1
            key = _scalar(match.group(1))
            if key in wanted:
                node[key] = self._value((match.group(2) or "").strip(), 0)
            else:
                self._skip_block()
        return node

    def _skip_block(self) -> None:
        while True:
            head = self._peek()
            if head is None or head[0] == 0:
                break
            self.pos += 1


def load_workflow(path: Path) -> dict[str, object]:
    reader = _Reader(path.read_text(encoding="utf-8"))
    # YAML 1.1 reads a bare `on` as the boolean true; accept both spellings.
    return reader.document({"name", "on", "true", "True"})


# --------------------------------------------------------------------------
# GitHub filter patterns.
#
# Not shell globs and not gitignore: `*` and `?` stop at a slash, `**`
# crosses them, patterns match the whole path, and a leading `!` reverses an
# earlier match in list order. `'*.md'` in the real ci.yml means top-level
# prose only, which is exactly why `docs/keybindings.md` -- generated from
# the command registry, with a test that fails when it drifts -- still runs
# CI when someone hand-edits it.
# --------------------------------------------------------------------------


def _pattern_to_regex(pattern: str) -> re.Pattern[str]:
    out: list[str] = []
    i = 0
    while i < len(pattern):
        ch = pattern[i]
        if ch == "*":
            if pattern.startswith("**", i):
                # `**/` is zero-or-more leading directories, so `**/x.js`
                # matches a top-level x.js as well as a nested one.
                if pattern.startswith("**/", i):
                    out.append("(?:.*/)?")
                    i += 3
                    continue
                out.append(".*")
                i += 2
                continue
            out.append("[^/]*")
        elif ch == "?":
            out.append("[^/]")
        else:
            out.append(re.escape(ch))
        i += 1
    return re.compile("".join(out) + r"\Z")


def path_matches(path: str, patterns: list[str]) -> bool:
    """GitHub's ordered evaluation: a later `!` pattern undoes an earlier hit."""
    matched = False
    for pattern in patterns:
        if pattern.startswith("!"):
            if _pattern_to_regex(pattern[1:]).match(path):
                matched = False
        elif _pattern_to_regex(pattern).match(path):
            matched = True
    return matched


# --------------------------------------------------------------------------
# The predicate.
# --------------------------------------------------------------------------


def _as_list(value: object) -> list[str] | None:
    if value is None:
        return None
    if isinstance(value, list):
        return [str(item) for item in value]
    return [str(value)]


def workflow_runs(
    document: dict[str, object], changed: list[str], base: str
) -> bool:
    """Would this workflow be scheduled by a pull request touching `changed`?"""
    triggers = document.get("on", document.get("true", document.get("True")))
    if not isinstance(triggers, dict) or "pull_request" not in triggers:
        return False
    if not changed:
        return False

    config = triggers["pull_request"]
    if not isinstance(config, dict):
        return True  # `pull_request:` with nothing under it: no filters.

    # `branches` on a pull_request filters the *base* branch, not the head.
    branches = _as_list(config.get("branches"))
    if branches is not None and not path_matches(base, branches):
        return False
    ignore_branches = _as_list(config.get("branches-ignore"))
    if ignore_branches is not None and path_matches(base, ignore_branches):
        return False

    paths = _as_list(config.get("paths"))
    if paths is not None:
        return any(path_matches(p, paths) for p in changed)
    paths_ignore = _as_list(config.get("paths-ignore"))
    if paths_ignore is not None:
        return any(not path_matches(p, paths_ignore) for p in changed)
    return True


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", help="changed paths; else stdin")
    parser.add_argument(
        "--workflows",
        default=str(Path(__file__).resolve().parent.parent / ".github" / "workflows"),
        help="directory of workflow files",
    )
    parser.add_argument("--base", default="main", help="the PR's base branch")
    args = parser.parse_args(argv)

    changed = args.paths
    if not changed and not sys.stdin.isatty():
        changed = [line.strip() for line in sys.stdin if line.strip()]

    directory = Path(args.workflows)
    if not directory.is_dir():
        print(f"no workflow directory at {directory}", file=sys.stderr)
        return 2

    expected: list[str] = []
    for path in sorted(directory.glob("*.y*ml")):
        try:
            document = load_workflow(path)
        except (OSError, YamlError) as exc:
            print(f"cannot read {path.name}: {exc}", file=sys.stderr)
            return 2
        if workflow_runs(document, changed, args.base):
            name = document.get("name")
            expected.append(str(name) if isinstance(name, str) else path.stem)

    for name in expected:
        print(name)
    return 0 if expected else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
