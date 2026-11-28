#!/usr/bin/env python3
"""Self-test: a landing that outlives its watchdog stops, names its phase, and leaves no gate running.

Usage: scripts/tests/test-issue-land-watchdog.py
"""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("refs_only", HERE / "test-issue-land-refs-only.py")
base_test = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base_test)

FAILURES: list[str] = []
SANDBOXES = base_test.SANDBOXES

HANGING_CHECK = """#!/usr/bin/env python3
import os, pathlib, time
pathlib.Path(os.environ["HANG_PID_FILE"]).write_text(str(os.getpid()))
time.sleep(600)
"""


def expect(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    # A zombie is dead for this purpose: it holds no CPU and no lock.
    stat = Path(f"/proc/{pid}/stat")
    return not (stat.exists() and stat.read_text().split(")")[-1].split()[0] == "Z")


def main() -> int:
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        base = Path(directory)
        root, origin, stub_dir = base / "repo", base / "origin.git", base / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        gh = stub_dir / "bin" / "gh"
        gh.write_text(base_test.GH_STUB, encoding="utf-8")
        gh.chmod(0o755)
        (stub_dir / "calls").write_text("", encoding="utf-8")
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
        root.mkdir()
        base_test.build_sandbox(root, base_test.pinned_channel())
        hang = root / "scripts" / "checks" / "check-hangs.py"
        hang.write_text(HANGING_CHECK, encoding="utf-8")
        hang.chmod(0o755)
        git = base_test.git
        git("init", "-q", "-b", "main", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "issue-7-watchdog", cwd=root)
        (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")

        pid_file = base / "hang.pid"
        os.environ["HANG_PID_FILE"] = str(pid_file)
        os.environ["POSTIO_LAND_WATCHDOG_SECONDS"] = "3"
        started = time.monotonic()
        try:
            result = base_test.land(root, base / "target", stub_dir, [])
        except Exception as hung_landing:  # noqa: BLE001 -- ScriptHung is the red this guards
            if pid_file.exists():
                os.kill(int(pid_file.read_text()), 9)
            print(f"FAIL  the landing stops: {hung_landing}", file=sys.stderr)
            return 1
        took = time.monotonic() - started
        output = result.stdout + result.stderr
        calls = [c.decode() for c in (stub_dir / "calls").read_bytes().split(b"\0") if c]

        expect("the landing stops", result.returncode != 0, f"exit {result.returncode}:\n{output}")
        expect("soon after its watchdog fires", took < 45, f"took {took:.0f}s:\n{output}")
        expect("and says which phase it was in",
               "watchdog" in output and "invariants" in output, output)
        expect("nothing was pushed or opened", not [c for c in calls if c.startswith("pr ")], f"{calls}")
        hung = int(pid_file.read_text()) if pid_file.exists() else 0
        if hung:
            for _ in range(20):
                if not alive(hung):
                    break
                time.sleep(0.25)
        expect("the gate it was waiting on is gone", hung and not alive(hung),
               f"the hanging check ({hung or 'never started'}) still runs")
        if hung and alive(hung):
            os.kill(hung, 9)

    for failure in FAILURES:
        print(f"FAIL: {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land watchdog self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
