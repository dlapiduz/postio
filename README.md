# Postio

[![CI](https://github.com/dlapiduz/postio/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/dlapiduz/postio/actions/workflows/ci.yml)
[![Nightly](https://github.com/dlapiduz/postio/actions/workflows/nightly.yml/badge.svg)](https://github.com/dlapiduz/postio/actions/workflows/nightly.yml)
[![Latest release](https://github.com/dlapiduz/postio/actions/workflows/release.yml/badge.svg)](https://github.com/dlapiduz/postio/releases)

<!-- Only GitHub's own badges: a shields.io or Codecov image would tell a
     third party the address of everyone who reads this page, which is the
     thing this README is promising Postio never does (#101, ADR 0011 §5). -->

**A local-first, keyboard-first email client for people who have too much
email.** Read less. Find anything. Act faster.

![Postio reading a conversation: the folder list, the message list and a
threaded reading pane, all driven from the keyboard](site/assets/img/conversation.png)

Postio keeps a complete, encrypted copy of your mail on your own machine,
with a full-text index built beside it. Opening the app, searching, and
moving around never wait on the network. Every action — archive, flag, move,
delete, snooze, undo — lands on that local copy instantly and reaches the
server in the background. It is a native GTK4/libadwaita application on Linux
and a native SwiftUI/AppKit one on macOS — **two frontends over one Rust
engine**, not a toolkit ported — works fully offline after its first sync, and
never sends anything you did not ask it to.

**Postio 0.4.0 is an alpha.** It is more complete than the number suggests,
but it is early software: expect rough edges, read the
[status](#whats-in-04-and-what-is-not) section before switching, and keep
your current client around until you trust it.

- **Home page and tour:** <https://dlapiduz.github.io/postio/>
- **User guide:** <https://dlapiduz.github.io/postio/docs/> — installing,
  every key, `config.toml`, how sync works, privacy, FAQ
- **Releases:** <https://github.com/dlapiduz/postio/releases>

## Why Postio

- **It is instant.** The inbox, a thread, a search result: all of it is read
  from the local store, never fetched live. Startup, navigation and search
  are held to real budgets and the *cause* of each budget is counted in the
  test suite, not just timed on one machine.
- **Search is how you move.** One query language works everywhere it
  appears: typed into the search box, saved as a folder in the sidebar, or
  written into `config.toml`. `from:ada has:attach after:2026-01-01` is a
  query you can type, and results appear as you type it.
- **Every action has a key.** `j`/`k` move, `e` replies, `a` archives, `u`
  undoes anything, `/` searches, `Ctrl+K` opens the command palette, `?`
  shows the cheat sheet. Every binding is rebindable. The mouse works too and
  is never required.
- **All your accounts, one inbox.** Several IMAP, Gmail or JMAP accounts,
  each synced by its own engine, grouped into one unified inbox at read
  time. Sign in with a password, an app-specific password, or OAuth 2 through
  your browser.
- **Private by design.** Remote images and tracking pixels stay blocked
  until you allow them per sender. Read receipts are never automatic.
  Unsubscribe links fire only when you click them. No telemetry, no crash
  reporting, no update ping. The reader runs no script that arrived in a
  message.
- **Encrypted at rest.** The local store is encrypted and the key lives in
  your OS keyring, so a stray backup or a stolen disk holds ciphertext.
  Credentials go in the keyring too, never in a config file or a log.
- **Built for triage.** Select a run of messages and the list header becomes
  the action bar. Snooze, schedule a send, fold quoted text, walk a
  conversation with `J`/`K`, archive a whole thread with `A`.

![Search results with the scope and refinement panels, and a matched message
previewed with its hits highlighted](site/assets/img/search.png)

## Install

Postio runs on Linux under Wayland (GTK 4.20 and libadwaita 1.7 or newer;
the maintainer's machine is Fedora 44), and on macOS 14 or newer. The Linux
build is the complete one and has the three ways in below; the macOS build is
[further down](#macos), is younger, and is honest about what it cannot do
yet.

### 1. The Flatpak bundle

Every tagged release publishes a prebuilt `.flatpak` bundle on the
[Releases page](https://github.com/dlapiduz/postio/releases), with a signed
build-provenance attestation and a software bill of materials beside it.

```bash
# Download postio-<version>-x86_64.flatpak from the Releases page, then:
flatpak install --user ./postio-<version>-x86_64.flatpak
flatpak run dev.postio.Postio
```

The bundle names Flathub as its runtime source, so `flatpak` fetches the
GNOME runtime on its own if you do not have it yet. A mail client holds your
credentials and your mail, so it is worth checking that a downloaded bundle
was built by this project's release workflow from the tagged commit:

```bash
gh attestation verify postio-<version>-x86_64.flatpak --repo dlapiduz/postio
```

Postio is not on Flathub yet; when it is, this section collapses to one
`flatpak install` line.

### 2. From source

System dependencies on Fedora:

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 libsecret-devel glib2-devel pkgconf-pkg-config
```

On Ubuntu 26.04 or newer (earlier releases ship a GTK older than Postio's
floor):

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
                 libwebkitgtk-6.0-dev libsecret-1-dev \
                 libglib2.0-dev libpango1.0-dev
```

Rust is pinned by [`rust-toolchain.toml`](rust-toolchain.toml); with
[rustup](https://rustup.rs) installed, the right compiler arrives on the
first `cargo` command. Then:

```bash
git clone https://github.com/dlapiduz/postio.git
cd postio
scripts/install-local.sh               # builds --release, installs to ~/.local
scripts/install-local.sh --uninstall   # removes exactly what it installed
```

That puts `postio` on your `$PATH` and Postio in your app grid with its
icon. The script checks the build dependencies first and names every missing
one at once. Prefer to build the Flatpak yourself? The manifest and the
one-time SDK setup are in [`flatpak/README.md`](flatpak/README.md).

### 3. Just try it

```bash
cargo run -p postio-app
```

builds and runs Postio from the checkout without installing anything.

### macOS

A native SwiftUI/AppKit application over the same engine, through a UniFFI
boundary ([ADR 0019](docs/decisions/0019-macos-frontend.md)). **It lives on
the `feature/macos` branch until it is closer to parity** ([#1306](https://github.com/dlapiduz/postio/pull/1306));
the crates under it are on `main` and are built and tested by CI on both
platforms. Thirteen of the
fifteen workspace crates already built and tested on macOS before any porting
began, which is what made this cheap: the two frontends share the store, the
protocols, the search index, the keymap and every presentation decision that
has no toolkit in it.

Needs Xcode (Swift 6) and the pinned Rust toolchain. There is no bundle to
download yet — it is built from the checkout:

```bash
scripts/macos-build.sh      # cargo, the bindings, then swift build
scripts/macos-bundle.sh     # assemble Postio.app
open macos/build/Postio.app
```

`scripts/macos-test.sh` runs the Swift tests with the library linked.
`macos/CLAUDE.md` has the two build loops and the Keychain behaviour of an
ad-hoc-signed build, which asks again after every rebuild.

**What works:** the three-pane shell, the message list over the paged store,
the reading pane with its remote-image blocking and per-sender allow list,
conversations, search with its query language and the chips that teach it,
the command palette, the cheat sheet, the menu bar built from the command
registry, keyboard commands in both layers, compose with rich text,
attachments and pictures in the body, scheduled send, notifications, a
settings window with every pane, the message-parts panel — so an
attachment can be saved, saved
alongside its siblings, previewed in place or handed to another
application — saved searches, the account verbs, one-click unsubscribe
with its activation log in Privacy settings, and repairing an account
whose credential has expired, by password or by browser, whichever it
broke by.

**What does not, yet.** Three commands in the registry reach nothing here,
and each is a decision rather than a wire:

- `detach_composer` has nothing to detach: compose on macOS is already a
  window of its own and never takes over the reading pane
  ([#1571](https://github.com/dlapiduz/postio/issues/1571)).
- `next_scope` cycles an account strip, and this sidebar lists every
  account's folders at once rather than re-rooting to one
  ([#1573](https://github.com/dlapiduz/postio/issues/1573)).
- `toggle_rail` needs the conversation rail, which is not drawn here yet
  ([#1576](https://github.com/dlapiduz/postio/issues/1576)) — and the rail
  is built on ADR 0032's one-document pane, which this frontend has not
  adopted ([#1595](https://github.com/dlapiduz/postio/issues/1595)).

Beyond the command sweep: the search bar has its chips, hit count, timing,
refine chips, sort control and footer hints, but not the scope rail
([#1157](https://github.com/dlapiduz/postio/issues/1157)) — which is the
same design question as `next_scope` above.

`crates/postio-ffi/tests/ffi_suite/command_coverage.rs` is what keeps that
second list honest: it sweeps every command in the registry and fails if one
reaches nothing and is not listed as debt. The list has gone from
forty-nine to three, and it may only shrink — a command that gains a handler
and stays listed fails the sweep just as one that loses a handler does.

## First run

Postio opens on a one-screen setup. Type your email address and the
autoconfig probe fills in the server settings: a built-in provider table
first, then Thunderbird's autoconfig service, then DNS SRV records, or you
can enter everything by hand. Providers that require OAuth 2 (Gmail,
Microsoft 365) open your browser to sign in; everything else takes a
password or an app-specific password. The credential goes straight into
your desktop keyring and is never written to a file.

The first sync brings the newest mail in first and then backfills every
folder to completion in the background, so search and offline reading
eventually cover your whole mailbox. Attachments download when you open
them, not proactively; `config.toml` can change both behaviours.

Add a second account any time with `Ctrl+Shift+N`.

Postio registers itself for `mailto:` links, so a link clicked in a browser
or a "share by email" from another application opens the composer with the
address, subject and body filled in. To make it the default mail client:

```bash
xdg-mime default dev.postio.Postio.desktop x-scheme-handler/mailto
```

## Everyday use

| Keys | Does |
|---|---|
| `j` / `k` | Next / previous message |
| `Enter` or `l` | Open the message or conversation |
| `e` / `E` / `f` | Reply / reply all / forward |
| `a` / `A` | Archive the message / the whole thread |
| `d`, `m`, `s`, `L` | Delete, move to…, flag, add a label |
| `b` / `B` | Snooze / unsnooze |
| `x`, `J` / `K` | Select this row, extend the selection down / up |
| `u` | Undo the last action, however many rows it touched |
| `/` | Search all mail (`>` runs a command, `#` jumps to a folder, `@` finds a person) |
| `Ctrl+S` in a search | Save the search as a folder in the sidebar |
| `c` | Compose (`Ctrl+Enter` sends, `Ctrl+Shift+Enter` schedules) |
| `g i`, `g d`, `g t`, `g s` | Go to inbox, drafts, sent, flagged |
| `g a` | Switch between an account and the unified inbox |
| `Ctrl+K` | Command palette, every command by name |
| `?` | The cheat sheet |

Search operators compose, and a leading `-` negates:

```
from:ada after:2026-01-01 has:attach
subject:invoice -in:archive is:unread
```

`from:` `to:` `subject:` `in:` `list:` `filename:` `has:attach` `is:unread`
`is:read` `is:flagged` `before:` `after:` `larger:` `smaller:` `account:`
`group:` `header:` `body:`

A search covers every folder except drafts, junk and trash; `in:trash` (or
`in:junk`, `in:drafts`) reaches those when you mean them.

The complete, generated keyboard reference is
[`docs/keybindings.md`](docs/keybindings.md); every binding can be changed
in `config.toml`.

![Replying inside the reading pane, with the quoted message folded under the
reply and the message list still visible](site/assets/img/compose.png)

## Configuration

Postio reads `config.toml` from `$POSTIO_CONFIG`, or
`$XDG_CONFIG_HOME/postio/config.toml`, or `~/.config/postio/config.toml`. A
missing file is fine — first run needs nothing on disk — and the file is
live-reloaded on save. The settings window edits the same file.

```toml
[ui]
density = "airy"          # airy | comfortable | compact
theme = "system"          # system | light | dark

[sync]
check_for_mail = "idle"   # idle (push) | poll
attachment_fetch = "on_open"

[keys]
archive = "y"             # overrides the default binding for `archive`

[filters.needs-reply]
query  = "is:unread from:team"
pinned = true             # shows in the sidebar as a folder
```

Every key, type and default is in [`docs/config.md`](docs/config.md).
**No credential ever lives in `config.toml`**: an account in the file only
references a keyring entry.

## What's in 0.4 and what is not

**In:** several accounts with a unified inbox; IMAP + SMTP, Gmail and JMAP
backends; password, app-specific password and OAuth 2 sign-in; folders,
threads and labels; read, archive, delete, flag, move, snooze; HTML and
plain-text reading with attachments and quoted-text folding; rich-text
compose, reply, reply-all, forward, drafts, scheduled send and an outbox
that never sends twice; local full-text search with operators, saved
searches and virtual folders; contacts and contact groups grown from your
mail; notifications; vim-style keys, a command palette, every binding
rebindable; background sync with IDLE, full offline use, and undo.

**Not yet:** filters and rules (designed, not built); AI features (a
founding idea, deliberately after the fundamentals); Microsoft Graph;
PGP/S-MIME; phishing and link warnings; vCard import/export. The native macOS
frontend reads and writes mail today and is in the repository, but is not at
parity with the Linux build and is not released — see
[macOS](#macos) for what it can and cannot do. Windows is unscheduled.

Postio is alpha software. Its test suite is large and its invariants are
machine-checked, and you should still treat it as early: it has met few
mailboxes so far, and the issue tracker is where the rough edges get filed.

## Documentation

| For | Read |
|---|---|
| Installing and using Postio | the [user guide](https://dlapiduz.github.io/postio/docs/) (source in [`docs/book/`](docs/book/)) |
| Every key | [`docs/keybindings.md`](docs/keybindings.md) |
| Every config key | [`docs/config.md`](docs/config.md) |
| What Postio must do, and must not | [`docs/PRODUCT.md`](docs/PRODUCT.md) |
| How it is put together, and why | [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and the ADRs in [`docs/decisions/`](docs/decisions/) |
| The performance budgets and what was measured | [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) |
| Hard-won lessons | [`docs/engineering-notes.md`](docs/engineering-notes.md) |
| Contributing, and the developer setup | [`CONTRIBUTING.md`](CONTRIBUTING.md) |
| The agent workflow and the gates | [`CLAUDE.md`](CLAUDE.md) |

## How Postio is built

Twenty crates in strict layers: a GTK view layer that speaks no SQL and no
protocol, an engine that owns the local store and the network, and a
UI-agnostic contract between them — commands down, events up, and the UI
never awaits the network. The boundaries are checked against cargo's
resolved dependency graph on every pull request, not left to convention.
The store is [Turso](https://github.com/tursodatabase/turso) with its
full-text index, encrypted page by page; raw messages and attachments live in
a content-addressed blob store beside it.

**This codebase is almost entirely AI-generated.** Postio is written by AI
coding agents — Claude, in parallel sessions — under a human maintainer who
sets scope, reviews the results and makes the product decisions. That is the
experiment, not a disclaimer: not whether an agent can emit code, but whether
a *process* can make agent-written software trustworthy. Every piece of work
is an issue worked on its own branch and landed as a pull request;
test-driven development is mandatory; the invariants — no GTK in the engine,
no SQL in the view layer, no message content in a log, a destructive command
must be undoable — are scripts that run on every landing, because a rule an
agent has to remember is a rule that drifts. Decisions are written down as
[ADRs](docs/decisions/), in public.

Read the code with the same scepticism you would give any codebase, and if
you find something wrong, the [issue
tracker](https://github.com/dlapiduz/postio/issues) is where this project
thinks. [`CONTRIBUTING.md`](CONTRIBUTING.md) says how to file an issue an
agent can act on, and how to set up a machine to build and test.

## License

MIT — see [`LICENSE`](LICENSE).
