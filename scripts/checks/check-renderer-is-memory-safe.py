#!/usr/bin/env python3
"""Refuse C, native bindings or a network feature in the reading renderer.

Spec 006 FR-023a renders hostile mail in Postio's own process, and the
maintainer accepted that only because every parser on the path is
memory-safe; FR-001 makes it incapable of a connection. This check walks
`postio-render`'s graph as a product build resolves it -- normal and build
edges, features as `cargo tree` resolves them for that build, so the
crate's own tests (a socket, `postio-test-support`) are exempt -- and
fails when that graph could run C or reach the network
(`specs/006-email-rendering/contracts/renderer-graph-checks.md` § 2).

It is a graph check rather than an observation because an observation only
covers the paths a test happened to take.

Exit status: 0 clean, 1 violation, 2 the check could not run.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATE = "postio-render"

# `links` values used only as uniqueness markers, with nothing native behind
# them.
LINKS_ALLOWED = {"rayon-core", "servo_style_crate"}
# Profiles that may abort on panic because they build one package that
# does not link the renderer -- held to that on every run, not trusted.
ABORT_PROFILES = {
    # The terminal's size-tuned build; its panic hook restores the
    # terminal before the abort (the root Cargo.toml says why).
    "release-tui": "postio-tui",
}
BUILD_TOOLS = {"cc", "cmake", "bindgen"}
IMAGE_REFUSED = {"avif", "avif-native", "tiff", "exr", "bmp", "ico"}

TRACE = "find which feature pulled it in (`cargo tree -p postio-render -e features -i {pkg}`) and turn it off"
# The graph a product build of the renderer resolves, which is Linux's: the
# renderer ships only in the GTK app (spec 006 is Linux-scoped; the Mac
# reads in WKWebView, spec 009 M5). Resolving for the host instead fails on
# a Mac, where chrono's iana-time-zone pulls core-foundation-sys into a
# graph no Mac build ever links.
TARGET = "x86_64-unknown-linux-gnu"

FONTS = "fonts come from FontSet via fontdb, not the system font stack (research R3)"


def graph() -> dict[str, set[str]]:
    """Every package in the product graph, with its resolved features."""
    out = subprocess.run(
        ["cargo", "tree", "-q", "-p", CRATE, "-e", "normal,build",
         "--target", TARGET,
         "--prefix", "none", "--no-dedupe", "-f", "{p}|{f}"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    packages: dict[str, set[str]] = {}
    for line in out.splitlines():
        if "|" not in line:
            continue
        package, features = line.split("|", 1)
        name = package.split(" ", 1)[0]
        features = features.removesuffix(" (*)")
        packages.setdefault(name, set()).update(f for f in features.split(",") if f)
    return packages


def links() -> dict[str, str]:
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout)
    return {p["name"]: p["links"] for p in meta["packages"] if p.get("links")}


def links_renderer(package: str) -> bool:
    out = subprocess.run(
        ["cargo", "tree", "-q", "-p", package, "-e", "normal,build",
         "--target", TARGET,
         "--prefix", "none", "-f", "{p}"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    return any(line.split(" ", 1)[0] == CRATE for line in out.splitlines())


def violations() -> list[str]:
    packages = graph()
    linked = links()
    found = []
    for name in sorted(packages):
        if name in linked and linked[name] not in LINKS_ALLOWED:
            found.append(f"{name} links native `{linked[name]}`: {TRACE.format(pkg=name)}")
        elif name.endswith("-sys"):
            found.append(f"{name} is a native binding: {TRACE.format(pkg=name)}")
        if name in BUILD_TOOLS:
            found.append(f"{name} compiles C/C++ at build time: {TRACE.format(pkg=name)}")
    refused = packages.get("image", set()) & IMAGE_REFUSED
    if refused:
        found.append(
            f"image has {', '.join(sorted(refused))}: postio-render enables png, jpeg, gif, "
            "webp only (research R4)"
        )
    if "net" in packages.get("blitz-dom", set()):
        found.append("blitz-dom has `net`: the renderer never fetches; remote bytes come "
                     "through postio-runtime's fetcher (FR-001)")
    if "system-fonts" in packages.get("blitz-dom", set()):
        found.append(f"blitz-dom has `system-fonts`: {FONTS}")
    for name in ("fontique", "parley"):
        if "system" in packages.get(name, set()):
            found.append(f"{name} has `system` (fontconfig, C, on the content path): {FONTS}")
    if "blitz-paint" in packages and "svg" not in packages["blitz-paint"]:
        found.append("blitz-paint lacks `svg`, so SVG is parsed and never painted: "
                     "enable blitz-paint/svg (research R5)")
    profiles = tomllib.loads((ROOT / "Cargo.toml").read_text()).get("profile", {})
    for profile, settings in sorted(profiles.items()):
        builds = ABORT_PROFILES.get(profile)
        if settings.get("panic") == "abort" and (builds is None or links_renderer(builds)):
            found.append(f"profile.{profile} sets panic = \"abort\", so a caught render panic "
                         "would crash the app: keep unwind (research R6)")
    return found


def main() -> int:
    if shutil.which("cargo") is None:
        print("renderer-is-memory-safe check could not run: cargo is not on PATH")
        return 2
    try:
        found = violations()
    except subprocess.CalledProcessError as error:
        print(f"renderer-is-memory-safe check could not run:\n{error.stderr}")
        return 2
    if found:
        print("renderer-is-memory-safe check FAILED (spec 006 FR-001, FR-023a):\n")
        for line in found:
            print(f"  - {line}")
        return 1
    print("renderer-is-memory-safe check passed (no C, no native links, no network "
          f"feature in {CRATE}'s graph).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
