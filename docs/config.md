# Configuration reference

<!-- Generated from `postio-config`'s schema by
`crates/postio-config/tests/config_suite/config_doc.rs`. Do not edit by hand:
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
| `sender_avatars` | boolean | `true` | Show each row's sender-initials chip. |

The macOS app's appearance. The desktop app and the terminal read none of these: they follow the system's light and dark, and draw rows at fixed heights.

## `[reader]`

| Key | Type | Default | Description |
|---|---|---|---|
| `zoom` | integer | `100` | How large messages are drawn, in percent: one of 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250 or 300. Another value loads as the nearest step. |

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
| `editor` | string | `""` | Which editor the compose window's hand-off opens the draft in: an application by name (`"BBEdit"`), a bundle identifier (`"com.apple.TextEdit"`), or a path. Empty means whatever the desktop already opens a text file with. A program that needs a terminal -- `vim`, `nano` -- cannot be opened by either frontend, and Postio says so rather than appearing to do nothing. |

## `[tui]`

| Key | Type | Default | Description |
|---|---|---|---|
| `preview` | string | `"toggle"` | How the terminal composer shows the message it would send: `toggle` (one key swaps editor and preview) or `split` (side by side). |
| `mouse` | boolean | `true` | Whether the terminal frontend takes the mouse. `false` leaves the terminal's own text selection; every key still works. |
| `colors` | table | `{}` | Colour overrides for the terminal frontend, by role: `text`, `dim`, `accent`, `selection`, `focus`, `unread`, `flagged`, `link`, `quote`, `code`, `error`, `warning`, `success`. A value is a colour name, a palette number or `#rrggbb`. `NO_COLOR` overrides every one. |

## `[logging]`

| Key | Type | Default | Description |
|---|---|---|---|
| `level` | string | `"info"` | How much to say, when `filter` does not say something more specific: `off`, `error`, `warn`, `info`, `debug` or `trace`. |
| `filter` | string | `""` | A per-target override in `EnvFilter` syntax, e.g. `"postio_sync=debug,io_imap=trace"`. Empty means "just use `level`". |
| `timestamps` | boolean | `true` | Prefix each log line with the time it was emitted. |

## `[focus]`

| Key | Type | Default | Description |
|---|---|---|---|
| `filtering` | boolean | `true` | Focus files spam and automated updates away as they arrive, each with its reason and one key from restored. `false` stops filing new mail away; what is already filtered stays where it is. |
| `reading` | string | `"dialog"` | Where Enter opens a message in Focus: `dialog`, over the list, or `pane`, beside it. A window narrower than 980 px uses the dialog whatever this says. F8 switches it. |

Focus's settings: what it files away, what it holds into digests, and the model and vault it may use. The desktop app and the terminal are Focus and read them; the macOS app reads none of them. The contract they are built to is [`specs/007-postio-focus/contracts/config.md`](../specs/007-postio-focus/contracts/config.md).

Focus writes to this file itself, when a sender is restored, a marker kind is stopped, or a digest rule is made, stopped or removed. It writes the way the settings window saves: everything it does not own is kept as written, and the file is replaced whole. A running app picks the change up as it would an edit in `$EDITOR`.

Every table below, filled in:

```toml
[focus]
filtering = true
reading = "dialog"

[focus.filter]
never = ["ada@example.com", "@example.org"]
stop_markers = [{ sender = "grace@example.net", kind = "question" }]

[[focus.digests]]
name = "Newsletters"
match = ["from:news@example.org", "from:digest@example.net"]
cadence = "weekly"
day = "saturday"
at = "09:00"

[focus.model]
endpoint = "http://127.0.0.1:11434/v1"
model = "a-small-model"
needs_action = true
digest_summary = true
like_this = true

[focus.vault]
path = "~/Notes"
tasks_note = "Tasks.md"
projects = "Projects"
```

## `[focus.filter]`

| Key | Type | Default | Description |
|---|---|---|---|
| `never` | array of strings | `[]` | Senders Focus never files away: pinned, or restored from Filtered. An entry is an address (`ada@example.com`) or a whole domain (`@example.com`), which covers that domain only. |
| `stop_markers` | array of tables | `[]` | Marker kinds stopped for a sender, each `{ sender, kind }` with `kind` `question` or `todo`. Focus writes one when a kind is dismissed three times for the same sender, and undo takes it back. |

Validation reports an entry that is not an address or a whole domain, or a stopped marker with no sender or an unknown kind. It names the entry by its position, never by its content, and that entry is ignored.

## `[[focus.digests]]`

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | string | `none (required)` | What the digest row and the rules list call it. Unique among the rules. |
| `match` | array of strings | `none (required)` | Queries in the one search language. The rule holds a message when any of them matches it. |
| `cadence` | string | `none (required)` | `daily`, `weekly` or `monthly`. |
| `day` | string or integer | `unset` | A weekday for a weekly rule (`saturday`), a day from 1 to 28 for a monthly one, and absent for a daily one. |
| `at` | string | `none (required)` | The local time it comes due, as `HH:MM`. |

One table per rule. The file's order is the order the rules are matched in, and the first rule that matches holds the message. Mail with an invitation, a question or a to-do in it is never held, and neither is a conversation you have written in.

**When a rule comes due.** The next delivery is the first time the cadence, day and `at` name after the previous delivery, or after the rule was made, in the local time zone. Days are counted on the calendar, not as 24-hour spans, so a weekly digest keeps its time across a change of the clocks. A time the clocks skip is pushed forward by the gap, and a time they repeat comes due once, at its first occurrence. Monthly days stop at 28, so every month has one. A delivery that came due while Focus was closed is made once when it next opens, and a delivery with nothing held is not made at all.

A rule that does not validate is not applied, and the other rules still are. Validation reports a missing or repeated name, a rule with no query, an empty query or one the search language cannot read, an unknown cadence, a day that does not fit the cadence, and a time that is not one.

## `[focus.model]`

| Key | Type | Default | Description |
|---|---|---|---|
| `endpoint` | string | `none (required)` | Where the model runtime listens, on this computer only: `http://127.0.0.1:<port>/v1`, `http://[::1]:<port>/v1`, `http://localhost:<port>/v1`, or `unix:` and a socket's absolute path. Anything else is refused, and no model is used. |
| `model` | string | `none (required)` | The model's name, as the runtime serves it. |
| `needs_action` | boolean | `true` | Ask the model which mail asks a question or sets a to-do, in place of the built-in detector. The detector answers whenever the model does not. |
| `digest_summary` | boolean | `true` | Have the model write each digest's summary, every statement citing the mail it came from. |
| `like_this` | boolean | `true` | Offer "Digest mail like this", which asks the model for a rule. |

**Absent means off.** Without this section nothing connects to a model, and Postio never looks for one: the built-in detector marks questions and to-dos, and digests open on their list of mail. Postio ships no model and starts none. It speaks to the one you run, through the OpenAI-compatible `/chat/completions` that common local runtimes serve, and names no runtime or model itself. A section with no endpoint, no model, or an endpoint on another computer is reported and used for nothing.

## `[focus.vault]`

| Key | Type | Default | Description |
|---|---|---|---|
| `path` | string | `none (required)` | The Obsidian vault's folder, written from `/` or `~/`. |
| `tasks_note` | string | `"Tasks.md"` | Where a task goes when no project is chosen, relative to the vault and inside it. |
| `projects` | string | `unset` | A folder of project notes, relative to the vault and inside it, besides the notes whose frontmatter says `type: project`. |

Where Focus captures a message as a task or a note. A task is one Obsidian Tasks line whose link opens the message in Focus, as `postio://message/<id>`. A vault that is not a folder on this computer, or a note or folder named outside it, is reported, and nothing is captured until it is fixed.

## `[keys]`

Overrides a command's binding, keyed by the command id. See the
[keyboard reference](keybindings.md) for every id and its default.

```toml
[keys]
archive = "w"
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

## `[saved_searches.<id>]`

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
Settings -> Accounts, and its own choice wins** (ADR 0035).

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

**Every account ends up with a folder for all six roles.** When one
resolves to nothing after all four tiers, Postio creates it on the
server -- once, never for the Inbox, and named after the role. A
server that refuses is not asked again: the role is shown as unmapped
in Settings -> Accounts with the server's own words beside it, which
is usually a permission and usually something you can fix.
