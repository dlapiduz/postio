# Configuration reference

<!-- Generated from `postio-config`'s schema by
`crates/postio-config/tests/config_doc.rs`. Do not edit by hand:
change the schema and run `POSTIO_UPDATE_DOCS=1 cargo test -p postio-config`. -->

`~/.config/postio/config.toml` is the settings -- there is no separate
store. A missing or empty file is not an error: every key below has a
working default, and Postio writes a starter file on first run so
there is something to find and edit rather than a blank buffer. The
file is watched and re-parsed live; a key this build does not
recognise survives a round trip untouched, in case a newer Postio
wrote it.

## `[ui]`

| Key | Type | Default | Description |
|---|---|---|---|
| `density` | string | `"airy"` | Message-list row height: `airy`, `comfortable` or `compact`. |
| `theme` | string | `"system"` | Light/dark preference: `system` (follows the desktop), `light` or `dark`. |
| `show_hover_actions` | boolean | `true` | Show per-row actions when the pointer rests over a row. |
| `thread_drill` | boolean | `true` | Let `t` drill the list column into the focused thread. |
| `show_key_hints` | boolean | `true` | Show the focused row's key hints (`e reply`, `a archive`, `t thread`). Off leaves every binding in force -- this only stops the row from naming them. |
| `sender_avatars` | boolean | `true` | Show each row's sender-initials chip. |

## `[sync]`

| Key | Type | Default | Description |
|---|---|---|---|
| `check_for_mail` | string | `"idle"` | How Postio learns about new mail: `idle` (hold an `IDLE` connection on INBOX for push delivery), `poll` (no `IDLE`, every mailbox reconciled on `poll_interval_secs`), or `manual` (never checks on its own). |
| `poll_interval_secs` | integer | `300` | Polling interval for folders without `IDLE`, in seconds. |
| `max_connections` | integer | `5` | Maximum simultaneous IMAP connections per account. |
| `sync_on_startup` | boolean | `true` | Start a sync as soon as the app opens. |
| `body_fetch` | string | `"lazy"` | When message bodies are downloaded: `lazy` (headers first, bodies backfilled) or `eager`. |
| `attachment_fetch` | string | `"on_open"` | When an attachment's bytes are downloaded: `on_open`, `eager`, or `never`. |
| `max_inline_bytes` | integer | `262144` | The largest inline part fetched with the message's text rather than left on the payload axis. A `cid:` image under this size arrives with the body, so HTML mail reads correctly offline; `0` turns the rule off. |
| `initial_sync_messages` | integer | `5000` | How many messages the first sync reaches back for, newest first. |
| `notify` | boolean | `true` | Master switch for desktop notifications on new mail. |
| `notify_roles` | array of strings | `["inbox"]` | Which mailbox roles produce a notification when mail arrives in them. |

## `[storage]`

| Key | Type | Default | Description |
|---|---|---|---|
| `max_bytes` | integer | `unset (no limit)` | Ceiling on the local blob store, in bytes. Omit the key for no limit -- the store is a cache and may evict what is refetchable, never message text or drafts. |

## `[compose]`

| Key | Type | Default | Description |
|---|---|---|---|
| `signature_on_reply` | string | `"above_quote"` | Where the signature goes on a reply: `above_quote` or `below_quote`. |
| `signature_on_forward` | string | `"above_quote"` | Where the signature goes on a forward. |

## `[logging]`

| Key | Type | Default | Description |
|---|---|---|---|
| `level` | string | `"info"` | How much to say, when `filter` does not say something more specific: `off`, `error`, `warn`, `info`, `debug` or `trace`. |
| `filter` | string | `""` | A per-target override in `EnvFilter` syntax, e.g. `"postio_sync=debug,io_imap=trace"`. Empty means "just use `level`". |
| `timestamps` | boolean | `true` | Prefix each log line with the time it was emitted. |

## `[keys]`

Overrides a command's binding, keyed by the command id. See the
[keyboard reference](keybindings.md) for every id and its default.

```toml
[keys]
archive = "y"
first_message = "g g"
command_palette = "mod+p"
```

`mod` is the primary accelerator -- Control on Linux, Command on macOS --
so one file means the same thing on both. Write `ctrl` when you mean the
Control key specifically; it stays literal everywhere.

## `[accounts.<id>]`

One table per account, keyed by a short id you choose. Servers,
security and the login name -- never a password, which lives in the
OS keyring and never touches this file.

```toml
[accounts.personal]
email = "ada@example.com"
display_name = "Personal"
default = true

[accounts.personal.imap]
host = "imap.example.com"
port = 993
security = "implicit-tls"

[accounts.personal.smtp]
host = "smtp.example.com"
port = 465
security = "implicit-tls"
```

## `[filters.<id>]`

A named, pinned search -- one table per saved search, keyed the same
way accounts are.

## `[mailboxes]`

Maps a role Postio already knows (`archive`, `sent`, `trash`, ...) to
the exact folder path your server uses for it, when autodetection
guesses wrong. Keyed by role, valued by path -- the way `[keys]` is
keyed by the thing you mean and valued by its spelling.

```toml
[mailboxes]
archive = "Archive/2024"
```

**This table applies to every account.** That is the right default for
the ordinary installation, which has one account, and the wrong one the
moment two accounts disagree about where their sent mail lives -- a fix
for iCloud that breaks Gmail on the same machine. So it is the
*default*, not the answer: **each account can map a role itself, in
Settings -> Accounts, and its own choice wins** (ADR 0027).

The full precedence, per account and per role:

1. the account's own map, chosen in Settings -> Accounts
2. this `[mailboxes]` table
3. the server's `SPECIAL-USE` attribute
4. a guess from the folder's name

Two consequences worth knowing:

- **A choice made in settings takes effect on the next sync pass**,
because discovery reads the store's map every time. Editing this file
needs a restart, because the file is read once at startup.
- **A mapping that names a folder the account no longer has is shown as
dangling in Settings -> Accounts** rather than silently ignored. A
role quietly falling back to a guess is how mail ends up filed
somewhere the user did not choose and cannot see they did not
choose.

Nothing here moves mail. Re-pointing a role changes which folder wears
the label from that moment on; the messages already in the old folder
stay where they are.
