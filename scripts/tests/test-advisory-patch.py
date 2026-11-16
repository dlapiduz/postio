#!/usr/bin/env python3
"""Prove `advisory-patch.py` patches what a lockfile can reach and says so.

RUSTSEC-2026-0533 (ammonia) and RUSTSEC-2026-0486 (rustix) were published
between nightly audits, so `main`'s lock carried them while `main` showed
green, and a different PR's Supply chain check found them by accident. Both
had a patched release inside the existing requirements: one `cargo update`
each. This is the offline proof that the script does that and classifies the
advisories it cannot fix.

Nothing here touches the network or the real cargo. A stub `cargo` on PATH
answers `cargo deny ... check advisories` with canned JSON diagnostics (the
shape cargo-deny 0.20 prints on stderr, one object per line) and records
every `cargo update`; the advisory disappears from its output once the
update the script should run has run.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "advisory-patch.py"

STUB = """\
#!{python}
import json, sys
from pathlib import Path

state_path = Path({state!r})
state = json.loads(state_path.read_text())
args = sys.argv[1:]
state["calls"].append(args)
if args[:1] == ["deny"]:
    for adv in state["advisories"]:
        if adv["package"] in state["fixed"]:
            continue
        fields = {{
            "code": adv.get("code", "vulnerability"),
            "severity": "error",
            "message": adv["title"],
            "advisory": {{"id": adv["id"], "package": adv["package"], "title": adv["title"]}},
            "graphs": [{{"Krate": {{"name": adv["package"], "version": adv["version"]}},
                        "parents": [{{"Krate": {{"name": "leaf", "version": "0.1.0"}}}}]}}],
            "notes": ["ID: " + adv["id"], "Solution: " + adv["solution"]],
        }}
        print(json.dumps({{"type": "diagnostic", "fields": fields}}), file=sys.stderr)
    print(json.dumps({{"type": "summary", "fields": {{}}}}), file=sys.stderr)
    state_path.write_text(json.dumps(state))
    sys.exit(1 if any(a["package"] not in state["fixed"] for a in state["advisories"]) else 0)
if args[:1] == ["update"]:
    spec = args[args.index("-p") + 1]
    name = spec.split("@")[0]
    if name in state["blocked"]:
        state_path.write_text(json.dumps(state))
        print("error: failed to select a version", file=sys.stderr)
        sys.exit(101)
    state["fixed"].append(name)
    state_path.write_text(json.dumps(state))
    sys.exit(0)
sys.exit(2)
"""

LEAF = """\
[package]
name = "leaf"
version = "0.1.0"
edition = "2024"

[dependencies]
ammonia = "4"
pinned = "=1.5.0"
"""

ADVISORIES = [
    {
        "id": "RUSTSEC-2026-0533",
        "package": "ammonia",
        "version": "4.2.4",
        "title": "mutation XSS",
        "solution": "Upgrade to >=4.2.5 OR >=4.1.9, <4.2.0 OR >=3.3.5, <4.0.0 "
        "(try `cargo update -p ammonia`)",
    },
    {
        "id": "RUSTSEC-2026-0486",
        "package": "rustix",
        "version": "1.1.5",
        "code": "unsound",
        "title": "xattr unsound",
        "solution": "Upgrade to >=1.1.6 (try `cargo update -p rustix`)",
    },
    {
        "id": "RUSTSEC-2026-9001",
        "package": "pinned",
        "version": "1.5.0",
        "title": "needs a newer major",
        "solution": "Upgrade to >=2.0.0 (try `cargo update -p pinned`)",
    },
    {
        "id": "RUSTSEC-2026-9002",
        "package": "abandoned",
        "version": "0.3.1",
        "title": "no fix",
        "solution": "No safe upgrade is available!",
    },
]


def sandbox(tmp: Path, advisories: list[dict], blocked: list[str]) -> tuple[Path, Path, dict]:
    root = tmp / "tree"
    (root / "scripts").mkdir(parents=True)
    (root / "crates" / "leaf" / "src").mkdir(parents=True)
    (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/leaf"]\nresolver = "3"\n')
    (root / "crates" / "leaf" / "Cargo.toml").write_text(LEAF)
    (root / "crates" / "leaf" / "src" / "lib.rs").write_text("")
    (root / "Cargo.lock").write_text("# fixture\n")
    (root / "deny.toml").write_text("[advisories]\nversion = 2\n")
    shutil.copy(SCRIPT, root / "scripts" / SCRIPT.name)

    bin_dir = tmp / "bin"
    bin_dir.mkdir()
    state = tmp / "state.json"
    state.write_text(
        json.dumps({"advisories": advisories, "fixed": [], "blocked": blocked, "calls": []})
    )
    stub = bin_dir / "cargo"
    stub.write_text(STUB.format(python=sys.executable, state=str(state)))
    stub.chmod(0o755)
    return root, state, {"PATH": f"{bin_dir}:{os.environ['PATH']}"}


def run(root: Path, env: dict) -> subprocess.CompletedProcess[str]:
    return patience.run(
        [sys.executable, str(root / "scripts" / SCRIPT.name)],
        cwd=root,
        env={**os.environ, **env},
        capture_output=True,
        text=True,
        timeout=120,
    )


def updates(state: Path) -> list[list[str]]:
    return [c for c in json.loads(state.read_text())["calls"] if c[:1] == ["update"]]


def fail(msg: str, result: subprocess.CompletedProcess[str] | None = None) -> bool:
    print(f"  FAIL: {msg}", file=sys.stderr)
    if result is not None:
        print(f"  stdout:\n{result.stdout}\n  stderr:\n{result.stderr}", file=sys.stderr)
    return False


def case_patches_within_semver() -> bool:
    """Both real advisories: the lowest reachable patched release, then clean."""
    with tempfile.TemporaryDirectory() as tmp:
        root, state, env = sandbox(Path(tmp), ADVISORIES[:2], [])
        result = run(root, env)
        if result.returncode != 0:
            return fail("a fully patchable lock should exit 0", result)
        got = updates(state)
        want = [
            ["update", "-p", "ammonia@4.2.4", "--precise", "4.2.5"],
            ["update", "-p", "rustix@1.1.5", "--precise", "1.1.6"],
        ]
        if got != want:
            return fail(f"wrong updates: {got}", result)
        if result.stdout.count("patched within semver") != 2:
            return fail("report should say 'patched within semver' per advisory", result)
        if "RUSTSEC-2026-0533" not in result.stdout:
            return fail("report should name the advisory", result)
    return True


def case_manifest_bump_names_the_line() -> bool:
    """A requirement that forbids the fix is reported with its manifest line."""
    with tempfile.TemporaryDirectory() as tmp:
        root, state, env = sandbox(Path(tmp), [ADVISORIES[2]], ["pinned"])
        before = (root / "crates" / "leaf" / "Cargo.toml").read_text()
        result = run(root, env)
        if result.returncode == 0:
            return fail("an unpatched advisory must not exit 0", result)
        if "needs a manifest bump" not in result.stdout:
            return fail("report should say 'needs a manifest bump'", result)
        if "crates/leaf/Cargo.toml:8" not in result.stdout:
            return fail("report should name the manifest line (crates/leaf/Cargo.toml:8)", result)
        if (root / "crates" / "leaf" / "Cargo.toml").read_text() != before:
            return fail("the script edited a manifest")
        if any("--precise" not in c for c in updates(state)[:1]):
            return fail("should try the precise update first", result)
    return True


def case_no_patched_version() -> bool:
    """No fix exists: say so, try nothing."""
    with tempfile.TemporaryDirectory() as tmp:
        root, state, env = sandbox(Path(tmp), [ADVISORIES[3]], [])
        result = run(root, env)
        if result.returncode == 0:
            return fail("an unpatchable advisory must not exit 0", result)
        if "no patched version" not in result.stdout:
            return fail("report should say 'no patched version'", result)
        if updates(state):
            return fail(f"nothing to update, yet: {updates(state)}", result)
    return True


def case_mixed_run_patches_what_it_can() -> bool:
    """One unfixable advisory does not stop the others being patched."""
    with tempfile.TemporaryDirectory() as tmp:
        root, state, env = sandbox(Path(tmp), ADVISORIES, ["pinned"])
        result = run(root, env)
        if result.returncode == 0:
            return fail("leftovers must exit non-zero", result)
        fixed = json.loads(state.read_text())["fixed"]
        if sorted(fixed) != ["ammonia", "rustix"]:
            return fail(f"patched {fixed}", result)
        for phrase in ("patched within semver", "needs a manifest bump", "no patched version"):
            if phrase not in result.stdout:
                return fail(f"report lacks '{phrase}'", result)
    return True


def case_clean_audit_does_nothing() -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        root, state, env = sandbox(Path(tmp), [], [])
        result = run(root, env)
        if result.returncode != 0 or updates(state):
            return fail("a clean audit should exit 0 and run no update", result)
    return True


def case_never_pushes_or_edits_the_lock_itself() -> bool:
    """The script's only effects are `cargo update` calls: no git, no gh."""
    text = SCRIPT.read_text()
    for word in ("git push", "gh pr", '"git"', '"gh"'):
        if word in text:
            print(f"  FAIL: script mentions {word!r}", file=sys.stderr)
            return False
    return True


def main() -> int:
    cases = [
        ("patches within semver, lowest patched release", case_patches_within_semver),
        ("a manifest bump names its line", case_manifest_bump_names_the_line),
        ("no patched version is reported, not attempted", case_no_patched_version),
        ("a mixed run patches what it can", case_mixed_run_patches_what_it_can),
        ("a clean audit does nothing", case_clean_audit_does_nothing),
        ("it neither pushes nor opens PRs", case_never_pushes_or_edits_the_lock_itself),
    ]
    failed = 0
    for name, case in cases:
        print(f"case: {name}")
        if not case():
            failed += 1
    if failed:
        print(f"\n{failed} case(s) failed.", file=sys.stderr)
        return 1
    print("test-advisory-patch: all cases passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
