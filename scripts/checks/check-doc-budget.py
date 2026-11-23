#!/usr/bin/env python3
"""Steering documents may only get shorter (CLAUDE.md, "Say it where it persists").

    scripts/checks/check-doc-budget.py [--update]

Counts words outside code fences in the files agents read to steer their
work and compares them with scripts/checks/doc-budget.txt. A file over its
budget fails: say the rule once, as a rule, and leave the history to git.
`--update` rewrites the baseline (only to lower it, or for a new file).
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE = ROOT / "scripts/checks/doc-budget.txt"
GLOBS = ["CLAUDE.md", "macos/CLAUDE.md", "docs/*.md", "docs/gotchas/*.md",
         "docs/decisions/*.md", ".specify/memory/constitution.md",
         ".specify/templates/*.md", ".claude/skills/*/SKILL.md", ".claude/agents/*.md"]
SKIP = {"docs/keybindings.md"}  # generated from the registry


def words(path):
    text = re.sub(r"(?ms)^```.*?^```", "", path.read_text(encoding="utf-8"))
    return len(text.split())


def counts():
    out = {}
    for pattern in GLOBS:
        for path in sorted(ROOT.glob(pattern)):
            rel = str(path.relative_to(ROOT))
            if rel not in SKIP:
                out[rel] = words(path)
    return out


def main():
    now = counts()
    if "--update" in sys.argv:
        BASELINE.write_text("".join(f"{k} {v}\n" for k, v in sorted(now.items())))
        return 0
    budget = {k: int(v) for k, v in (l.rsplit(None, 1) for l in BASELINE.read_text().splitlines() if l.strip())}
    bad = 0
    for rel, n in sorted(now.items()):
        limit = budget.get(rel)
        if limit is None:
            print(f"{rel}: no budget; add `{rel} {n}` to {BASELINE.relative_to(ROOT)} (keep it short)")
            bad = 1
        elif n > limit:
            print(f"{rel}: {n} words, budget {limit}. Say the rule once, as a rule; history is git's.")
            bad = 1
    return bad


if __name__ == "__main__":
    sys.exit(main())
