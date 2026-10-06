//! `docs/config.md` is generated from the config schema.
//!
//! `postio-config` is a set of `serde` structs with doc comments, and Rust
//! has no reflection to walk them at runtime — so unlike
//! `keybindings_doc.rs`, which renders `postio_core::registry::all()`
//! directly, this file owns a hand-written table of every documented key
//! and *asserts* that the table's paths are exactly the keys
//! [`reference_config`] serialises. Adding a field without adding its row
//! here fails this test with the field's name in the message; removing a
//! field without removing its row does too.
//!
//! ADR 0011 Q3 is the design this follows, including why: `schemars` would
//! put a schema library in the graph of every crate that depends on
//! `postio-config` (`postio-core` among them) to render one page, and
//! parsing the source with `syn` in a build script is a second, driftable
//! model of the same schema.

use std::fmt::Write as _;
use std::path::PathBuf;

use postio_config::Config;

fn document_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/config.md")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/config.md"))
}

/// One documented key: its dotted path, its TOML type, its default exactly
/// as it would appear in `config.toml`, and the prose.
struct Entry {
    path: &'static str,
    kind: &'static str,
    default: &'static str,
    description: &'static str,
}

const ENTRIES: &[Entry] = &[
    // ── [ui] ──────────────────────────────────────────────────────────
    Entry {
        path: "ui.density",
        kind: "string",
        default: "\"airy\"",
        description: "Message-list row height: `airy`, `comfortable` or `compact`.",
    },
    Entry {
        path: "ui.theme",
        kind: "string",
        default: "\"system\"",
        description: "Light/dark preference: `system` (follows the desktop), `light` or `dark`.",
    },
    Entry {
        path: "ui.show_hover_actions",
        kind: "boolean",
        default: "true",
        description: "Show per-row actions when the pointer rests over a row.",
    },
    Entry {
        path: "ui.sender_avatars",
        kind: "boolean",
        default: "true",
        description: "Show each row's sender-initials chip.",
    },
    // ── [reader] ──────────────────────────────────────────────────────
    Entry {
        path: "reader.zoom",
        kind: "integer",
        default: "100",
        description: "How large messages are drawn, in percent: one of 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250 or 300. Another value loads as the nearest step.",
    },
    // ── [sync] ────────────────────────────────────────────────────────
    Entry {
        path: "sync.check_for_mail",
        kind: "string",
        default: "\"idle\"",
        description: "How Postio learns about new mail: `idle` (hold an `IDLE` connection on INBOX for push delivery), `poll` (no `IDLE`, every mailbox reconciled on `poll_interval_secs`), or `manual` (never checks on its own).",
    },
    Entry {
        path: "sync.poll_interval_secs",
        kind: "integer",
        default: "300",
        description: "Polling interval for folders without `IDLE`, in seconds.",
    },
    Entry {
        path: "sync.max_connections",
        kind: "integer",
        default: "5",
        description: "Maximum simultaneous IMAP connections per account.",
    },
    Entry {
        path: "sync.sync_on_startup",
        kind: "boolean",
        default: "true",
        description: "Start a sync as soon as the app opens.",
    },
    Entry {
        path: "sync.body_fetch",
        kind: "string",
        default: "\"lazy\"",
        description: "When message bodies are downloaded: `lazy` (headers first, bodies backfilled) or `eager`.",
    },
    Entry {
        path: "sync.attachment_fetch",
        kind: "string",
        default: "\"on_open\"",
        description: "When an attachment's bytes are downloaded: `on_open`, `eager`, or `never`.",
    },
    Entry {
        path: "sync.max_inline_bytes",
        kind: "integer",
        default: "262144",
        description: "The largest inline part fetched with the message's text rather than left on the payload axis. A `cid:` image under this size arrives with the body, so HTML mail reads correctly offline; `0` turns the rule off.",
    },
    Entry {
        path: "sync.initial_sync_messages",
        kind: "integer",
        default: "5000",
        description: "How many messages the first sync reaches back for, newest first.",
    },
    Entry {
        path: "sync.notify",
        kind: "boolean",
        default: "true",
        description: "Master switch for desktop notifications on new mail.",
    },
    Entry {
        path: "sync.notify_roles",
        kind: "array of strings",
        default: "[\"inbox\"]",
        description: "Which mailbox roles produce a notification when mail arrives in them.",
    },
    // ── [storage] ─────────────────────────────────────────────────────
    Entry {
        path: "storage.max_bytes",
        kind: "integer",
        default: "unset (no limit)",
        description: "Ceiling on the local blob store, in bytes. Omit the key for no limit -- \
                       the store is a cache and may evict what is refetchable, never message \
                       text or drafts.",
    },
    // ── [compose] ─────────────────────────────────────────────────────
    Entry {
        path: "compose.signature_on_reply",
        kind: "string",
        default: "\"above_quote\"",
        description: "Where the signature goes on a reply: `above_quote` or `below_quote`.",
    },
    Entry {
        path: "compose.signature_on_forward",
        kind: "string",
        default: "\"above_quote\"",
        description: "Where the signature goes on a forward.",
    },
    Entry {
        path: "compose.editor",
        kind: "string",
        default: "\"\"",
        description: "Which editor the compose window's hand-off opens the draft in: an \
                      application by name (`\"BBEdit\"`), a bundle identifier \
                      (`\"com.apple.TextEdit\"`), or a path. Empty means whatever the \
                      desktop already opens a text file with. A program that needs a \
                      terminal -- `vim`, `nano` -- cannot be opened by either frontend, \
                      and Postio says so rather than appearing to do nothing.",
    },
    // ── [tui] ─────────────────────────────────────────────────────────
    Entry {
        path: "tui.preview",
        kind: "string",
        default: "\"toggle\"",
        description: "How the terminal composer shows the message it would send: `toggle` \
                       (one key swaps editor and preview) or `split` (side by side).",
    },
    Entry {
        path: "tui.mouse",
        kind: "boolean",
        default: "true",
        description: "Whether the terminal frontend takes the mouse. `false` leaves the \
                       terminal's own text selection; every key still works.",
    },
    Entry {
        path: "tui.colors",
        kind: "table",
        default: "{}",
        description: "Colour overrides for the terminal frontend, by role: `text`, `dim`, \
                       `accent`, `selection`, `focus`, `unread`, `flagged`, `link`, `quote`, \
                       `code`, `error`, `warning`, `success`. A value is a colour name, a \
                       palette number or `#rrggbb`. `NO_COLOR` overrides every one.",
    },
    // ── [logging] ─────────────────────────────────────────────────────
    Entry {
        path: "logging.level",
        kind: "string",
        default: "\"info\"",
        description: "How much to say, when `filter` does not say something more specific: \
                       `off`, `error`, `warn`, `info`, `debug` or `trace`.",
    },
    Entry {
        path: "logging.filter",
        kind: "string",
        default: "\"\"",
        description: "A per-target override in `EnvFilter` syntax, e.g. \
                       `\"postio_sync=debug,io_imap=trace\"`. Empty means \"just use `level`\".",
    },
    Entry {
        path: "logging.timestamps",
        kind: "boolean",
        default: "true",
        description: "Prefix each log line with the time it was emitted.",
    },
    // ── [focus] ───────────────────────────────────────────────────────
    Entry {
        path: "focus.filtering",
        kind: "boolean",
        default: "true",
        description: "Focus files spam and automated updates away as they arrive, each \
                       with its reason and one key from restored. `false` stops filing new \
                       mail away; what is already filtered stays where it is.",
    },
    Entry {
        path: "focus.reading",
        kind: "string",
        default: "\"dialog\"",
        description: "Where Enter opens a message in Focus: `dialog`, over the list, or \
                       `pane`, beside it. A window narrower than 980 px uses the dialog \
                       whatever this says. F8 switches it.",
    },
    // ── [focus.filter] ────────────────────────────────────────────────
    Entry {
        path: "focus.filter.never",
        kind: "array of strings",
        default: "[]",
        description: "Senders Focus never files away: pinned, or restored from Filtered. \
                       An entry is an address (`ada@example.com`) or a whole domain \
                       (`@example.com`), which covers that domain only.",
    },
    Entry {
        path: "focus.filter.stop_markers",
        kind: "array of tables",
        default: "[]",
        description: "Marker kinds stopped for a sender, each `{ sender, kind }` with `kind` \
                       `question` or `todo`. Focus writes one when a kind is dismissed three \
                       times for the same sender, and undo takes it back.",
    },
    // ── [[focus.digests]] ─────────────────────────────────────────────
    Entry {
        path: "focus.digests.name",
        kind: "string",
        default: "none (required)",
        description: "What the digest row and the rules list call it. Unique among the rules.",
    },
    Entry {
        path: "focus.digests.match",
        kind: "array of strings",
        default: "none (required)",
        description: "Queries in the one search language. The rule holds a message when any \
                       of them matches it.",
    },
    Entry {
        path: "focus.digests.cadence",
        kind: "string",
        default: "none (required)",
        description: "`daily`, `weekly` or `monthly`.",
    },
    Entry {
        path: "focus.digests.day",
        kind: "string or integer",
        default: "unset",
        description: "A weekday for a weekly rule (`saturday`), a day from 1 to 28 for a \
                       monthly one, and absent for a daily one.",
    },
    Entry {
        path: "focus.digests.at",
        kind: "string",
        default: "none (required)",
        description: "The local time it comes due, as `HH:MM`.",
    },
    // ── [focus.model] ─────────────────────────────────────────────────
    Entry {
        path: "focus.model.endpoint",
        kind: "string",
        default: "none (required)",
        description: "Where the model runtime listens, on this computer only: \
                       `http://127.0.0.1:<port>/v1`, `http://[::1]:<port>/v1`, \
                       `http://localhost:<port>/v1`, or `unix:` and a socket's absolute path. \
                       Anything else is refused, and no model is used.",
    },
    Entry {
        path: "focus.model.model",
        kind: "string",
        default: "none (required)",
        description: "The model's name, as the runtime serves it.",
    },
    Entry {
        path: "focus.model.needs_action",
        kind: "boolean",
        default: "true",
        description: "Ask the model which mail asks a question or sets a to-do, in place of \
                       the built-in detector. The detector answers whenever the model does not.",
    },
    Entry {
        path: "focus.model.digest_summary",
        kind: "boolean",
        default: "true",
        description: "Have the model write each digest's summary, every statement citing the \
                       mail it came from.",
    },
    Entry {
        path: "focus.model.like_this",
        kind: "boolean",
        default: "true",
        description: "Offer \"Digest mail like this\", which asks the model for a rule.",
    },
    // ── [focus.vault] ─────────────────────────────────────────────────
    Entry {
        path: "focus.vault.path",
        kind: "string",
        default: "none (required)",
        description: "The Obsidian vault's folder, written from `/` or `~/`.",
    },
    Entry {
        path: "focus.vault.tasks_note",
        kind: "string",
        default: "\"Tasks.md\"",
        description: "Where a task goes when no project is chosen, relative to the vault and \
                       inside it.",
    },
    Entry {
        path: "focus.vault.projects",
        kind: "string",
        default: "unset",
        description: "A folder of project notes, relative to the vault and inside it, besides \
                       the notes whose frontmatter says `type: project`.",
    },
];

/// A table inside a section that is documented as a section of its own:
/// its keys are rows under its own heading, not one row of their parent's.
/// `array` is an array of tables, `[[...]]`, written once per entry.
struct Nested {
    path: &'static str,
    array: bool,
}

const NESTED: &[Nested] = &[
    Nested {
        path: "focus.filter",
        array: false,
    },
    Nested {
        path: "focus.digests",
        array: true,
    },
    Nested {
        path: "focus.model",
        array: false,
    },
    Nested {
        path: "focus.vault",
        array: false,
    },
];

/// Every nested table filled in, as a person would write it. It is printed
/// under `[focus]`, and [`reference_config`] reads it, so an example that
/// stops parsing, or stops naming every key, fails the tests below.
const FOCUS_EXAMPLE: &str = "\
[focus]
filtering = true
reading = \"dialog\"

[focus.filter]
never = [\"ada@example.com\", \"@example.org\"]
stop_markers = [{ sender = \"grace@example.net\", kind = \"question\" }]

[[focus.digests]]
name = \"Newsletters\"
match = [\"from:news@example.org\", \"from:digest@example.net\"]
cadence = \"weekly\"
day = \"saturday\"
at = \"09:00\"

[focus.model]
endpoint = \"http://127.0.0.1:11434/v1\"
model = \"a-small-model\"
needs_action = true
digest_summary = true
like_this = true

[focus.vault]
path = \"~/Notes\"
tasks_note = \"Tasks.md\"
projects = \"Projects\"
";

/// What each section says after its table, where a row is not enough.
const PROSE: &[(&str, &str)] = &[
    (
        "ui",
        "The macOS app's appearance. The desktop app and the terminal read none \
         of these: they follow the system's light and dark, and draw rows at \
         fixed heights.\n",
    ),
    (
        "focus",
        "Focus's settings: what it files away, what it holds into digests, \
         and the model and vault it may use. The desktop app and the terminal \
         are Focus and read them; the macOS app reads none of them. The \
         contract they are built to is \
         [`specs/007-postio-focus/contracts/config.md`](../specs/007-postio-focus/contracts/config.md).\n\
         \n\
         Focus writes to this file itself, when a sender is restored, a marker \
         kind is stopped, or a digest rule is made, stopped or removed. It \
         writes the way the settings window saves: everything it does not own is \
         kept as written, and the file is replaced whole. A running app picks \
         the change up as it would an edit in `$EDITOR`.\n\
         \n\
         Every table below, filled in:\n\
         \n\
         ```toml\n",
    ),
    (
        "focus.filter",
        "Validation reports an entry that is not an address or a whole domain, \
         or a stopped marker with no sender or an unknown kind. It names the \
         entry by its position, never by its content, and that entry is ignored.\n",
    ),
    (
        "focus.digests",
        "One table per rule. The file's order is the order the rules are \
         matched in, and the first rule that matches holds the message. Mail \
         with an invitation, a question or a to-do in it is never held, and \
         neither is a conversation you have written in.\n\
         \n\
         **When a rule comes due.** The next delivery is the first time the \
         cadence, day and `at` name after the previous delivery, or after the \
         rule was made, in the local time zone. Days are counted on the \
         calendar, not as 24-hour spans, so a weekly digest keeps its time \
         across a change of the clocks. A time the clocks skip is pushed \
         forward by the gap, and a time they repeat comes due once, at its \
         first occurrence. Monthly days stop at 28, so every month has one. A \
         delivery that came due while Focus was closed is made once when it \
         next opens, and a delivery with nothing held is not made at all.\n\
         \n\
         A rule that does not validate is not applied, and the other rules \
         still are. Validation reports a missing or repeated name, a rule with \
         no query, an empty query or one the search language cannot read, an \
         unknown cadence, a day that does not fit the cadence, and a time that \
         is not one.\n",
    ),
    (
        "focus.model",
        "**Absent means off.** Without this section nothing connects to a \
         model, and Postio never looks for one: the built-in detector marks \
         questions and to-dos, and digests open on their list of mail. Postio \
         ships no model and starts none. It speaks to the one you run, through \
         the OpenAI-compatible `/chat/completions` that common local runtimes \
         serve, and names no runtime or model itself. A section with no \
         endpoint, no model, or an endpoint on another computer is reported \
         and used for nothing.\n",
    ),
    (
        "focus.vault",
        "Where Focus captures a message as a task or a note. A task is one \
         Obsidian Tasks line whose link opens the message in Focus, as \
         `postio://message/<id>`. A vault that is not a folder on this \
         computer, or a note or folder named outside it, is reported, and \
         nothing is captured until it is fixed.\n",
    ),
];

/// A `Config` built so every documented key actually serialises, for the
/// completeness check below.
///
/// [`Config::default`] alone will not do: `storage.max_bytes` is an
/// `Option<u64>` that is `None` by default, and a `None` field with no
/// `skip_serializing_if` is simply absent from the TOML `toml` writes --
/// there is no way to spell "documented, but unset" as a bare default. This
/// gives it a value purely so its path exists to compare against; the table
/// above still documents the real default, "unset".
fn reference_config() -> Config {
    let mut config = Config::default();
    config.storage.max_bytes = Some(0);
    // `[focus]`'s nested tables are empty or absent by default, and neither
    // is written out: the example fills every one of them in.
    config.focus = Config::from_toml_str(FOCUS_EXAMPLE)
        .expect("the `[focus]` example parses")
        .focus;
    config
}

/// Every key path a serialised [`reference_config`] carries.
///
/// `[section]` then its leaves, and one level further for the tables in
/// [`NESTED`], whose keys are documented as a section of their own; an
/// array of tables contributes the keys of its entries. Everything else
/// is a leaf. `[accounts]`/`[filters]`/`[mailboxes]`/`[keys]` are dynamic
/// maps that serialise as bare, empty tables with no leaves of their own to
/// collect -- they are documented as sections in the rendered prose
/// instead, not as rows in this table.
fn schema_paths() -> Vec<String> {
    let text = toml::to_string(&reference_config()).expect("Config always serialises");
    let value: toml::Value = toml::from_str(&text).expect("what was just serialised, parses");
    let toml::Value::Table(sections) = value else {
        panic!("a config document is always a table at the top level");
    };
    let mut paths = Vec::new();
    for (section, contents) in sections {
        if let toml::Value::Table(fields) = contents {
            collect(&section, &fields, &mut paths);
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn collect(prefix: &str, fields: &toml::Table, paths: &mut Vec<String>) {
    for (key, value) in fields {
        let path = format!("{prefix}.{key}");
        let nested = NESTED.iter().any(|nested| nested.path == path);
        match value {
            toml::Value::Table(inner) if nested => collect(&path, inner, paths),
            toml::Value::Array(entries) if nested => {
                for entry in entries {
                    if let toml::Value::Table(inner) = entry {
                        collect(&path, inner, paths);
                    }
                }
            }
            _ => paths.push(path),
        }
    }
}

/// What a section's heading says: `[focus.filter]`, or `[[focus.digests]]`
/// for an array of tables.
fn heading(section: &str) -> String {
    if NESTED
        .iter()
        .any(|nested| nested.path == section && nested.array)
    {
        format!("[[{section}]]")
    } else {
        format!("[{section}]")
    }
}

/// The prose after `section`'s table, if it has any. `[focus]`'s ends by
/// opening a code block for [`FOCUS_EXAMPLE`], which is closed here.
fn prose(section: &str, out: &mut String) {
    let Some((_, text)) = PROSE.iter().find(|(name, _)| *name == section) else {
        return;
    };
    out.push('\n');
    out.push_str(text);
    if section == "focus" {
        out.push_str(FOCUS_EXAMPLE);
        out.push_str("```\n");
    }
}

fn render() -> String {
    let mut out = String::new();
    out.push_str(
        "# Configuration reference\n\
         \n\
         <!-- Generated from `postio-config`'s schema by\n\
         `crates/postio-config/tests/config_suite/config_doc.rs`. Do not edit by hand:\n\
         change the schema and run `POSTIO_UPDATE_DOCS=1 cargo test -p postio-config`. -->\n\
         \n\
         `~/.config/postio/config.toml` is the settings -- there is no separate\n\
         store. A missing or empty file is not an error: every key below has a\n\
         working default, and Postio writes a starter file on first run so\n\
         there is something to find and edit rather than a blank buffer. The\n\
         file is watched and re-parsed live; a key this build does not\n\
         recognise survives a round trip untouched, in case a newer Postio\n\
         wrote it.\n\
         \n",
    );

    let mut section = "";
    for entry in ENTRIES {
        let (this_section, key) = entry.path.rsplit_once('.').expect("path has a section");
        if this_section != section {
            if !section.is_empty() {
                prose(section, &mut out);
                out.push('\n');
            }
            section = this_section;
            let _ = writeln!(out, "## `{}`\n", heading(section));
            let _ = writeln!(out, "| Key | Type | Default | Description |");
            let _ = writeln!(out, "|---|---|---|---|");
        }
        let _ = writeln!(
            out,
            "| `{key}` | {} | `{}` | {} |",
            entry.kind, entry.default, entry.description
        );
    }
    prose(section, &mut out);
    out.push('\n');

    out.push_str(
        "## `[keys]`\n\
         \n\
         Overrides a command's binding, keyed by the command id. See the\n\
         [keyboard reference](keybindings.md) for every id and its default.\n\
         \n\
         ```toml\n\
         [keys]\n\
         archive = \"w\"\n\
         first_message = \"g g\"\n\
         command_palette = \"mod+p\"\n\
         ```\n\
         \n\
         `mod` is the primary accelerator -- Control on Linux, Command on macOS --\n\
         so one file means the same thing on both. Write `ctrl` when you mean the\n\
         Control key specifically; it stays literal everywhere.\n\
         \n\
         ## `[accounts.<id>]`\n\
         \n\
         One table per account, keyed by a short id you choose. Servers,\n\
         security and the login name -- never a password, which lives in the\n\
         OS keyring and never touches this file.\n\
         \n\
         ```toml\n\
         [accounts.personal]\n\
         email = \"ada@example.com\"\n\
         display_name = \"Personal\"\n\
         default = true\n\
         \n\
         [accounts.personal.imap]\n\
         host = \"imap.example.com\"\n\
         port = 993\n\
         security = \"implicit-tls\"\n\
         \n\
         [accounts.personal.smtp]\n\
         host = \"smtp.example.com\"\n\
         port = 465\n\
         security = \"implicit-tls\"\n\
         ```\n\
         \n\
         ## `[filters.<id>]`\n\
         \n\
         A named, pinned search -- one table per saved search, keyed the same\n\
         way accounts are.\n\
         \n\
         ## `[mailboxes]`\n\
         \n\
         Maps a role Postio already knows (`archive`, `sent`, `trash`, ...) to\n\
         the exact folder path your server uses for it, when autodetection\n\
         guesses wrong. Keyed by role, valued by path -- the way `[keys]` is\n\
         keyed by the thing you mean and valued by its spelling.\n\
         \n\
         ```toml\n\
         [mailboxes]\n\
         archive = \"Archive/2024\"\n\
         ```\n\
         \n\
         **This table applies to every account.** That is the right default for\n\
         the ordinary installation, which has one account, and the wrong one the\n\
         moment two accounts disagree about where their sent mail lives -- a fix\n\
         for iCloud that breaks Gmail on the same machine. So it is the\n\
         *default*, not the answer: **each account can map a role itself, in\n\
         Settings -> Accounts, and its own choice wins** (ADR 0035).\n\
         \n\
         The full precedence, per account and per role:\n\
         \n\
         1. the account's own map, chosen in Settings -> Accounts\n\
         2. this `[mailboxes]` table\n\
         3. the server's `SPECIAL-USE` attribute\n\
         4. a guess from the folder's name\n\
         \n\
         Two consequences worth knowing:\n\
         \n\
         - **A choice made in settings takes effect on the next sync pass**,\n\
           because discovery reads the store's map every time. Editing this file\n\
           needs a restart, because the file is read once at startup.\n\
         - **A mapping that names a folder the account no longer has is shown as\n\
           dangling in Settings -> Accounts** rather than silently ignored. A\n\
           role quietly falling back to a guess is how mail ends up filed\n\
           somewhere the user did not choose and cannot see they did not\n\
           choose.\n\
         \n\
         Nothing here moves mail. Re-pointing a role changes which folder wears\n\
         the label from that moment on; the messages already in the old folder\n\
         stay where they are.\n\
         \n\
         **Every account ends up with a folder for all six roles.** When one\n\
         resolves to nothing after all four tiers, Postio creates it on the\n\
         server -- once, never for the Inbox, and named after the role. A\n\
         server that refuses is not asked again: the role is shown as unmapped\n\
         in Settings -> Accounts with the server's own words beside it, which\n\
         is usually a permission and usually something you can fix.\n",
    );

    out
}

#[test]
fn the_config_reference_matches_the_schema() {
    let path = document_path();
    let rendered = render();

    if std::env::var_os("POSTIO_UPDATE_DOCS").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create docs/");
        }
        std::fs::write(&path, &rendered).expect("write the config reference");
        return;
    }

    let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}\nrun `POSTIO_UPDATE_DOCS=1 cargo test -p postio-config` to generate it",
            path.display()
        )
    });

    assert_eq!(
        on_disk, rendered,
        "docs/config.md is out of date with config_doc.rs's own table; \
         run `POSTIO_UPDATE_DOCS=1 cargo test -p postio-config`"
    );
}

#[test]
fn every_documented_path_is_a_real_key_and_every_real_key_is_documented() {
    let documented: Vec<String> = ENTRIES.iter().map(|entry| entry.path.to_owned()).collect();
    let mut documented_sorted = documented.clone();
    documented_sorted.sort();

    let real = schema_paths();

    for path in &real {
        assert!(
            documented.contains(path),
            "`{path}` is a real config key with no row in config_doc.rs's ENTRIES table"
        );
    }
    for path in &documented {
        assert!(
            real.contains(path),
            "config_doc.rs documents `{path}`, which does not exist in the schema -- \
             a stale row, or a typo"
        );
    }
    assert_eq!(
        documented_sorted, real,
        "ENTRIES and the schema name the same keys, but not exactly this multiset -- \
         check for a duplicated row"
    );
}
