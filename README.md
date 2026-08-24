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
cargo run -p postio-app
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

| Budget | Target | Measured |
|---|---|---|
| Startup to usable UI (populated DB) | < 500 ms | **147 ms** |
| Ordinary UI interaction | < 16 ms | **~2 ms** |
| Local search | < 100 ms | **29 ms** |
| Memory, 100,000 messages | no full-mailbox load | **47 MiB**, flat |

Transitions are ≤ 100 ms or absent entirely, and `prefers-reduced-motion`
is always honored. A mailbox is never loaded into memory in full — the
message list is windowed over paged SQLite.

#### The baseline

Measured on one developer machine, which makes the numbers a regression
guard rather than a promise about anybody else's hardware. Reproduce them:

```sh
cargo run -p postio-runtime --example seed_store -- /tmp/postio.db 20000
POSTIO_STORE=/tmp/postio.db POSTIO_STARTUP_TRACE=1 POSTIO_STARTUP_EXIT=1 \
  cargo run --release -p postio-app

cargo bench -p postio-runtime --bench store_reads   # the database read
cargo bench -p postio-gtk     --bench list_scroll   # the row draw
cargo bench -p postio-search  --bench search_budget --features index
```

Startup, on a 20,000-message store with an account and six folders:
147 ms, of which 47 ms is `adw::init` and 82 ms is the first frame. The
window, the styles and the fonts are 18 ms between them.

An ordinary interaction is a scroll, and a scroll is two things — a page
read and a screenful of rows drawn. Together they are the ~2 ms above:

| | 1,000 messages | 100,000 messages |
|---|---|---|
| Page read, top of the folder | 139 µs | 219 µs |
| Page read, scrolled to the middle | — | 176 µs |
| Page read, *jumped* to the middle | — | 28 ms |
| Row draw, one screenful | 1.6 ms | 1.6 ms |

Flat against mailbox size, which is the claim that matters: reading page
one of a hundred thousand messages costs what reading page one of a
thousand costs. The exception is a *jump* to a page nobody has scrolled
through — the store has no boundary to seek from and falls back to walking
— which happens once per jump, and every page after it is the 176 µs row.

Search, over a 120,000-message index, by query shape:

| Query shape | Measured |
|---|---|
| Composed — an operator plus free text, what the search bar usually produces | 0.4 ms |
| Simple term — a word matching about 1% of the corpus | 4.5 ms |
| Operator only — `from:`, no free text and no FTS join | 9.7 ms |
| Common word — a word in every message, the worst case | 18.4 ms |
| Common word, with facet counts | 29.4 ms |

The worst shape is the one to watch: `MATCH` and the `count(*)` behind it
have to walk effectively the whole corpus, and it is where a missing index
would show up first.

#### Memory, and the claim it tests

A mailbox is never loaded into memory in full. Measured rather than
asserted, by sampling `/proc/<pid>/status` while the application is open on
a store of each size:

| | 1,000 messages | 100,000 messages |
|---|---|---|
| Anonymous — the application's own heap | 47 MiB | 47 MiB |
| File-backed — mapped store, WAL, shared libraries | 83 MiB | 167 MiB |
| Resident total | 131 MiB | 215 MiB |

**The anonymous figure is the claim.** It is what Postio itself allocates —
the windowed list model, the widgets, the runtime — and it does not move
between a thousand messages and a hundred thousand. The file-backed half
grows because `PRAGMA mmap_size` is 256 MiB and SQLite maps as much of the
store as it touches; those are reclaimable page-cache pages, not mail the
application is holding on to.

Reproduce it:

```sh
cargo run --release -p postio-runtime --example seed_store -- /tmp/big.db 100000
POSTIO_STORE=/tmp/big.db cargo run --release -p postio-app &
grep -E '^(VmRSS|RssAnon|RssFile):' /proc/$!/status
```

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
postio-app        The composition root: opens the store, starts the engine,
   |              runs the UI. The only crate that knows both halves exist.
   +-- postio-gtk        GTK4 + libadwaita + WebKitGTK. Widgets, CSS, keymap.
   |     |               Command down / Event up. No SQL, no protocol.
   |     +-- postio-search   FTS5 index, query-operator parser
   |
   +-- postio-runtime    The database half: the store, and the loop that
         |               drains the queue, backfills bodies and reconnects.
         +-- postio-sync     operation queue, QRESYNC resync, IDLE, backoff
         |     +-- postio-imap (io-imap)   postio-smtp (io-smtp)
         +-- postio-storage  SQLite, migrations, repositories, blob store

postio-core       UI-agnostic contract, under both: command bus, registry,
   |              event stream, app state, undo, tokio<->glib bridge.
   +-- postio-config   TOML schema, validation, watcher, live reload
postio-model      pure domain types + JWZ threading. No storage, no protocol.
```

`postio-core` has no GTK dependency, which is what keeps a non-Linux
frontend possible later. See [`CLAUDE.md`](CLAUDE.md) for the full set of
architectural invariants CI enforces, and [`docs/decisions/`](docs/decisions/)
for the reasoning behind key choices (e.g. why `io-imap`).

## License

MIT — see [`LICENSE`](LICENSE).
