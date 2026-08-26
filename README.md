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

System dependencies — Fedora 40+:

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 sqlite-devel libsecret-devel glib2-devel pkgconf-pkg-config
```

Ubuntu 26.04 (earlier releases ship a GTK older than the 4.20 floor):

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
                 libwebkitgtk-6.0-dev libsqlite3-dev libsecret-1-dev \
                 libglib2.0-dev libpango1.0-dev
```

Rust is pinned by [`rust-toolchain.toml`](rust-toolchain.toml) — with
[rustup](https://rustup.rs), the right compiler arrives on the first `cargo`
command.

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
builds against the GNOME 50 runtime:

```bash
python3 flatpak/flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json
flatpak-builder --user --install --force-clean flatpak/build-dir flatpak/dev.postio.Postio.json
```

One-time SDK setup and the details are in [`flatpak/README.md`](flatpak/README.md).

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

## It must feel instant

Performance is a functional requirement, enforced by `cargo bench`:

| Budget | Target | Measured |
|---|---|---|
| Startup to usable UI (populated DB) | < 500 ms | **147 ms** |
| Ordinary UI interaction | < 16 ms | **~2 ms** |
| Local search | < 100 ms | **27 ms** |
| Memory, 100,000 messages | no full-mailbox load | **47 MiB**, flat |

The full baseline — what was measured, on what, and how to reproduce every
number — is [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md).

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
