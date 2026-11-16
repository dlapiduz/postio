#!/usr/bin/env python3
"""Self-test for scripts/deps-survey.py.

The survey asks crates.io, GitHub, rust-lang.org, endoflife.date and
Flathub what is newest. This builds a miniature repository and a directory
of canned answers (`--offline`) and proves it sorts what is behind the way
the maintainer would, and that a source it could not reach is reported as
unknown and never as current. Nothing here touches the network.

Usage: scripts/tests/test-deps-survey.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "deps-survey.py"
FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str = "") -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def load():
    spec = importlib.util.spec_from_file_location("deps_survey", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def crate_json(latest: str) -> str:
    return json.dumps({"crate": {"max_stable_version": latest, "max_version": latest + "-rc.1"}})


def build(root: Path, fx: Path, survey) -> None:
    write(root / "Cargo.toml", """\
[workspace]
members = ["crates/a", "crates/postio-workspace-hack"]
[workspace.dependencies]
postio-a = { path = "crates/a" }
serde = "1.0.200"
turso = { version = "=0.8.3", default-features = false }
io-imap = "0.7"
pre = "=0.3.0-beta.4"
io-smtp = "0.6"
""")
    write(root / "crates/a/Cargo.toml", """\
[package]
name = "a"
[dependencies]
serde.workspace = true
turso = { workspace = true }
tokio = "1.40"
bigdep = "1.2"
fresh = "2"
ghost = "3"
gtk = { version = "0.11.4", package = "gtk4" }
local = { path = "../b", version = "0.1" }
io-imap = "0.7"
[dev-dependencies]
tempfile = "~3.10.0"
[build-dependencies]
cc = "1"
""")
    write(root / "crates/postio-workspace-hack/Cargo.toml", """\
[package]
name = "postio-workspace-hack"
[dependencies]
hackonly = "1"
""")
    write(root / "scripts/deps-groups.toml", """\
[[group]]
name = "pimalaya"
members = ["io-imap", "io-smtp"]
waits_on = "the whole stack moving together"
""")
    write(root / "rust-toolchain.toml", '[toolchain]\nchannel = "1.99.1"\n')
    write(root / "mise.toml", '[tools]\npython = "3.14.7"\ngh = "2.98.0"\njq = "1.8.1"\nsccache = "0.17.0"\n')
    write(root / "scripts/install-hakari.sh", 'VERSION="0.9.39"\n')
    write(root / "scripts/install-nextest.sh", 'set -e\nVERSION="0.9.143"\nSHA256="x"\n')
    write(root / ".github/workflows/ci.yml", """\
jobs:
  a:
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
      - uses: ./.github/workflows/nightly.yml
""")
    write(root / "flatpak/app.json", json.dumps({"runtime": "org.gnome.Platform", "runtime-version": "50"}))
    write(root / "flatpak/tui.json", json.dumps({"runtime": "org.freedesktop.Platform", "runtime-version": "25.08"}))
    write(root / "patches/blitz/upstream.toml", 'version = "0.3.0-beta.4"\n')

    f = survey.fixture_name
    web = lambda u: f("https://" + u)
    write(fx / web("crates.io/api/v1/crates/serde"), crate_json("1.0.231"))  # within, workspace
    write(fx / web("crates.io/api/v1/crates/turso"), crate_json("0.8.5"))  # pinned, within
    write(fx / web("crates.io/api/v1/crates/tokio"), crate_json("1.41.0"))  # current
    write(fx / web("crates.io/api/v1/crates/bigdep"), crate_json("2.0.1"))  # major
    write(fx / web("crates.io/api/v1/crates/fresh"), crate_json("2.3.0"))  # current
    # ghost: no fixture at all -> unknown
    write(fx / web("crates.io/api/v1/crates/gtk4"), crate_json("0.12.0"))  # package rename, major
    write(fx / web("crates.io/api/v1/crates/tempfile"), crate_json("3.27.0"))  # ~ pin, within
    write(fx / web("crates.io/api/v1/crates/cc"), crate_json("1.2.0"))
    write(fx / web("crates.io/api/v1/crates/io-imap"), crate_json("0.8.0"))  # grouped
    write(fx / web("crates.io/api/v1/crates/io-smtp"), crate_json("0.6.9"))  # in group, but current
    write(fx / web("crates.io/api/v1/crates/blitz-dom"),
          json.dumps({"crate": {"max_stable_version": "0.2.4", "max_version": "0.3.0-beta.9"}}))
    write(fx / web("crates.io/api/v1/crates/pre"),
          json.dumps({"crate": {"max_stable_version": "0.2.4", "max_version": "0.3.0-beta.4"}}))
    write(fx / web("static.rust-lang.org/dist/channel-rust-stable.toml"),
          '[pkg.rust]\nversion = "1.100.0 (abcdef 2026-11-01)"\n')
    write(fx / web("endoflife.date/api/python.json"),
          json.dumps([{"cycle": "3.15", "latest": "3.15.1"}, {"cycle": "3.14", "latest": "3.14.9"}]))
    write(fx / f("gh api repos/cli/cli/releases/latest"), json.dumps({"tag_name": "v2.98.0"}))
    write(fx / f("gh api repos/jqlang/jq/releases/latest"), json.dumps({"tag_name": "jq-1.9.0"}))
    # sccache: no fixture -> unknown
    write(fx / web("crates.io/api/v1/crates/cargo-hakari"), crate_json("0.9.40"))
    write(fx / web("crates.io/api/v1/crates/cargo-nextest"), crate_json("0.9.143"))
    write(fx / f("gh api repos/actions/checkout/releases/latest"), json.dumps({"tag_name": "v7.1.0"}))
    write(fx / f("gh api repos/actions/cache/releases/latest"), json.dumps({"tag_name": "v6.1.0"}))
    write(fx / web("dl.flathub.org/repo/refs/heads/runtime/org.gnome.Platform/x86_64/51"), "404")
    write(fx / web("dl.flathub.org/repo/refs/heads/runtime/org.gnome.Platform/x86_64/52"), "404")
    write(fx / web("dl.flathub.org/repo/refs/heads/runtime/org.freedesktop.Platform/x86_64/26.08"), "200")
    # 27.08: no fixture -> not decisive, but 26.08 already settles "behind"


def names(items) -> set[str]:
    return {i["name"] for i in items}


def main() -> int:
    survey = load()

    # The requirement arithmetic, on its own.
    m = survey.requirement_allows
    case("caret allows a minor", m("1.2", "1.9.0"))
    case("caret refuses a major", not m("1.2", "2.0.0"))
    case("0.x caret refuses a minor", not m("0.8.3", "0.9.0"))
    case("0.x caret allows a patch", m("0.8.3", "0.8.9"))
    case("exact pin refuses a patch", not m("=0.8.3", "0.8.4"))
    case("tilde refuses a minor", not m("~3.10.0", "3.11.0"))
    case("tilde allows a patch", m("~3.10.0", "3.10.9"))
    case("wildcard allows anything", m("*", "9.9.9"))
    case("range allows inside", m(">=1.2, <1.5", "1.4.0"))
    case("range refuses outside", not m(">=1.2, <1.5", "1.5.0"))
    case("0.0.x caret is a pin", not m("0.0.3", "0.0.4"))

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        root, fx = tmp / "repo", tmp / "fx"
        root.mkdir()
        fx.mkdir()
        build(root, fx, survey)
        run = subprocess.run(
            [sys.executable, "-I", str(SCRIPT), "--root", str(root), "--offline", str(fx), "--json"],
            capture_output=True, text=True)
        case("json run exits 0", run.returncode == 0, run.stderr)
        try:
            data = json.loads(run.stdout)
        except ValueError:
            data = {}
            case("json parses", False, run.stdout + run.stderr)
        crates = data.get("crates", {})
        within = {i["name"]: i for i in crates.get("within_semver", [])}
        major = {i["name"]: i for i in crates.get("major", [])}
        groups = {g["name"]: g for g in crates.get("groups", [])}
        case("a requirement that already allows latest is not behind", "serde" not in within, str(within))
        case("workspace-inherited dep listed once",
             len([1 for i in crates.get("within_semver", []) if i["name"] == "turso"]) == 1)
        case("exact pin flagged", within.get("turso", {}).get("pinned") is True, str(within))
        case("tilde bound is within semver", "tempfile" in within, str(within))
        case("major split out", set(major) == {"bigdep", "gtk4"}, str(major))
        case("current crates absent", not ({"tokio", "fresh", "cc"} & (set(within) | set(major))))
        case("workspace-hack skipped and path deps skipped",
             "hackonly" not in str(data) and "local" not in str(data))
        case("unreachable crate is unknown, not current",
             names(crates.get("unknown", [])) == {"ghost"}, str(crates.get("unknown")))
        case("group shown once with its blocker",
             list(groups) == ["pimalaya"] and groups["pimalaya"]["waits_on"].startswith("the whole"),
             str(groups))
        case("group lists only members that are behind",
             names(groups.get("pimalaya", {}).get("members", [])) == {"io-imap"}, str(groups))
        case("grouped crate not repeated as a plain line",
             "io-imap" not in (set(within) | set(major)))

        case("an exact prerelease pin is current at its own release, not behind a stable",
             "pre" not in (set(within) | set(major)), str(within))

        tc = data.get("toolchain", {})
        case("toolchain behind", tc.get("status") == "behind" and tc.get("latest") == "1.100.0", str(tc))

        tools = {i["name"]: i for i in data.get("tools", [])}
        case("hakari behind", tools.get("cargo-hakari", {}).get("status") == "behind", str(tools))
        case("nextest current", tools.get("cargo-nextest", {}).get("status") == "current", str(tools))
        case("python takes the newest cycle", tools.get("python", {}).get("latest") == "3.15.1", str(tools))
        case("gh current", tools.get("gh", {}).get("status") == "current", str(tools))
        case("jq tag prefix stripped", tools.get("jq", {}).get("latest") == "1.9.0", str(tools))
        case("tool without an answer is unknown", tools.get("sccache", {}).get("status") == "unknown", str(tools))

        actions = {i["name"]: i for i in data.get("actions", [])}
        case("action behind", actions.get("actions/checkout", {}).get("status") == "behind", str(actions))
        case("action current", actions.get("actions/cache", {}).get("status") == "current", str(actions))
        case("local workflow ref ignored", len(actions) == 2, str(actions))

        fp = {i["name"]: i for i in data.get("flatpak", [])}
        case("gnome runtime current when no newer branch",
             fp.get("org.gnome.Platform", {}).get("status") == "current", str(fp))
        case("freedesktop runtime behind when 26.08 exists",
             fp.get("org.freedesktop.Platform", {}).get("status") == "behind"
             and fp["org.freedesktop.Platform"]["latest"] == "26.08", str(fp))

        bl = data.get("blitz", {})
        case("blitz behind", bl.get("status") == "behind" and bl.get("latest") == "0.3.0-beta.9", str(bl))

        text = subprocess.run(
            [sys.executable, "-I", str(SCRIPT), "--root", str(root), "--offline", str(fx)],
            capture_output=True, text=True)
        case("text run exits 0", text.returncode == 0, text.stderr)
        out = text.stdout
        case("text names the pin and the unknowns",
             "turso" in out and "=0.8.3" in out and "ghost" in out and "unknown" in out, out)
        case("text shows the group once",
             out.count("pimalaya") == 1, out)

        # Everything unreachable: nothing may read as current.
        empty = tmp / "empty"
        empty.mkdir()
        dead = subprocess.run(
            [sys.executable, "-I", str(SCRIPT), "--root", str(root), "--offline", str(empty), "--json"],
            capture_output=True, text=True)
        d = json.loads(dead.stdout) if dead.returncode == 0 else {}
        case("no answers means no 'current' anywhere",
             "current" not in json.dumps(d).replace('"current":', ""), json.dumps(d)[:400])
        case("no answers: toolchain unknown", d.get("toolchain", {}).get("status") == "unknown")

    if FAILURES:
        print("\nFAILED:\n" + "\n".join(FAILURES))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
