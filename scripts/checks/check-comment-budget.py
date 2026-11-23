#!/usr/bin/env python3
"""Comment lines per crate may only go down (CLAUDE.md, "Comments").

    scripts/checks/check-comment-budget.py [--update]

Counts whole-line `//` comments in each crate's `.rs` files and in `macos/`
Swift, and compares them with scripts/checks/comment-budget.txt. A crate over
its line fails, naming the remedy; one under it prints the lower number.
`--update` rewrites the baseline from the tree (only to lower it, or for a
new crate).
"""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE = ROOT / "scripts/checks/comment-budget.txt"


def counts():
    out = {}
    for path in sorted(ROOT.glob("crates/*/**/*.rs")):
        crate = path.relative_to(ROOT).parts[1]
        n = sum(1 for line in path.read_text(encoding="utf-8", errors="replace").splitlines()
                if line.lstrip().startswith("//"))
        out[crate] = out.get(crate, 0) + n
    out["macos"] = sum(
        sum(1 for line in p.read_text(encoding="utf-8", errors="replace").splitlines()
            if line.lstrip().startswith("//"))
        for p in ROOT.glob("macos/**/*.swift"))
    return out


def main():
    now = counts()
    if "--update" in sys.argv:
        BASELINE.write_text("".join(f"{k} {v}\n" for k, v in sorted(now.items())))
        return 0
    budget = {k: int(v) for k, v in (l.split() for l in BASELINE.read_text().splitlines() if l.strip())}
    bad = 0
    for crate, n in sorted(now.items()):
        limit = budget.get(crate)
        if limit is None:
            print(f"{crate}: no budget; add `{crate} {n}` to {BASELINE.relative_to(ROOT)}")
            bad = 1
        elif n > limit:
            print(f"{crate}: {n} comment lines, budget {limit}. Keep only a why the code "
                  f"cannot show (CLAUDE.md, Comments); move history to the commit body.")
            bad = 1
        elif n < limit:
            print(f"{crate}: {n} < {limit}; lower its line in {BASELINE.relative_to(ROOT)}")
    return bad


if __name__ == "__main__":
    sys.exit(main())
