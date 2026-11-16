#!/usr/bin/env python3
"""Self-test: a diff that changes the build environment lands as `--full-suite`.

A Rust toolchain bump passed every local gate and the sanity tier; only the
full suite caught Cargo moving test binaries out of `target/<profile>/deps/`,
and only because the author happened to ask for it. `scripts/risk-of-diff.sh`
now names such a diff (`rust-toolchain.toml`, `mise.toml`,
`.cargo/config.toml`, `flatpak/*.json`) and `issue-land.sh` adds the
`full-suite` label through the same path `--full-suite` uses, and says so.
Any other risky diff is still only named, never labelled.

Usage: scripts/tests/test-issue-land-build-env.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import importlib.util
import shutil
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


def run_case(name: str, path: str, text: str, *, labelled: bool, announced: bool) -> None:
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
        shutil.copy(base_test.HERE / "risk-of-diff.sh", root / "scripts" / "risk-of-diff.sh")
        (root / "scripts" / "risk-of-diff.sh").chmod(0o755)
        git = base_test.git
        git("init", "-q", "-b", "main", cwd=root)
        # The landing commits under GIT_CONFIG_GLOBAL=/dev/null, so a runner
        # with no global identity needs one in the repository itself.
        git("config", "user.email", "test@example.com", cwd=root)
        git("config", "user.name", "Test", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "chore/build-env-check", cwd=root)
        target = root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

        result = base_test.land(root, base / "target", stub_dir, [], "chore(build): change " + path)
        calls = [c.decode() for c in (stub_dir / "calls").read_bytes().split(b"\0") if c]
        output = result.stdout + result.stderr
        if result.returncode != 0:
            FAILURES.append(f"{name}: landing failed:\n{output}")
            return
        got = [c for c in calls if c.startswith("pr edit") and "--add-label full-suite" in c]
        if (len(got) == 1) != labelled:
            FAILURES.append(f"{name}: labelled={len(got)} expected {labelled}; calls {calls}")
        if ("full suite requested" in output) != announced:
            FAILURES.append(f"{name}: announcement mismatch (want {announced}):\n{output}")
        if announced and path not in output:
            FAILURES.append(f"{name}: output does not say why ({path}):\n{output}")


def main() -> int:
    for path, text in [
        ("mise.toml", "[tools]\n"),
        (".cargo/config.toml", "[build]\n"),
        ("flatpak/x.json", '{"runtime-version": "1"}\n'),
    ]:
        run_case(f"{path} labels full-suite", path, text, labelled=True, announced=True)
    run_case("an ordinary file does not", "dummy/src/extra.rs", "// x\n", labelled=False, announced=False)
    for failure in FAILURES:
        print(f"FAIL: {failure}")
    if FAILURES:
        return 1
    print("issue-land build-env self-test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
