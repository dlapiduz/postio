#!/usr/bin/env python3
"""Patch the security advisories a lockfile can reach, and report the rest.

An advisory is published on any day, not on a day anybody pushed. The PR
gate deliberately does not run `advisories` (see
`checks/check-dependency-policy.py`), and the nightly audit finds them a day
later, so `main` can carry one while it shows green: RUSTSEC-2026-0533
(ammonia) and RUSTSEC-2026-0486 (rustix) were found by accident by another
PR's Supply chain check. Both had a patched release inside the existing
requirements, which is one `cargo update` each. This does that.

For every `vulnerability` or `unsound` advisory in
`cargo deny --format json check advisories`:

  * a patched release the lock can reach: runs
    `cargo update -p <crate>@<locked> --precise <lowest such release>`;
  * a patched release only a new requirement allows: reports it as needing
    a manifest bump and names the manifest line. It never edits a manifest;
  * no patched release: reports that.

Then it re-runs the audit and says what is left. It changes `Cargo.lock` in
the working tree and nothing else: no commit, no push, no PR. The caller
lands the result (`/steward` says how).

Exit status: 0 the audit is clean, 1 something remains (the report says
what), 2 cargo-deny could not be run or read, 3 an advisory's patched
versions could not be read from cargo-deny's output (the format changed;
reported as such, never as "no patched version").

The patched ranges come from the advisory's `patched_versions` or
`versions.patched` field when cargo-deny prints one. 0.20.2 does not, so
they are otherwise parsed from the diagnostic's
`Solution: Upgrade to >=A OR >=B, <C (try ...)` note; `Solution: No safe
upgrade is available!` is the only statement taken to mean there is no fix. Install the pinned tool with
`scripts/install-cargo-deny.sh`.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# `unmaintained` and `yanked` are not fixed by a patched version; they are left to the audit.
FIXABLE = {"vulnerability", "unsound"}

Version = tuple[int, int, int]


@dataclass
class Advisory:
    id: str
    crate: str
    version: str
    code: str
    title: str
    solution: str
    structured: list[str] | None = None
    parents: list[str] = field(default_factory=list)


def parse_version(text: str) -> Version | None:
    m = re.match(r"(\d+)(?:\.(\d+))?(?:\.(\d+))?", text.strip())
    if not m:
        return None
    return (int(m[1]), int(m[2] or 0), int(m[3] or 0))


def fmt(v: Version) -> str:
    return ".".join(map(str, v))


def compatible(locked: Version, candidate: Version) -> bool:
    """Cargo's caret rule: the leftmost non-zero component must match."""
    if locked[0] != 0:
        return candidate[0] == locked[0]
    if locked[1] != 0:
        return candidate[:2] == locked[:2]
    return candidate == locked


def patched_lower_bounds(adv: Advisory) -> list[Version] | None:
    """Lower bounds of the patched ranges; None when no range is named.

    A range with an upper bound at or below nothing useful (a backport line,
    `>=4.1.9, <4.2.0`) still contributes its lower bound; the caller keeps
    only those above the locked version.
    """
    if adv.structured is not None:
        ranges = adv.structured
    else:
        m = re.search(r"Upgrade to (.+?)(?: \(try|$)", adv.solution)
        if not m:
            return None
        ranges = m[1].split(" OR ")
    bounds: list[Version] = []
    for rng in ranges:
        for comparator in rng.split(","):
            c = re.match(r"\s*(>=|>|=)?\s*(\d[\w.\-+]*)\s*$", comparator)
            if c and c[1] in (">=", "="):
                v = parse_version(c[2])
                if v:
                    bounds.append(v)
    return bounds or None


def says_no_fix(adv: Advisory) -> bool:
    """cargo-deny's explicit statement that nothing is patched."""
    if adv.structured is not None:
        return not adv.structured
    return "No safe upgrade is available" in adv.solution


def lowest_reachable(adv: Advisory) -> tuple[Version | None, Version | None]:
    """(lowest patched release above the lock inside its semver series,
    lowest patched release above the lock in any series)."""
    locked = parse_version(adv.version)
    bounds = patched_lower_bounds(adv)
    if locked is None or not bounds:
        return None, None
    above = sorted(b for b in bounds if b > locked)
    within = [b for b in above if compatible(locked, b)]
    return (within[0] if within else None), (above[0] if above else None)


def structured_patched(advisory: dict) -> list[str] | None:
    for value in (advisory.get("patched_versions"), (advisory.get("versions") or {}).get("patched")):
        if isinstance(value, list):
            return [str(v) for v in value]
    return None


def audit() -> tuple[list[Advisory], list[str]]:
    """Run the audit; (fixable advisories, ids of other failures)."""
    try:
        result = subprocess.run(
            ["cargo", "deny", "--format", "json", "check", "advisories"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=600,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"advisory-patch: could not run cargo-deny: {error}", file=sys.stderr)
        raise SystemExit(2)

    found: dict[tuple[str, str, str], Advisory] = {}
    other: list[str] = []
    saw_json = False
    for line in result.stderr.splitlines():
        try:
            d = json.loads(line)
        except ValueError:
            continue
        saw_json = True
        fields = d.get("fields", {})
        advisory = fields.get("advisory")
        if d.get("type") != "diagnostic" and "code" not in fields:
            continue
        if not advisory or fields.get("severity") != "error":
            continue
        graph = (fields.get("graphs") or [{}])[0]
        krate = graph.get("Krate", {})
        code = fields.get("code", "")
        solution = next(
            (n for n in fields.get("notes", []) if n.startswith("Solution:")), ""
        )
        adv = Advisory(
            id=advisory["id"],
            crate=advisory.get("package", krate.get("name", "?")),
            version=krate.get("version", "0.0.0"),
            code=code,
            title=advisory.get("title", ""),
            solution=solution,
            structured=structured_patched(advisory),
            parents=[p["Krate"]["name"] for p in graph.get("parents", []) if "Krate" in p],
        )
        if code in FIXABLE:
            found[(adv.id, adv.crate, adv.version)] = adv
        else:
            other.append(f"{adv.id} {adv.crate} {adv.version} ({code})")
    if result.returncode != 0 and not saw_json:
        print(
            f"advisory-patch: cargo-deny failed without a report:\n{result.stderr}",
            file=sys.stderr,
        )
        raise SystemExit(2)
    return list(found.values()), other


def manifest_lines(crate: str) -> list[str]:
    """`path:line` of every workspace manifest line that declares `crate`."""
    hits: list[str] = []
    pattern = re.compile(
        rf'^\s*(?:{re.escape(crate)}\s*(?:=|\.)|\[(?:[\w\-.]*\.)?dependencies\.{re.escape(crate)}\])'
    )
    for manifest in sorted(ROOT.rglob("Cargo.toml")):
        rel = manifest.relative_to(ROOT)
        if "target" in rel.parts or "vendor" in rel.parts:
            continue
        for n, text in enumerate(manifest.read_text().splitlines(), 1):
            if pattern.match(text):
                hits.append(f"{rel}:{n}")
    return hits


def update(adv: Advisory, precise: Version | None) -> bool:
    spec = f"{adv.crate}@{adv.version}"
    attempts = []
    if precise:
        attempts.append(["cargo", "update", "-p", spec, "--precise", fmt(precise)])
    attempts.append(["cargo", "update", "-p", spec])
    for cmd in attempts:
        done = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        if done.returncode == 0:
            return True
    return False


def main() -> int:
    advisories, other = audit()
    if not advisories and not other:
        print("advisory-patch: the advisories audit is clean; nothing to do.")
        return 0

    verdict: dict[tuple[str, str, str], str] = {}
    for adv in advisories:
        within, anywhere = lowest_reachable(adv)
        key = (adv.id, adv.crate, adv.version)
        if patched_lower_bounds(adv) is None:
            verdict[key] = "no patched version" if says_no_fix(adv) else "unreadable"
        elif anywhere is None:
            verdict[key] = "no patched version"
        elif within is None:
            verdict[key] = "needs a manifest bump"
        else:
            update(adv, within)
            verdict[key] = "patched within semver"

    remaining, other_after = audit()
    still = {(a.id, a.crate, a.version): a for a in remaining}

    print("advisory-patch report")
    for adv in advisories:
        key = (adv.id, adv.crate, adv.version)
        status = verdict[key]
        within, anywhere = lowest_reachable(adv)
        head = f"  {adv.id}  {adv.crate} {adv.version}: "
        if status == "patched within semver" and key in still:
            status = "needs a manifest bump"
        if status == "patched within semver":
            print(head + "patched within semver")
        elif status == "needs a manifest bump":
            target = anywhere or within
            want = f">= {fmt(target)}" if target else adv.solution
            lines = manifest_lines(adv.crate)
            where = (
                "edit " + ", ".join(lines)
                if lines
                else "transitive via " + (", ".join(adv.parents) or "?")
                + "; bump the direct dependency that requires it"
            )
            print(head + f"needs a manifest bump to {want} ({where})")
        elif status == "unreadable":
            print(head + "could not read the patched versions from cargo-deny's "
                  "output (format changed?); fix scripts/advisory-patch.py or read the advisory")
        else:
            print(head + "no patched version" + (f" ({adv.title})" if adv.title else ""))
    for note in other_after or other:
        print(f"  {note}: not fixable by updating; left to the audit")

    if remaining or other_after:
        print("advisory-patch: the audit is still red; the items above need a person.")
        if "unreadable" in verdict.values() and any(
            (a.id, a.crate, a.version) in still and verdict[(a.id, a.crate, a.version)] == "unreadable"
            for a in advisories
        ):
            return 3
        return 1
    print("advisory-patch: the audit is clean. Cargo.lock is changed in the working tree.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
