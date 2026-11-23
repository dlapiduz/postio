#!/usr/bin/env python3
"""Comment shape may only improve (CLAUDE.md, "Comments").

    scripts/checks/check-comment-shape.py [--update]

Counts, per `.rs` and `.swift` file, comment blocks over three lines (doc
examples with a fence and `POSTIO-` lines excepted), `//!` headers in test
files (one per header), FR-/SC-/T### ids and divider lines, and compares with
scripts/checks/comment-shape-baseline.txt. A file over its count fails.
`--update` rewrites the baseline from the tree.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE = ROOT / "scripts/checks/comment-shape-baseline.txt"
RULE_HELP = ("Shorten each to a why of at most three lines (CLAUDE.md, Comments); "
             "the history goes in the commit body.")

RUNS = "-=*#~─"
TASK_ID = re.compile(r"\b(?:FR-\d+|SC-\d+|T\d{3})\b")
DIVIDER = re.compile(rf"^[{RUNS}]{{4,}}$|^[{RUNS}]{{3,}}\s.*\s[{RUNS}]{{3,}}$|^[{RUNS}]{{3,}}\s.*[{RUNS}]{{3,}}$|^MARK: -")


def marker(stripped):
    if stripped.startswith("//!"):
        return "//!"
    if stripped.startswith("///") and not stripped.startswith("////"):
        return "///"
    if stripped.startswith("//"):
        return "//"
    return None


def body(stripped, mark):
    return stripped[len(mark):].strip()


def violations(path, rel):
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    is_test = re.match(r"crates/[^/]+/tests/", rel) is not None
    out = []
    block = []

    def flush():
        counted = [t for _, t in block if "POSTIO-" not in t]
        if len(counted) > 3 and not any("```" in t for _, t in block):
            out.append((block[0][0], "long-block", f"{len(counted)} lines"))
        block.clear()

    prev = None
    for number, line in enumerate(lines, 1):
        stripped = line.strip()
        mark = marker(stripped)
        if mark != prev:
            flush()
        prev = mark
        if mark is None:
            continue
        text = body(stripped, mark)
        block.append((number, text))
        if mark == "//!" and is_test and len(block) == 1:
            out.append((number, "test-header", text))
        if TASK_ID.search(text):
            out.append((number, "task-id", text))
        if DIVIDER.search(text):
            out.append((number, "divider", text))
    flush()
    return sorted(out)


def scan():
    found = {}
    paths = list(ROOT.glob("crates/*/**/*.rs")) + list(ROOT.glob("macos/**/*.swift"))
    for path in sorted(set(paths)):
        rel = path.relative_to(ROOT).as_posix()
        v = violations(path, rel)
        if v:
            found[rel] = v
    return found


def main():
    now = scan()
    if "--update" in sys.argv:
        BASELINE.write_text("".join(f"{p} {len(v)}\n" for p, v in sorted(now.items())))
        return 0
    base = {}
    if BASELINE.exists():
        for l in BASELINE.read_text().splitlines():
            if l.strip():
                p, n = l.rsplit(None, 1)
                base[p] = int(n)
    bad = 0
    for path, v in sorted(now.items()):
        limit = base.get(path, 0)
        if len(v) > limit:
            print(f"{path}: {len(v)} comment-shape violations, baseline {limit}")
            for number, rule, text in v:
                print(f"{path}:{number}: {rule} {text}")
            bad = 1
        elif len(v) < limit:
            print(f"{path}: {len(v)} < {limit}; lower its line in {BASELINE.relative_to(ROOT)}")
    for path, limit in sorted(base.items()):
        if path not in now:
            print(f"{path}: 0 < {limit}; remove its line from {BASELINE.relative_to(ROOT)}")
    if bad:
        print(RULE_HELP)
    return bad


if __name__ == "__main__":
    sys.exit(main())
