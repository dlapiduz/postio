#!/usr/bin/env python3
"""Self-test for `issue-land.sh --closes` and the flags it no longer takes.

Usage: scripts/tests/test-issue-land-closes.py
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("refs_only", HERE / "test-issue-land-refs-only.py")
base_test = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base_test)

FAILURES: list[str] = []
SANDBOXES = base_test.SANDBOXES


def expect(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def land(base: Path, branch: str, args: list[str]) -> tuple[subprocess.CompletedProcess[str], list[str], Path]:
    root, origin, stub_dir = base / "repo", base / "origin.git", base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    gh = stub_dir / "bin" / "gh"
    gh.write_text(base_test.GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub_dir / "calls").write_text("", encoding="utf-8")
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    root.mkdir()
    base_test.build_sandbox(root, base_test.pinned_channel())
    git = base_test.git
    git("init", "-q", "-b", "main", cwd=root)
    git("config", "user.email", "test@example.com", cwd=root)
    git("config", "user.name", "Test", cwd=root)
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)
    git("checkout", "-q", "-b", branch, cwd=root)
    (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")
    result = base_test.land(root, base / "target", stub_dir, args)
    calls = [c.decode() for c in (stub_dir / "calls").read_bytes().split(b"\0") if c]
    return result, calls, origin


def body_of(calls: list[str]) -> str:
    created = [c for c in calls if c.startswith("pr create")]
    return created[0] if len(created) == 1 else ""


def pushed(origin: Path, branch: str) -> bool:
    out = subprocess.run(["git", "--git-dir", str(origin), "branch", "--list", branch],
                         capture_output=True, text=True, check=True).stdout
    return bool(out.strip())


def main() -> int:
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        base = Path(directory)

        result, calls, _ = land(base / "batch", "issue-1189-batch", ["--closes", "12,34"])
        body = body_of(calls)
        output = result.stdout + result.stderr
        expect("a batch lands", result.returncode == 0, output)
        expect("its PR closes its own issue and every --closes one",
               all(f"Closes #{n}" in body for n in (1189, 12, 34)), f"body:\n{body}")
        expect("the gate chain's budget is five minutes", "(budget 300s)" in output, output)

        result, calls, _ = land(base / "small", "chore/batch", ["--closes", "56"])
        body = body_of(calls)
        expect("a chore branch's PR closes what --closes names",
               result.returncode == 0 and "Closes #56" in body, f"{result.stdout}{result.stderr}\n{body}")
        expect("and does not also say it closes nothing", body and "No issue" not in body, f"body:\n{body}")

        refused = [
            ("--closes with something that is not an issue number", ["--closes", "12,abc"], "abc"),
            ("--full-suite", ["--full-suite"], "main"),
            ("--full", ["--full"], "main"),
        ]
        for index, (name, args, says) in enumerate(refused):
            result, calls, origin = land(base / f"refused-{index}", "issue-1189-refused", args)
            output = result.stdout + result.stderr
            expect(f"{name} is refused before anything is pushed",
                   result.returncode == 2 and not body_of(calls) and not pushed(origin, "issue-1189-refused"),
                   f"exit {result.returncode}:\n{output}\n{calls}")
            expect(f"{name}'s refusal says why", says in output and "--- rustfmt ---" not in output, output)

    for failure in FAILURES:
        print(f"FAIL: {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land --closes self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
