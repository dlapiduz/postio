#!/usr/bin/env python3
"""Prove the format-after-edit hook formats the file a session edited in its
own worktree, says so, and touches nothing else."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HOOK = str(Path(__file__).resolve().parent / "format-rust.py")
FORMATTED = "fn main() {}\n"
UNFORMATTED = "fn  main(){}\n"
failures = 0


def run(path: Path, root: Path, tool: str = "Edit") -> subprocess.CompletedProcess[str]:
    payload = {"hook_event_name": "PostToolUse", "tool_name": tool,
               "tool_input": {"file_path": str(path)}, "cwd": str(root)}
    env = {k: v for k, v in os.environ.items() if k != "RUSTUP_TOOLCHAIN"}
    env["POSTIO_WORKTREES"] = str(root / "postio-worktrees")
    return subprocess.run([sys.executable, HOOK], input=json.dumps(payload),
                          capture_output=True, text=True, env=env, timeout=60)


def case(label: str, ok: bool, result: subprocess.CompletedProcess[str]) -> None:
    global failures
    if ok:
        print(f"  ok   {label}")
    else:
        failures += 1
        print(f"  FAIL {label}\n    exit {result.returncode} stdout {result.stdout!r} "
              f"stderr {result.stderr!r}")


def write(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    return path


if shutil.which("rustfmt") is None:
    print("rustfmt is not installed here; skipping")
    sys.exit(77)

with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    tree = root / "postio-worktrees" / "issue-7-x"

    f = write(tree / "crates/postio-core/src/lib.rs", UNFORMATTED)
    r = run(f, root)
    told = r.stdout and "lib.rs" in json.loads(r.stdout)["hookSpecificOutput"]["additionalContext"]
    case("an edited file in a worktree is formatted", f.read_text() == FORMATTED, r)
    case("and the session is told, so it re-reads before the next edit",
         r.returncode == 0 and bool(told), r)

    r = run(f, root, tool="Write")
    case("a formatted file is left alone, silently",
         f.read_text() == FORMATTED and r.returncode == 0 and r.stdout == "", r)

    f = write(tree / "crates/postio-core/src/lib.rs", "mod child;\nfn  main(){}\n")
    child = write(tree / "crates/postio-core/src/child.rs", UNFORMATTED)
    r = run(f, root)
    case("the file's child modules are not touched", child.read_text() == UNFORMATTED, r)

    f = write(tree / "crates/postio-core/src/broken.rs", "fn main( {}\n")
    r = run(f, root)
    case("a file mid-edit that does not parse is left as it is",
         f.read_text() == "fn main( {}\n" and r.returncode == 0 and r.stdout == "", r)

    for label, path in (
        ("the shared checkout is not formatted", root / "postio/crates/x/src/lib.rs"),
        ("target/ is not formatted", tree / "target/debug/build/x/out/gen.rs"),
        ("vendor/ is not formatted", tree / "vendor/x/src/lib.rs"),
        ("a file that is not Rust is not formatted", tree / "docs/x.md"),
    ):
        f = write(path, UNFORMATTED)
        r = run(f, root)
        case(label, f.read_text() == UNFORMATTED and r.returncode == 0 and r.stdout == "", r)

    r = run(tree / "gone.rs", root)
    case("a file that is not there is not an error", r.returncode == 0 and r.stdout == "", r)

print(f"\n{'all passed' if not failures else f'{failures} failed'}")
sys.exit(1 if failures else 0)
