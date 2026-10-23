"""What a test case is, where it lives, what it cites and what it smells of.

Shared by `scripts/test-inventory.py`, `scripts/test-coverage-sets.py` and
`scripts/checks/check-assertion-smells.py` (specs/012-test-suite-value,
research R5 and R6). Everything here reads files; nothing runs cargo.

A case is what `cargo nextest list` lists -- the runner's own idea of a
test, which includes the GTK and widgets suites' harness-free cases, since
their harnesses answer `--list` the way libtest does.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Case:
    binary_id: str  # `postio-ffi::ffi_suite`, or `postio-storage` for unit tests
    name: str  # `focus_pickers::opening_succeeds`
    package: str
    kind: str  # nextest's binary kind: lib, test, bin, proc-macro, bench
    binary_name: str
    crate_dir: Path
    ignored: bool
    filtered_out: bool  # the profile's default-filter excludes it: the measurement tier

    @property
    def id(self) -> str:
        return f"{self.binary_id} {self.name}"

    @property
    def fn_name(self) -> str:
        return self.name.rsplit("::", 1)[-1]


def parse_nextest_list(data: dict, root: Path) -> list[Case]:
    """Every case in `cargo nextest list --message-format json`, in its order.

    A package's directory is `<root>/crates/<package>` when that exists, so a
    recorded listing can be replayed against another tree; otherwise the
    listing's own `cwd`.
    """
    cases = []
    for suite in data["rust-suites"].values():
        package = suite["package-name"]
        crate_dir = root / "crates" / package
        if not crate_dir.is_dir():
            crate_dir = Path(suite["cwd"])
        for name, meta in suite["testcases"].items():
            match = meta.get("filter-match", {})
            cases.append(
                Case(
                    binary_id=suite["binary-id"],
                    name=name,
                    package=package,
                    kind=suite["kind"],
                    binary_name=suite["binary-name"],
                    crate_dir=crate_dir,
                    ignored=bool(meta.get("ignored")),
                    filtered_out=match.get("status") == "mismatch"
                    and match.get("reason") == "default-filter",
                )
            )
    return cases


def _roots(case: Case) -> list[Path]:
    """The file a binary's module tree starts from, for each spelling cargo allows."""
    d = case.crate_dir
    if case.kind in ("lib", "proc-macro"):
        return [d / "src" / "lib.rs"]
    if case.kind == "test":
        return [d / "tests" / f"{case.binary_name}.rs", d / "tests" / case.binary_name / "main.rs"]
    if case.kind == "bench":
        return [d / "benches" / f"{case.binary_name}.rs", d / "benches" / case.binary_name / "main.rs"]
    return [
        d / "src" / "main.rs",
        d / "src" / "bin" / f"{case.binary_name}.rs",
        d / "src" / "bin" / case.binary_name / "main.rs",
    ]


def _defines(path: Path, fn_name: str) -> bool:
    try:
        text = path.read_text()
    except OSError:
        return False
    return re.search(rf"\bfn\s+{re.escape(fn_name)}\b", text) is not None


def case_file(case: Case, root: Path | None = None) -> Path | str:
    """The file that defines `case`, or `"ambiguous"`. Never a guess.

    The module path is walked from the binary's root file: `a::b::tests::f`
    tries `a/b/tests.rs`, then `a/b.rs` (an inline `mod tests`), then `a.rs`,
    each also as `…/mod.rs`, and takes the deepest file that defines `fn f`.
    When none does -- a module a macro made, say -- the crate is searched
    for `fn f`, and only a single hit is believed.
    """
    modules = case.name.split("::")[:-1]
    for start in _roots(case):
        if not start.is_file():
            continue
        base = start.parent
        for depth in range(len(modules), 0, -1):
            stem = base.joinpath(*modules[:depth])
            for candidate in (stem.with_suffix(".rs"), stem / "mod.rs"):
                if candidate.is_file() and _defines(candidate, case.fn_name):
                    return candidate
        if _defines(start, case.fn_name):
            return start
    hits = [p for p in sorted(case.crate_dir.rglob("*.rs")) if _defines(p, case.fn_name)]
    return hits[0] if len(hits) == 1 else "ambiguous"


# --- reading Rust without a parser --------------------------------------

_CHAR = re.compile(r"'(?:\\.|[^\\'])'")
_RAW = re.compile(r'r(#*)"')


def _skip(text: str, i: int) -> int:
    """If a string, char literal or comment starts at `i`, the index after it; else `i`."""
    if text.startswith("//", i):
        end = text.find("\n", i)
        return len(text) if end < 0 else end
    if text.startswith("/*", i):
        end = text.find("*/", i + 2)
        return len(text) if end < 0 else end + 2
    raw = _RAW.match(text, i)
    if raw and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
        close = '"' + raw.group(1)
        end = text.find(close, raw.end())
        return len(text) if end < 0 else end + len(close)
    if text[i] == '"':
        j = i + 1
        while j < len(text):
            if text[j] == "\\":
                j += 2
                continue
            if text[j] == '"':
                return j + 1
            j += 1
        return len(text)
    if text[i] == "'":
        char = _CHAR.match(text, i)
        if char:
            return char.end()
    return i


def _matching(text: str, open_at: int, open_ch: str, close_ch: str) -> int:
    """The index just past the bracket matching the one at `open_at`."""
    depth = 0
    i = open_at
    while i < len(text):
        nxt = _skip(text, i)
        if nxt != i:
            i = nxt
            continue
        if text[i] == open_ch:
            depth += 1
        elif text[i] == close_ch:
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return len(text)


def _fn_at(text: str, fn_name: str) -> re.Match | None:
    return re.search(
        rf"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+{re.escape(fn_name)}\b", text, re.M
    )


def fn_body(path: Path | str, fn_name: str) -> str:
    """The text between the braces of `fn fn_name`, or "" when it cannot be found."""
    if not isinstance(path, Path):
        return ""
    try:
        text = path.read_text()
    except OSError:
        return ""
    found = _fn_at(text, fn_name)
    if not found:
        return ""
    i = found.end()
    while i < len(text) and text[i] != "{":
        nxt = _skip(text, i)
        i = nxt if nxt != i else i + 1
    end = _matching(text, i, "{", "}")
    return text[i + 1 : end - 1]


def file_header(text: str) -> str:
    """The leading `//!` block of a file."""
    lines = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("//!"):
            lines.append(stripped[3:])
        elif stripped and not stripped.startswith("#!"):
            break
    return "\n".join(lines)


def fn_preamble(text: str, fn_name: str) -> str:
    """The comments and doc comments directly above `fn fn_name`, attributes skipped."""
    found = _fn_at(text, fn_name)
    if not found:
        return ""
    lines = []
    for line in reversed(text[: found.start()].splitlines()):
        stripped = line.strip()
        if stripped.startswith("//"):
            lines.append(stripped.lstrip("/!"))
        elif stripped.startswith("#[") or stripped.endswith(")]") or stripped == "]":
            continue
        else:
            break
    return "\n".join(reversed(lines))


# --- what a case cites ----------------------------------------------------

_SPEC = re.compile(r"specs/(\d{3})-|\bspec (\d{3})\b", re.I)
_ITEM = re.compile(r"\b(FR-\d{3}|SC-\d{3}|US\d+|T\d{3})\b")
_ISSUE = re.compile(r"(?<![\w&])#(\d{2,5})\b")


def _specs(text: str) -> set[str]:
    return {a or b for a, b in _SPEC.findall(text)}


def parse_refs(path: Path | str, fn_name: str) -> list[str]:
    """The spec items, specs and issues a case cites, sorted.

    Items (`FR-021`, `T014`, `US3`) repeat across specs, so each is keyed by
    the one spec in scope -- the case's own doc comment's, else its file
    header's -- as `009/FR-021`, and as `?/FR-021` when that is not exactly
    one spec. A spec cited with no item is listed alone (`009`).
    """
    if not isinstance(path, Path):
        return []
    try:
        text = path.read_text()
    except OSError:
        return []
    header, own = file_header(text), fn_preamble(text, fn_name)
    specs = _specs(own) or _specs(header)
    items = set(_ITEM.findall(header)) | set(_ITEM.findall(own))
    scope = next(iter(specs)) if len(specs) == 1 else "?"
    refs = {f"#{n}" for n in _ISSUE.findall(header) + _ISSUE.findall(own)}
    if items:
        refs |= {f"{scope}/{item}" for item in items}
    else:
        refs |= specs
    return sorted(refs)


# --- what a case smells of ------------------------------------------------

_ASSERT = re.compile(r"\b(assert|assert_eq|assert_ne|debug_assert|debug_assert_eq|debug_assert_ne)!\s*\(")
_CONSTANT_ATOMS = [
    re.compile(r'r#*"[^"]*"#*|"(?:\\.|[^"\\])*"'),  # strings
    _CHAR,
    re.compile(r"\b\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?(?:[iuf](?:8|16|32|64|128|size))?\b"),
    re.compile(r"\b(?:true|false)\b"),
    re.compile(r"\b[A-Z][A-Za-z0-9]*::default\(\)"),  # Placement::default()
    re.compile(r"\b[A-Z][A-Za-z0-9]*(?:::[A-Z][A-Za-z0-9_]*)+\b"),  # Placement::Dialog
    re.compile(r"\b(?:[a-z_][a-z0-9_]*::)*[A-Z][A-Z0-9_]*\b(?!\s*[(:{])"),  # LIST_MIN, keys::ARCHIVE
]
_LEFTOVER_OK = re.compile(r"^[\s+\-*/%<>=!&|()]*$")
_COUNTS = re.compile(r"\b(?:calls|call_count|times)\b")
_DISPLAY_SKIP = re.compile(r'eprintln!\s*\(\s*"skipping[^"]*"\s*\)\s*;\s*return\b')


def _split_args(args: str) -> list[str]:
    """Top-level comma-separated arguments of a macro call."""
    parts, depth, start, i = [], 0, 0, 0
    while i < len(args):
        nxt = _skip(args, i)
        if nxt != i:
            i = nxt
            continue
        ch = args[i]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append(args[start:i])
            start = i + 1
        i += 1
    parts.append(args[start:])
    return [p.strip() for p in parts if p.strip()]


def assertions(body: str) -> list[tuple[str, list[str]]]:
    """Each `assert*!` in `body`: its macro name and its top-level arguments."""
    found = []
    for m in _ASSERT.finditer(body):
        end = _matching(body, m.end() - 1, "(", ")")
        found.append((m.group(1), _split_args(body[m.end() : end - 1])))
    return found


def _is_constant(expr: str) -> bool:
    rest = expr
    for atom in _CONSTANT_ATOMS:
        rest = atom.sub(" ", rest)
    return _LEFTOVER_OK.match(rest) is not None


def _compared(macro: str, args: list[str]) -> list[str]:
    """The operands an assertion judges, without its message."""
    return args[:2] if macro.endswith(("_eq", "_ne")) else args[:1]


def smells(body: str, refs: list[str]) -> set[str]:
    """The smells of research R6 that a case's body and refs carry.

    Flags for a person to read, not verdicts: a constant-only case may be
    pinning an accessibility label on purpose, and saying why is what moves
    it out of this list.
    """
    found = set()
    checks = [(macro, _compared(macro, args)) for macro, args in assertions(body)]
    if checks:
        if all(ops and all(_is_constant(op) for op in ops) for _, ops in checks):
            found.add("constant-only")
        if all(
            macro.endswith("assert")
            and ops
            and re.search(r"\.is_(?:ok|some)\(\)\s*$", ops[0])
            and not re.search(r"==|!=|&&|\|\|", ops[0])
            for macro, ops in checks
        ):
            found.add("is-ok-only")
        if all(any(_COUNTS.search(op) for op in ops) for _, ops in checks):
            found.add("mock-count-only")
    if _DISPLAY_SKIP.search(body):
        found.add("display-skip")
    if not refs:
        found.add("untraced")
    return found


# --- which layer, and what it costs ---------------------------------------

FRONTENDS = {"postio-ffi", "postio-gtk", "postio-widgets", "postio-tui"}
CONTROLLERS = {"postio-focus"}
DISPLAY_CRATES = {"postio-gtk", "postio-widgets"}  # their integration suites open windows
MEASUREMENT_MARKER = "POSTIO-MEASUREMENT:"
_READS_REPO = re.compile(r"CARGO_MANIFEST_DIR|include_str!\s*\(")
_REPO_PATH = re.compile(r'"[^"]*(?:\.\./|docs/|scripts/|\.md"|Cargo\.toml)')


def layer(case: Case, path: Path | str, body: str) -> tuple[str, int, bool]:
    """`(layer, cost_rank, display)` for a case: ADR 0049's layers, by crate.

    The layer is the crate's (no per-test list, FR-002), refined by two
    facts in the source: the `POSTIO-MEASUREMENT:` marker the nightly tier
    keys on, and a body that reads the repository's own files. Cost ranks
    order the layers for the redundancy report: a case is only ever proposed
    as redundant to one that is no more expensive (FR-006).
    """
    display = case.package in DISPLAY_CRATES and case.kind == "test"
    if case.package in FRONTENDS:
        name, rank = "frontend", 3 if display else 2
    elif case.package in CONTROLLERS:
        name, rank = "controller", 1
    else:
        name, rank = "rule", 0
    text = path.read_text() if isinstance(path, Path) else ""
    if case.filtered_out or MEASUREMENT_MARKER in file_header(text) or MEASUREMENT_MARKER in body:
        name = "measurement"
    elif _READS_REPO.search(body) and _REPO_PATH.search(body):
        name = "meta"
    return name, rank, display
