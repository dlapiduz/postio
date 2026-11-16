#!/usr/bin/env python3
"""One report of everything in the repository that is behind its upstream.

"Upgrade everything" used to be a dozen hand-made queries. This asks them
all once: direct Rust dependencies (crates.io), the Rust toolchain, the
pinned tools, GitHub Actions, Flatpak runtimes and the vendored Blitz.

A source it cannot reach is reported as unknown. Unknown is never current:
a survey that went quiet on a rate limit must not read as "nothing to do".

Usage:
  scripts/deps-survey.py                 readable report (takes minutes:
                                         crates.io is asked one crate a second)
  scripts/deps-survey.py --json          the same, for machines
  scripts/deps-survey.py --offline DIR   answer from canned files in DIR
                                         (the self-test; see fixture_name)
  scripts/deps-survey.py --root DIR      survey another checkout

Needs `gh` for GitHub releases. Python 3.11+, standard library only.
Exit status: 0 the survey ran (whatever it found), 2 bad usage.
"""

from __future__ import annotations

import argparse
import glob
import json
import re
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

# Generic on purpose: the registry asks for a contact, and none is given.
USER_AGENT = "postio-deps-survey"
CRATES_API = "https://crates.io/api/v1/crates/"
RUST_STABLE = "https://static.rust-lang.org/dist/channel-rust-stable.toml"
PYTHON_EOL = "https://endoflife.date/api/python.json"
FLATHUB = "https://dl.flathub.org/repo/refs/heads/runtime/{id}/x86_64/{branch}"
HACK_CRATE = "postio-workspace-hack"

# mise tools whose releases are on GitHub, and the tag prefix to strip.
MISE_GITHUB = {
    "gh": ("cli/cli", "v"),
    "jq": ("jqlang/jq", "jq-"),
    "sccache": ("mozilla/sccache", "v"),
}


class Unreachable(Exception):
    """A source gave no answer; the caller reports unknown."""


def fixture_name(key: str) -> str:
    """The file in an --offline directory that answers `key`.

    `key` is the URL asked, or `gh api <path>` for a GitHub call.
    """
    key = re.sub(r"^https?://", "", key)
    return re.sub(r"[^A-Za-z0-9._-]+", "_", key).strip("_")


class Source:
    """The only place that talks to the world."""

    def __init__(self, offline: Path | None) -> None:
        self.offline = offline
        self.cache: dict[str, str] = {}
        self.last_crates_call = 0.0

    def _canned(self, key: str) -> str:
        try:
            return (self.offline / fixture_name(key)).read_text()
        except OSError as e:
            raise Unreachable(f"no canned answer for {key}") from e

    def text(self, url: str) -> str:
        if url in self.cache:
            return self.cache[url]
        if self.offline is not None:
            out = self._canned(url)
        else:
            if url.startswith(CRATES_API):  # the registry asks for one a second
                wait = 1.05 - (time.monotonic() - self.last_crates_call)
                if wait > 0:
                    time.sleep(wait)
                self.last_crates_call = time.monotonic()
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            try:
                with urllib.request.urlopen(req, timeout=30) as r:
                    out = r.read().decode()
            except (urllib.error.URLError, OSError, ValueError) as e:
                raise Unreachable(f"{url}: {e}") from e
        self.cache[url] = out
        return out

    def json(self, url: str):
        try:
            return json.loads(self.text(url))
        except ValueError as e:
            raise Unreachable(f"{url}: not JSON") from e

    def gh(self, path: str):
        key = f"gh api {path}"
        if self.offline is not None:
            try:
                return json.loads(self._canned(key))
            except ValueError as e:
                raise Unreachable(f"{key}: not JSON") from e
        try:
            p = subprocess.run(["gh", "api", path], capture_output=True, text=True, timeout=60)
            if p.returncode != 0:
                raise Unreachable(f"{key}: {p.stderr.strip()[:120]}")
            return json.loads(p.stdout)
        except (OSError, subprocess.SubprocessError, ValueError) as e:
            raise Unreachable(f"{key}: {e}") from e

    def status(self, url: str) -> int:
        """HTTP status of a HEAD request; only 200 and 404 are answers."""
        if self.offline is not None:
            try:
                return int(self._canned(url).strip())
            except ValueError as e:
                raise Unreachable(f"{url}: bad canned status") from e
        req = urllib.request.Request(url, method="HEAD", headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.status
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return 404
            raise Unreachable(f"{url}: HTTP {e.code}") from e
        except (urllib.error.URLError, OSError) as e:
            raise Unreachable(f"{url}: {e}") from e


# ---- versions and requirements ------------------------------------------

def vtuple(text: str) -> tuple[int, ...]:
    """Numeric part of a version: `v7.0.1`, `jq-1.8`, `0.3.0-beta.4` -> ints."""
    core = re.search(r"\d+(?:\.\d+)*", text)
    return tuple(int(n) for n in core.group(0).split(".")) if core else ()


def vkey(text: str):
    """Sort key that knows prereleases: 0.3.0-beta.9 > 0.3.0-beta.4, < 0.3.0."""
    nums = vtuple(text)
    m = re.search(r"\d-([0-9A-Za-z.-]+)", text)
    if not m:
        return (nums, 1, ())
    idents = tuple((0, int(i), "") if i.isdigit() else (1, 0, i) for i in m.group(1).split("."))
    return (nums, 0, idents)


def _is_prerelease(text: str) -> bool:
    return bool(re.search(r"\d-[0-9A-Za-z]", text))


def _pad(t: tuple[int, ...]) -> tuple[int, int, int]:
    t = (tuple(t) + (0, 0, 0))[:3]
    return t  # type: ignore[return-value]


_COMPARATOR = re.compile(
    r"^(\^|~|=|>=|>|<=|<)?\s*v?(\d+|\*|x)(?:\.(\d+|\*|x))?(?:\.(\d+|\*|x))?(?:-[0-9A-Za-z.+-]*)?$")


def _parse_comparator(text: str):
    """-> (op, nums as given, padded); nums stops at the first wildcard."""
    m = _COMPARATOR.match(text.strip())
    if not m:
        raise ValueError(f"unreadable requirement {text!r}")
    op = m.group(1) or "^"
    nums: list[int] = []
    for part in m.groups()[1:]:
        if part is None or part in ("*", "x"):
            break
        nums.append(int(part))
    return op, tuple(nums), _pad(tuple(nums))


def _caret_upper(nums: tuple[int, ...], v: tuple[int, int, int]) -> tuple[int, int, int]:
    a, b, c = v
    if a > 0 or len(nums) <= 1:
        return (a + 1, 0, 0)
    if b > 0 or len(nums) == 2:
        return (0, b + 1, 0)
    return (0, 0, c + 1)


def _bounds(op: str, nums: tuple[int, ...], v: tuple[int, int, int]):
    """-> (lower inclusive or None, upper exclusive or None)."""
    if not nums:
        return None, None
    a, b, c = v
    if op == "=":
        if len(nums) == 3:
            return v, (a, b, c + 1)
        return v, ((a, b + 1, 0) if len(nums) == 2 else (a + 1, 0, 0))
    if op == "^":
        return v, _caret_upper(nums, v)
    if op == "~":
        return v, ((a + 1, 0, 0) if len(nums) == 1 else (a, b + 1, 0))
    if op == ">=":
        return v, None
    if op == ">":
        return (v[0], v[1], v[2] + 1), None
    if op == "<":
        return None, v
    return None, (a, b, c + 1)  # <=


def requirement_allows(req: str, version: str) -> bool:
    """Does a Cargo requirement admit this (stable) version?"""
    ver = _pad(vtuple(version))
    for part in req.split(","):
        if re.match(r"\s*=\s*v?\d[^,]*-", part):  # an exact prerelease pin
            if vkey(part.split("=", 1)[1].strip()) != vkey(version):
                return False
            continue
        lo, hi = _bounds(*_parse_comparator(part))
        if lo is not None and ver < lo:
            return False
        if hi is not None and ver >= hi:
            return False
    return True


def classify(req: str, latest: str) -> str:
    """current | within-semver | major, for a requirement behind `latest`."""
    if requirement_allows(req, latest):
        return "current"
    op, nums, v = _parse_comparator(req.split(",")[0])
    if nums and op != "<" and op != "<=":
        if _pad(vtuple(latest)) < _caret_upper(nums if len(nums) == 3 else (v[0], v[1], v[2]), v):
            return "within-semver"
    return "major"


# ---- crates.io ----------------------------------------------------------

def latest_stable(src: Source, name: str, pre: bool = False) -> str:
    """Newest stable version; with `pre`, newest of all (we are on a prerelease)."""
    data = src.json(CRATES_API + name)
    try:
        if pre:
            return data["crate"]["max_version"]
        latest = data["crate"].get("max_stable_version")
        if not latest:
            stable = [v["num"] for v in data.get("versions", [])
                      if not v.get("yanked") and not _is_prerelease(v["num"])]
            # A crate that has only prereleases (Blitz, oo7) is "latest" by them.
            latest = max(stable, key=vkey) if stable else data["crate"]["max_version"]
        return latest
    except (KeyError, ValueError, TypeError, AttributeError) as e:
        raise Unreachable(f"{name}: unexpected crates.io answer") from e


def load_groups(root: Path) -> list[dict]:
    path = root / "scripts" / "deps-groups.toml"
    if not path.exists():
        return []
    return tomllib.loads(path.read_text()).get("group", [])


def direct_dependencies(root: Path) -> dict[str, dict]:
    """crates.io name -> {"reqs": {req: [where]}}, direct and external only."""
    found: dict[str, dict] = {}

    def add(name, spec, where):
        if isinstance(spec, str):
            req = spec
        elif isinstance(spec, dict):
            if spec.get("workspace") or "path" in spec or "git" in spec or "version" not in spec:
                return
            req = spec["version"]
            name = spec.get("package", name)
        else:
            return
        if name.startswith("postio-"):
            return
        found.setdefault(name, {"reqs": {}})["reqs"].setdefault(req, []).append(where)

    def tables(doc, where):
        for key in ("dependencies", "dev-dependencies", "build-dependencies"):
            for n, s in doc.get(key, {}).items():
                add(n, s, where)
        for target in doc.get("target", {}).values():
            tables(target, where)

    top = tomllib.loads((root / "Cargo.toml").read_text())
    for n, s in top.get("workspace", {}).get("dependencies", {}).items():
        add(n, s, "workspace")
    members: list[Path] = []
    for pattern in top.get("workspace", {}).get("members", []):
        members += [Path(p) for p in sorted(glob.glob(str(root / pattern)))]
    for m in members:
        if m.name == HACK_CRATE or not (m / "Cargo.toml").exists():
            continue
        tables(tomllib.loads((m / "Cargo.toml").read_text()), m.name)
    return found


def survey_crates(src: Source, root: Path, progress) -> dict:
    deps = direct_dependencies(root)
    groups = load_groups(root)
    group_of = {m: g for g in groups for m in g.get("members", [])}
    within, major, unknown, grouped = [], [], [], {}
    current = 0
    for i, name in enumerate(sorted(deps), 1):
        progress(f"crates.io {i}/{len(deps)} {name}")
        reqs = deps[name]["reqs"]
        try:
            latest = latest_stable(src, name, any(_is_prerelease(r) for r in reqs))
        except Unreachable as e:
            unknown.append({"name": name, "requirement": ", ".join(reqs), "error": str(e)})
            continue
        # The loosest requirement decides: one that allows latest means a
        # manifest can already take it; report the tightest behind one.
        behind = [(r, classify(r, latest)) for r in reqs]
        behind = [(r, k) for r, k in behind if k != "current"]
        if not behind:
            current += 1
            continue
        req, kind = behind[0]
        item = {"name": name, "requirement": req, "latest": latest,
                "pinned": any(r.lstrip().startswith("=") for r, _ in behind),
                "in": sorted({w for r, _ in behind for w in reqs[r]})}
        if name in group_of:
            grouped.setdefault(group_of[name]["name"], {
                "name": group_of[name]["name"], "waits_on": group_of[name].get("waits_on", ""),
                "members": []})["members"].append(item)
        elif kind == "within-semver":
            within.append(item)
        else:
            major.append(item)
    return {"checked": len(deps), "current": current, "within_semver": within, "major": major,
            "groups": list(grouped.values()), "unknown": unknown}


# ---- the rest -----------------------------------------------------------

def compare(name: str, current: str, fetch, **extra) -> dict:
    """A version pinned somewhere against whatever `fetch()` says is newest."""
    item = {"name": name, "current": current, **extra}
    try:
        latest = fetch()
    except Unreachable as e:
        return {**item, "latest": None, "status": "unknown", "error": str(e)}
    item["latest"] = latest
    if not vtuple(current) or not vtuple(latest):
        return {**item, "status": "unknown", "error": "unreadable version"}
    item["status"] = "behind" if vkey(latest) > vkey(current) else "current"
    return item


def survey_toolchain(src: Source, root: Path) -> dict:
    channel = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    if not vtuple(channel):
        return {"name": "rust", "current": channel, "latest": None, "status": "unknown",
                "error": "channel is not an exact version"}

    def fetch():
        try:
            return tomllib.loads(src.text(RUST_STABLE))["pkg"]["rust"]["version"].split()[0]
        except (ValueError, KeyError) as e:
            raise Unreachable("unexpected channel-rust-stable.toml") from e
    return compare("rust", channel, fetch)


def github_latest(src: Source, repo: str, strip: str = "v") -> str:
    try:
        tag = src.gh(f"repos/{repo}/releases/latest")["tag_name"]
    except (KeyError, TypeError) as e:
        raise Unreachable(f"{repo}: no release tag") from e
    return tag.removeprefix(strip)


def survey_tools(src: Source, root: Path) -> list[dict]:
    out = []
    for script in sorted((root / "scripts").glob("install-*.sh")):
        m = re.search(r'^VERSION="?([^"\s]+)"?', script.read_text(), re.M)
        if not m:
            continue
        stem = script.stem.removeprefix("install-")
        crate = stem if stem.startswith("cargo-") else f"cargo-{stem}"
        out.append(compare(crate, m.group(1), lambda c=crate: latest_stable(src, c),
                           source=f"{script.name} (crates.io)"))
    mise = root / "mise.toml"
    for tool, current in (tomllib.loads(mise.read_text()).get("tools", {}) if mise.exists() else {}).items():
        if tool == "python":
            def fetch():
                try:
                    return src.json(PYTHON_EOL)[0]["latest"]
                except (IndexError, KeyError, TypeError) as e:
                    raise Unreachable("unexpected endoflife.date answer") from e
            out.append(compare(tool, current, fetch, source="mise.toml (endoflife.date)"))
        elif tool in MISE_GITHUB:
            repo, strip = MISE_GITHUB[tool]
            out.append(compare(tool, current, lambda r=repo, s=strip: github_latest(src, r, s),
                               source=f"mise.toml (github {repo})"))
        else:
            out.append({"name": tool, "current": current, "latest": None, "status": "unknown",
                        "error": "no upstream configured in deps-survey.py"})
    return out


def survey_actions(src: Source, root: Path) -> list[dict]:
    pins: dict[tuple[str, str], list[str]] = {}
    pattern = re.compile(r"uses:\s*([\w.-]+/[\w.-]+(?:/[\w./-]+)?)@[0-9a-f]{40}\s*#\s*(v?\d[\w.]*)")
    for wf in sorted((root / ".github" / "workflows").glob("*.yml")):
        for m in pattern.finditer(wf.read_text()):
            repo = "/".join(m.group(1).split("/")[:2])
            pins.setdefault((repo, m.group(2)), []).append(wf.name)
    return [compare(repo, ver, lambda r=repo: github_latest(src, r, ""), workflows=sorted(set(wfs)))
            for (repo, ver), wfs in sorted(pins.items())]


def _next_branches(branch: str) -> list[str]:
    if "." in branch:  # freedesktop style: 25.08 -> 26.08, 27.08
        year, rest = branch.split(".", 1)
        return [f"{int(year) + n}.{rest}" for n in (1, 2)]
    return [str(int(branch) + n) for n in (1, 2)]


def survey_flatpak(src: Source, root: Path) -> list[dict]:
    out, seen = [], set()
    for manifest in sorted((root / "flatpak").glob("*.json")):
        try:
            doc = json.loads(manifest.read_text())
            runtime, branch = doc["runtime"], str(doc["runtime-version"])
            nxt = _next_branches(branch)
        except (ValueError, KeyError, TypeError):
            continue
        if (runtime, branch) in seen:
            continue
        seen.add((runtime, branch))
        item = {"name": runtime, "current": branch, "manifest": manifest.name}
        newer, errors = [], []
        for b in nxt:
            try:
                if src.status(FLATHUB.format(id=runtime, branch=b)) == 200:
                    newer.append(b)
            except Unreachable as e:
                errors.append(str(e))
        if newer:
            out.append({**item, "latest": newer[-1], "status": "behind"})
        elif errors:  # a 404 for one branch proves nothing about one we could not ask
            out.append({**item, "latest": None, "status": "unknown", "error": errors[0]})
        else:
            out.append({**item, "latest": branch, "status": "current"})
    return out


def survey_blitz(src: Source, root: Path) -> dict:
    pin = tomllib.loads((root / "patches/blitz/upstream.toml").read_text())["version"]
    return compare("blitz-dom", pin, lambda: latest_stable(src, "blitz-dom", _is_prerelease(pin)))


# ---- output -------------------------------------------------------------

def render(report: dict) -> str:
    lines: list[str] = []
    w = lines.append

    def row(item, extra=""):
        cur = item.get("requirement") or item.get("current")
        w(f"    {item['name']:<34} {cur} -> {item['latest']}{extra}")

    c = report["crates"]
    behind = len(c["within_semver"]) + len(c["major"]) + sum(len(g["members"]) for g in c["groups"])
    w(f"Rust crates: {c['checked']} direct, {behind} behind, {len(c['unknown'])} unknown")
    for title, key in (("within semver (a manifest edit or `cargo update`)", "within_semver"),
                       ("major", "major")):
        if c[key]:
            w(f"  {title}")
            for i in c[key]:
                row(i, "   PINNED (=)" if i["pinned"] else "")
    for g in c["groups"]:
        w(f"  group {g['name']}: waits on {g['waits_on']}")
        for i in g["members"]:
            row(i, "   PINNED (=)" if i["pinned"] else "")
    if c["unknown"]:
        w("  unknown (could not ask)")
        for i in c["unknown"]:
            w(f"    {i['name']:<34} {i['requirement']}   {i['error']}")

    def block(title, items):
        bad = [i for i in items if i["status"] != "current"]
        w("")
        w(f"{title}: {len(items)} checked, {sum(i['status'] == 'behind' for i in items)} behind, "
          f"{sum(i['status'] == 'unknown' for i in items)} unknown")
        for i in bad:
            if i["status"] == "behind":
                w(f"    {i['name']:<34} {i['current']} -> {i['latest']}")
            else:
                w(f"    {i['name']:<34} {i['current']}   unknown: {i.get('error', '')}")

    block("Rust toolchain", [report["toolchain"]])
    block("Tool pins", report["tools"])
    block("GitHub Actions", report["actions"])
    block("Flatpak runtimes", report["flatpak"])
    block("Blitz (vendored)", [report["blitz"]])
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--offline", metavar="DIR", type=Path,
                    help="answer from canned files in DIR instead of the network")
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent,
                    help="the checkout to survey (default: this one)")
    args = ap.parse_args(argv)

    src = Source(args.offline)

    def progress(msg: str) -> None:
        if args.offline is None and sys.stderr.isatty():
            print(f"\r\033[K{msg}", end="", file=sys.stderr, flush=True)

    report = {
        "crates": survey_crates(src, args.root, progress),
        "toolchain": survey_toolchain(src, args.root),
        "tools": survey_tools(src, args.root),
        "actions": survey_actions(src, args.root),
        "flatpak": survey_flatpak(src, args.root),
        "blitz": survey_blitz(src, args.root),
    }
    progress("")
    sys.stdout.write(json.dumps(report, indent=2) + "\n" if args.json else render(report))
    return 0


if __name__ == "__main__":
    sys.exit(main())
