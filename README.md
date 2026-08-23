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

Postio is pre-release and under active development. The core mail
experience — accounts, sync, threading, search, the three-pane reading
layout, keyboard navigation, compose — is being built out epic by epic;
see [`spec.md`](spec.md) for the full product spec and the design canvas
at [`Design/Mail Client.dc.html`](Design/Mail%20Client.dc.html) for the
visual target.

**v1 scope:** Linux (GTK4/libadwaita) only, IMAP + SMTP only, targeting
iCloud with an app-specific password (no OAuth). SQLite for metadata,
threading, sync state and full-text search, plus a content-addressed blob
store for raw messages and attachments — no maildir/mbox/notmuch, no store
picker.

**Deliberately not in v1:** other platforms, other protocols, OAuth, and
AI features (rules/filters, smart labels, MCP support) — all founding
ideas for later, tracked separately so the core mail experience lands
first.

Screenshots of the running app will go here once the shell (E7) is far
enough along to be worth a picture — see `docs/images/`.

## Building and running

### System dependencies

Fedora 40+ (developed and tested on Fedora 44 / GNOME 50 / Wayland):

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 sqlite-devel libsecret-devel glib2-devel pkgconf-pkg-config
```

Verified working against: gtk4 4.22.4, libadwaita-1 1.9.3,
webkitgtk-6.0 2.52.5, sqlite3 3.51.2, libsecret-1 0.21.7, glib-2.0 2.88.3.

### Build

```bash
cargo build --workspace
```

### Run

```bash
cargo run -p postio-gtk --bin postio
```

### Test

```bash
cargo test --workspace                  # never touches the network
cargo test --workspace -- --ignored     # live iCloud tests; needs POSTIO_TEST_* env
cargo clippy --workspace --all-targets -- -D warnings
cargo bench                             # perf budgets — see "It must feel instant" below
```

Development is test-driven: a failing test is written before the code
that makes it pass, for every crate. See [`CLAUDE.md`](CLAUDE.md) for the
full contributor workflow, commit conventions, and architectural
invariants enforced in CI.

### It must feel instant

Performance is a functional requirement, enforced by `cargo bench` rather
than checked by hand:

| Budget | Target |
|---|---|
| Startup to usable UI (populated DB) | < 500 ms |
| Ordinary UI interaction | < 16 ms |
| Local search | < 100 ms |

Transitions are ≤ 100 ms or absent entirely, and `prefers-reduced-motion`
is always honored. A mailbox is never loaded into memory in full — the
message list is windowed over paged SQLite.

## Configuration

Postio reads `config.toml` from `$POSTIO_CONFIG`, or else
`$XDG_CONFIG_HOME/postio/config.toml`, or else
`~/.config/postio/config.toml`. A missing file is fine — first run needs
nothing on disk — and the file is live-reloaded on save.

**No credential ever lives in `config.toml`.** Passwords and
app-specific passwords go in the Secret Service keyring; an account in
the file only references a keyring entry.

```toml
[ui]
density = "airy"          # airy | comfortable | compact
theme = "system"          # system | light | dark

[accounts.personal]
email = "ada@example.com"
display_name = "Ada"
default = true

[accounts.personal.imap]
host = "imap.example.com"
port = 993
security = "implicit-tls"

[accounts.personal.smtp]
host = "smtp.example.com"
port = 465
security = "implicit-tls"

[sync]
idle = true

[keys]
archive = "y"             # overrides the default binding for `archive`
```

For iCloud, use the app-specific password you generate at
<https://appleid.apple.com/account/manage> — iCloud does not support
plain account passwords for IMAP/SMTP.

Sections in brief:

- `[ui]` — density, theme, hover actions, thread drill-in.
- `[accounts.<id>]` — one table per account: address, display name, and
  nested `[accounts.<id>.imap]` / `[accounts.<id>.smtp]` server settings.
- `[sync]` — IDLE vs. polling, connection budget.
- `[filters]` — named saved search queries.
- `[keys]` — command id to key binding, overriding the defaults below.

Any key this version of Postio does not recognize is preserved verbatim
on save, so a config file survives a downgrade.

## Keybindings

Postio is built to be driven entirely from the keyboard. The full
reference — every command, its default binding, where it applies, and
whether it's undoable — is generated from the command registry and lives
in [`docs/keybindings.md`](docs/keybindings.md), so it can never drift
out of sync with the running application. Every binding is rebindable
from `[keys]` in `config.toml`, and the running app's `?` cheat sheet and
`Ctrl+K` command palette are generated from the same table.

## Architecture

```
postio-gtk    GTK4 + libadwaita + WebKitGTK. Widgets, CSS, keymap, palette.
     |        Command down / Event up. No SQL, no IMAP.
postio-core   UI-agnostic runtime: command bus, registry, event stream,
     |        app state, undo stack, tokio<->glib bridge.
     +-- postio-sync     operation queue, QRESYNC resync, IDLE, backoff
     |     +-- postio-imap (io-imap)   postio-smtp (io-smtp)
     +-- postio-storage  SQLite, migrations, repositories, blob store
     +-- postio-search   FTS5 index, query-operator parser
     +-- postio-config   TOML schema, validation, watcher, live reload
postio-model  pure domain types + JWZ threading. No storage, no protocol.
```

`postio-core` has no GTK dependency, which is what keeps a non-Linux
frontend possible later. See [`CLAUDE.md`](CLAUDE.md) for the full set of
architectural invariants CI enforces, and [`docs/decisions/`](docs/decisions/)
for the reasoning behind key choices (e.g. why `io-imap`).

## License

MIT — see [`LICENSE`](LICENSE).
