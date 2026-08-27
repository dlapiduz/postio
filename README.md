# Postio

Postio is a local-first, keyboard-first email client built for people who
have too much email.

Read less. Find anything. Act faster.

Postio keeps a full copy of your mail in a local SQLite database with a
built-in full-text index, so search and navigation never wait on the
network. Every action — archive, flag, move, delete, undo — applies
instantly to that local copy and is queued for the server in the
background. The UI never awaits the network.

## Status

Postio is pre-release and under active development. **v1 scope:** Linux
(GTK4/libadwaita) only, IMAP + SMTP only, app-specific passwords (no OAuth).
Other platforms, other protocols, OAuth, and AI features are founding ideas
deliberately deferred so the core mail experience lands first. See
[`docs/PRODUCT.md`](docs/PRODUCT.md) for what Postio must do.

Screenshots will go here once the shell is far enough along to be worth a
picture.

## This codebase is almost entirely AI-generated

Postio is written by AI coding agents — Claude, running in parallel
sessions — under the direction of a human maintainer who sets scope, reviews
the results, and makes the product decisions. That is not a disclaimer; it is
the experiment: not whether an agent can emit code, but whether a *process*
can make agent-written software trustworthy.

- **Every piece of work is a GitHub issue**, worked on its own branch, landed
  as a PR. The issue history is the reasoning, in public.
- **Test-driven development is mandatory**: the failing test comes first, and
  a gate chain — tests, clippy as errors, formatting, architectural boundary
  checks, a personal-data scanner, dependency audits — runs on every landing.
- **The invariants are machine-checked, not remembered.** A crate that must
  not link GTK, a view layer that must not speak SQL, a log that must never
  contain message content — each is a script, because a rule an agent (or a
  person) has to remember is a rule that drifts.
- **Decisions are written down** as [ADRs](docs/decisions/), and hard-won
  lessons live in [`docs/engineering-notes.md`](docs/engineering-notes.md).

Read the code with the same skepticism you would give any codebase — and if
you find something wrong, the issue tracker is where this project thinks. See
[`CONTRIBUTING.md`](CONTRIBUTING.md) for how to file an issue an agent can
act on.

## Building and running

### Supported systems

Postio is Linux only, GTK4/libadwaita, Wayland first and X11 where it
happens to work (see [`docs/PRODUCT.md`](docs/PRODUCT.md) §2). What that
means in practice:

- **Verified**: Fedora 40+ under Wayland, against the exact library versions
  the code is written for — gtk4 4.22, libadwaita 1.9, WebKitGTK 2.52. CI
  additionally builds and tests on Ubuntu 26.04.
- **Expected to work, not verified**: other distributions that ship the same
  library floors (the Ubuntu 26.04 line below is one such case), other
  Wayland compositors, and X11 sessions generally.
- Older GTK4/libadwaita (anything before Ubuntu 26.04's 4.20/1.7, for
  example) will fail to build, not misbehave at runtime — `cargo` reports the
  missing symbol at compile time.

### Quickstart

System dependencies — Fedora 40+:

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 sqlite-devel libsecret-devel glib2-devel pkgconf-pkg-config

# The store is SQLCipher, which builds OpenSSL from source (ADR 0014). Its
# `Configure` is a perl program, and Fedora splits the perl standard library
# into packages — without these the build stops at a `Can't locate X.pm in
# @INC` inside a cargo build script, one module at a time.
sudo dnf install perl-FindBin perl-IPC-Cmd perl-Pod-Html perl-Digest-SHA \
                 perl-Text-Template perl-Time-Piece

# Optional but strongly recommended on a machine with more than one checkout:
# every fresh target directory rebuilds that OpenSSL from C source (~4 min),
# and ccache is what lets the second one cost seconds. Wired in automatically
# via scripts/cc-wrapper.sh; without ccache the build is unchanged. #736.
sudo dnf install ccache
```

Ubuntu 26.04 (earlier releases ship a GTK older than the 4.20 floor):

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
                 libwebkitgtk-6.0-dev libsqlite3-dev libsecret-1-dev \
                 libglib2.0-dev libpango1.0-dev
```

Debian and Ubuntu ship the perl modules OpenSSL needs in `perl-base` and
`perl-modules`, both of which `build-essential` already pulls in, so the
extra step above is Fedora-specific.

Rust is pinned by [`rust-toolchain.toml`](rust-toolchain.toml) — with
[rustup](https://rustup.rs), the right compiler arrives on the first `cargo`
command.

The tools the gates run on — Python, `gh`, `jq`, `sccache` — are pinned by
[`mise.toml`](mise.toml), because `scripts/checks/` is 54 Python scripts and
nothing said which Python. It is optional: `mise install` once if you use
[mise](https://mise.jdx.dev), and everything resolves off `PATH` as before if
you do not. It deliberately does not pin Rust (`rust-toolchain.toml` owns
that, and a second place to say it is the bug that pin exists to prevent) or
the system libraries above, which are distro packages rather than tooling.

`gh` needs to be **2.94.0 or newer**: `scripts/issue-claim.sh` reads
`--json blockedBy`, which that release added (cli/cli#13057). An older `gh`
rejects the field outright — writes its complaint to stderr, nothing to
stdout — so `scripts/issue-*.sh` refuse up front with a sentence naming both
versions rather than the traceback that used to follow from the empty
stdout (#558). `mise.toml` already pins comfortably above the floor, which
is the ordinary way this stays true; the runtime check in
`scripts/lib/require-gh.sh` is the backstop for the sessions that do not use
mise, the same gap `RUSTUP_TOOLCHAIN` leaves for the Rust pin
(`docs/engineering-notes.md`).

```bash
cargo run -p postio-app        # build and run
cargo test --workspace         # never touches the network
```

### Installing

To go from source to a working app — `postio` on your `$PATH`, Postio in the
app grid with its icon:

```bash
scripts/install-local.sh               # builds --release, installs to ~/.local
scripts/install-local.sh --uninstall   # removes exactly what it installed
```

Prefer a sandboxed build? The Flatpak manifest in [`flatpak/`](flatpak/)
builds against the GNOME 50 runtime and is Flathub-submission-ready, but
Postio isn't on Flathub yet — for now, build it yourself:

```bash
python3 flatpak/flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json
flatpak-builder --user --install --force-clean flatpak/build-dir flatpak/dev.postio.Postio.json
```

One-time SDK setup and the details are in [`flatpak/README.md`](flatpak/README.md).
Once a Flathub listing exists this will collapse to a single `flatpak
install flathub dev.postio.Postio`; the listing's own description is kept in
[`dev.postio.Postio.metainfo.xml`](crates/postio-gtk/data/dev.postio.Postio.metainfo.xml)
rather than written twice.

A tagged release also publishes a prebuilt `.flatpak` bundle on the
[Releases page](https://github.com/dlapiduz/postio/releases), alongside a
signed build-provenance attestation and a software bill of materials — a
mail client holds your credentials and your mail, so a downloaded bundle
should be checkable rather than merely trusted because it appeared on a
release page. Verify one with the [GitHub
CLI](https://cli.github.com):

```bash
gh attestation verify postio-VERSION-x86_64.flatpak --repo dlapiduz/postio
```

A successful verification confirms the bundle was built by this project's
`release.yml` workflow, from the tagged commit, and has not been modified
since. The SBOM (`postio-VERSION.spdx.json`, also attached to the release) is
attested the same way and lists every dependency the build actually shipped.

First run opens onto a one-screen setup: type your email address and the
autoconfig probe fills in the server settings (a preset table, Thunderbird
autoconfig, then DNS SRV — or manual entry). The password goes straight into
the OS keyring; it is never written to a file. iCloud accounts need an
app-specific password from <https://account.apple.com>.

Then drive it from the keyboard: `j`/`k` to move, `Enter` to open, `e` reply,
`a` archive, `u` undo anything, `/` search (`from:ada is:unread …`), `Ctrl+K`
for the command palette, `?` for the full cheat sheet. Every binding is
rebindable; the generated reference is
[`docs/keybindings.md`](docs/keybindings.md).

### Troubleshooting

**`cargo build` fails looking for a library** (`pkg-config` errors naming
`gtk4`, `libadwaita-1`, `webkitgtk-6.0`, `sqlite3`, or `libsecret-1`): a
system dependency from the Fedora or Ubuntu list above is missing or too
old. Reinstall that line — `pkg-config --modversion gtk4` (etc.) shows what
you actually have against the floors in
[`docs/PRODUCT.md`](docs/PRODUCT.md) §2.

**The window fails to open, or opens with no decorations / broken
rendering**: Postio is a GTK4/libadwaita app and only Wayland is verified —
X11 sessions are expected to work but aren't part of the tested path. If
you're on X11 and hit a rendering issue, running under a Wayland session
(or, as a fallback, forcing the X11 backend with `GDK_BACKEND=x11 cargo run
-p postio-app`) is the first thing to try before filing an issue.

**Onboarding can't save the account, or every launch reopens onboarding**:
Postio stores credentials in the OS keyring over the Secret Service D-Bus
API (`org.freedesktop.secrets`), never in `config.toml`. That needs a
running keyring daemon — GNOME Keyring or KWallet's Secret Service
integration are the common ones. Minimal desktop environments and window
managers often don't start one by default; on Fedora,
`sudo dnf install gnome-keyring` and ensure your session starts it
(GNOME/KDE sessions do this automatically). A locked keyring blocks the
same way — unlock it and retry.

## It must feel instant

Performance is a functional requirement, enforced by `cargo bench`:

| Budget | Target | Measured |
|---|---|---|
| Startup to usable UI (populated DB) | < 500 ms | **427 ms** |
| Ordinary UI interaction | < 16 ms | **0.3 ms** typical |
| Local search | < 100 ms | **42 ms** worst shape |
| Memory, 100,000 messages | no full-mailbox load | **55 MiB**, flat past 100k |

Measured against an **encrypted** store — the database is SQLCipher (ADR 0014)
and there is no unencrypted configuration in normal use, so each figure already
carries the cost of decrypting every page on the way in.

The full baseline — what was measured, on what, which numbers are floors
rather than means, and how to reproduce every one — is
[`docs/PERFORMANCE.md`](docs/PERFORMANCE.md). Two cases are outside budget
today and tracked as their own issues rather than smoothed over here: a
unified thread page across two accounts ([#619]), and startup against its
recorded baseline ([#636]).

[#619]: https://github.com/dlapiduz/postio/issues/619
[#636]: https://github.com/dlapiduz/postio/issues/636

## Configuration

Postio reads `config.toml` from `$POSTIO_CONFIG`, or
`$XDG_CONFIG_HOME/postio/config.toml`, or `~/.config/postio/config.toml`. A
missing file is fine — first run needs nothing on disk — and the file is
live-reloaded on save. Unrecognized keys are preserved on save, so a config
file survives a downgrade.

**No credential ever lives in `config.toml`** — passwords go in the Secret
Service keyring, and an account in the file only references a keyring entry.

```toml
[ui]
density = "airy"          # airy | comfortable | compact
theme = "system"          # system | light | dark

[accounts.personal]
email = "ada@example.com"
default = true

[accounts.personal.imap]
host = "imap.example.com"
port = 993
security = "implicit-tls"

[keys]
archive = "y"             # overrides the default binding for `archive`
```

Other sections: `[accounts.<id>.smtp]`, `[sync]` (IDLE vs. polling),
`[filters]` (named saved searches), `[logging]`.

## Architecture

Fourteen crates in strict layers: a GTK view layer that speaks no SQL and no
protocol, an engine that owns the database and the network, and a UI-agnostic
contract between them — commands down, events up, and the UI never awaits the
network. The boundaries are enforced against cargo's resolved dependency
graph by CI, not by convention.

The full picture — the crate map, the load-bearing decisions and their
costs — is [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), with long-form
ADRs in [`docs/decisions/`](docs/decisions/). The agent workflow, commit
conventions, and quality gates are in [`CLAUDE.md`](CLAUDE.md).

## License

MIT — see [`LICENSE`](LICENSE).
