#!/usr/bin/env python3
"""Self-test for the pinned cargo tools' install scripts and their shim.

Many sessions share one machine, each in a worktree on a different commit.
When the install scripts replaced ~/.cargo/bin/<tool>, a branch that bumped a
pin and a session on main's pin reinstalled each other's version forever. The
cases pin the replacement: versions install side by side, the plain
`cargo-<tool>` resolves the pin of the checkout it runs in, a missing version
installs on first use, a second run downloads nothing, the checksum is still
checked against the pinned digest, and CI keeps the plain install.

Everything runs in a sandbox HOME with stub curl, sha256sum and cargo.

Usage: scripts/tests/test-install-pinned-tools.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402

SCRIPTS = Path(__file__).resolve().parent.parent
FAILURES: list[str] = []

TOOL = '#!/bin/sh\necho "{name} {version}"\necho "$PWD ran {name} {version} $*" >> "$TOOL_LOG"\n'

CURL = """#!/usr/bin/env bash
echo "curl $*" >> "$TOOL_LOG"
out=""; url=""
while [ $# -gt 0 ]; do
    case "$1" in -o) out="$2"; shift ;; http*) url="$1" ;; esac
    shift
done
work=$(mktemp -d)
case "$url" in
*nexte.st/*)
    tool=cargo-nextest; dir=""
    version=$(echo "$url" | sed 's#.*nexte.st/\\([^/]*\\)/.*#\\1#') ;;
*)
    dir=$(basename "$url" .tar.gz)
    tool=$(echo "$dir" | grep -oE '^cargo-[a-z]+')
    version=$(echo "$dir" | grep -oE '[0-9]+[.][0-9]+[.][0-9]+' | head -1) ;;
esac
printf '#!/bin/sh\\necho "%s %s"\\n' "$tool" "$version" > "$work/$tool"
chmod +x "$work/$tool"
members="$tool"
if [ -n "$dir" ]; then mkdir -p "$work/$dir"; cp "$work/$tool" "$work/$dir/"; members="$members $dir/$tool"; fi
tar -czf "$out" -C "$work" $members
"""

SUM = """#!/usr/bin/env bash
read -r line
echo "sum $line" >> "$TOOL_LOG"
case "$line" in "$EXPECT_DIGEST  "*) exit 0 ;; *) echo "checksum mismatch" >&2; exit 1 ;; esac
"""

CARGO = """#!/usr/bin/env bash
echo "cargo $*" >> "$TOOL_LOG"
[ "$1" = install ] || exit 0
name="$2"; shift 2; version=""; root="$HOME/.cargo"
while [ $# -gt 0 ]; do
    case "$1" in --version) version="$2"; shift ;; --root) root="$2"; shift ;; esac
    shift
done
mkdir -p "$root/bin"
printf '#!/bin/sh\\necho "%s %s"\\n' "$name" "$version" > "$root/bin/$name"
chmod +x "$root/bin/$name"
"""


def case(name: str, ok: bool, detail: str = "") -> None:
    if ok:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


class Sandbox:
    def __init__(self, base: Path) -> None:
        self.base = base
        self.home = base / "home"
        self.stubs = base / "stubs"
        self.log = base / "log"
        self.home.mkdir()
        self.stubs.mkdir()
        self.log.touch()
        for name, body in (("curl", CURL), ("sha256sum", SUM), ("shasum", SUM), ("cargo", CARGO)):
            (self.stubs / name).write_text(body)
            (self.stubs / name).chmod(0o755)

    def repo(self, name: str, **versions: str) -> Path:
        """A fake checkout whose install scripts pin the given versions."""
        root = self.base / name
        (root / "scripts" / "lib").mkdir(parents=True)
        for f in SCRIPTS.glob("install-*.sh"):
            text = f.read_text()
            tool = f.stem.removeprefix("install-")
            if tool in versions:
                text = re.sub(r'^VERSION="[^"]*"', f'VERSION="{versions[tool]}"', text, count=1, flags=re.M)
            (root / "scripts" / f.name).write_text(text)
            (root / "scripts" / f.name).chmod(0o755)
        for f in (SCRIPTS / "lib").glob("*.sh"):
            shutil.copy(f, root / "scripts" / "lib" / f.name)
        return root

    def env(self, **extra: str) -> dict[str, str]:
        env = {
            "PATH": f"{self.home}/.cargo/bin:{self.stubs}:/usr/bin:/bin",
            "HOME": str(self.home),
            "TOOL_LOG": str(self.log),
            "TMPDIR": str(self.base),
        }
        env.update(extra)
        return env

    def run(self, argv: list[str], cwd: Path, **extra: str) -> subprocess.CompletedProcess[str]:
        return patience.run(argv, cwd=cwd, env=self.env(**extra), capture_output=True, text=True, timeout=60)

    def lines(self, prefix: str) -> list[str]:
        return [x for x in self.log.read_text().splitlines() if x.startswith(prefix)]


def digest_for_this_platform(script: Path) -> str:
    key = "SHA256_MAC" if sys.platform == "darwin" else "SHA256_LINUX"
    return re.search(rf'^{key}="([0-9a-f]+)"', script.read_text(), re.M).group(1)


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        sb = Sandbox(Path(raw))
        old = sb.repo("old", nextest="0.9.143")
        new = sb.repo("new", nextest="0.9.152")
        digest = digest_for_this_platform(old / "scripts" / "install-nextest.sh")
        store = sb.home / ".local/share/postio-tools/cargo-nextest"
        shim = sb.home / ".cargo/bin/cargo-nextest"

        for repo in (old, new):
            r = sb.run([str(repo / "scripts/install-nextest.sh")], repo, EXPECT_DIGEST=digest)
            case(f"install from {repo.name}", r.returncode == 0, r.stderr)
        case("both versions kept", (store / "0.9.143/bin/cargo-nextest").exists()
             and (store / "0.9.152/bin/cargo-nextest").exists(), str(list(store.glob("*"))))
        case("plain name is the shim, not a tool", shim.exists() and shim.read_text() == (SCRIPTS / "lib/cargo-tool-shim.sh").read_text(), "")
        for repo, want in ((old, "0.9.143"), (new, "0.9.152")):
            sub = repo / "scripts"
            r = sb.run(["cargo-nextest", "nextest"], sub, EXPECT_DIGEST=digest)
            case(f"shim in {repo.name}/scripts runs {want}", r.stdout.strip() == f"cargo-nextest {want}",
                 r.stdout + r.stderr)

        sums = sb.lines("sum ")
        case("checksum verified against the pin", sums and all(s.startswith(f"sum {digest}") for s in sums), str(sums))
        bad = sb.repo("bad", nextest="0.9.100")
        r = sb.run([str(bad / "scripts/install-nextest.sh")], bad, EXPECT_DIGEST="0" * 64)
        case("a wrong digest installs nothing", r.returncode != 0
             and not (store / "0.9.100").exists(), r.stdout + r.stderr)
        case("a failed install leaves no staging directory",
             not list(store.glob(".stage-*")), str(list(store.glob("*"))))

        before = len(sb.lines("curl"))
        r = sb.run([str(new / "scripts/install-nextest.sh")], new, EXPECT_DIGEST=digest)
        case("second run downloads nothing", r.returncode == 0 and len(sb.lines("curl")) == before
             and "already installed" in r.stdout, r.stdout + r.stderr)

        fresh = sb.repo("fresh", nextest="0.9.200")
        r = sb.run(["cargo-nextest", "nextest"], fresh, EXPECT_DIGEST=digest)
        case("shim installs a missing pin, then runs it",
             r.returncode == 0 and r.stdout.strip() == "cargo-nextest 0.9.200", r.stdout + r.stderr)

        r = sb.run(["cargo-nextest", "nextest"], sb.home)
        case("outside a checkout the newest installed runs", r.stdout.strip() == "cargo-nextest 0.9.200",
             r.stdout + r.stderr)

        for tool, name in (("hakari", "cargo-hakari"), ("machete", "cargo-machete"), ("cargo-deny", "cargo-deny")):
            a = sb.repo(f"{tool}-a", **{tool: "1.0.0"})
            b = sb.repo(f"{tool}-b", **{tool: "2.0.0"})
            for repo in (a, b):
                pin = re.search(r"\b[0-9a-f]{64}\b", (repo / f"scripts/install-{tool}.sh").read_text())
                r = sb.run([str(repo / f"scripts/install-{tool}.sh")], repo, EXPECT_DIGEST=pin.group(0))
                case(f"{name} install from {repo.name}", r.returncode == 0, r.stdout + r.stderr)
            for repo, want in ((a, "1.0.0"), (b, "2.0.0")):
                r = sb.run([name, "--version"], repo)
                case(f"{name} resolves {want} in {repo.name}", r.stdout.strip() == f"{name} {want}",
                     r.stdout + r.stderr)
        case("no pinned tool compiles from source", not sb.lines("cargo install"), str(sb.lines("cargo install")))

        ci_home = Path(raw) / "ci-home"
        ci_home.mkdir()
        r = patience.run([str(new / "scripts/install-nextest.sh")], cwd=new,
                         env={**sb.env(EXPECT_DIGEST=digest, GITHUB_ACTIONS="true"), "HOME": str(ci_home)},
                         capture_output=True, text=True, timeout=60)
        binary = ci_home / ".cargo/bin/cargo-nextest"
        case("CI installs the plain binary", r.returncode == 0 and binary.exists()
             and "0.9.152" in binary.read_text() and not (ci_home / ".local").exists(), r.stdout + r.stderr)

    for f in FAILURES:
        print(f"FAIL  {f}", file=sys.stderr)
    return 1 if FAILURES else 0


if __name__ == "__main__":
    sys.exit(main())
