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
        path: "ui.show_key_hints",
        kind: "boolean",
        default: "true",
        description: "Show the focused row's key hints (`e reply`, `a archive`). \
                       Off leaves every binding in force -- this only stops the row from \
                       naming them.",
    },
    Entry {
        path: "ui.sender_avatars",
        kind: "boolean",
        default: "true",
        description: "Show each row's sender-initials chip.",
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
    config
}

/// Every `section.key` path a serialised [`reference_config`] carries.
///
/// Two levels only: `[section]` then its leaves. Nothing in the schema
/// nests deeper than that, and `[accounts]`/`[filters]`/`[mailboxes]`/`[keys]`
/// are dynamic maps that serialise as bare, empty tables with no leaves of
/// their own to collect -- they are documented as sections in the rendered
/// prose instead, not as rows in this table.
fn schema_paths() -> Vec<String> {
    let text = toml::to_string(&reference_config()).expect("Config always serialises");
    let value: toml::Value = toml::from_str(&text).expect("what was just serialised, parses");
    let mut paths = Vec::new();
    let toml::Value::Table(sections) = value else {
        panic!("a config document is always a table at the top level");
    };
    for (section, contents) in sections {
        if let toml::Value::Table(fields) = contents {
            for key in fields.keys() {
                paths.push(format!("{section}.{key}"));
            }
        }
    }
    paths.sort();
    paths
}

fn render() -> String {
    let mut out = String::new();
    out.push_str(
        "# Configuration reference\n\
         \n\
         <!-- Generated from `postio-config`'s schema by\n\
         `crates/postio-config/tests/config_doc.rs`. Do not edit by hand:\n\
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
        let (this_section, key) = entry.path.split_once('.').expect("path has a section");
        if this_section != section {
            if !section.is_empty() {
                out.push('\n');
            }
            section = this_section;
            let _ = writeln!(out, "## `[{section}]`\n");
            let _ = writeln!(out, "| Key | Type | Default | Description |");
            let _ = writeln!(out, "|---|---|---|---|");
        }
        let _ = writeln!(
            out,
            "| `{key}` | {} | `{}` | {} |",
            entry.kind, entry.default, entry.description
        );
    }
    out.push('\n');

    out.push_str(
        "## `[keys]`\n\
         \n\
         Overrides a command's binding, keyed by the command id. See the\n\
         [keyboard reference](keybindings.md) for every id and its default.\n\
         \n\
         ```toml\n\
         [keys]\n\
         archive = \"y\"\n\
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
         ## `[[rules]]`\n\
         \n\
         Filing rules, applied in the order they appear in the file -- an\n\
         array of tables rather than a map, so inserting a rule in the middle\n\
         is a matter of where you type it and nothing has to be renumbered.\n\
         \n\
         Each rule needs a `query` (the search bar\'s own language) or a\n\
         `filter` naming a `[filters]` entry to reuse, plus one or more\n\
         `actions`: `move:<mailbox>`, `label:<name>`, `flag`, `unflag`,\n\
         `mark-read`, `mark-unread`, `archive`, `trash`, `forward:<address>`.\n\
         A rule may not delete mail; `trash` moves it to the Trash folder.\n\
         \n\
         `stop = true` stops the rules below this one when it matches; the\n\
         default is `false`, so a rule that labels everything from a list does\n\
         not silently disable the rest. `enabled = false` is how a rule is\n\
         dry-run.\n\
         \n\
         ```toml\n\
         [[rules]]\n\
         name    = \"receipts\"\n\
         query   = \"from:billing has:attach\"\n\
         actions = [\"move:Receipts\", \"mark-read\"]\n\
         stop    = true\n\
         \n\
         [[rules]]\n\
         name    = \"needs-reply\"\n\
         filter  = \"needs-reply\"\n\
         actions = [\"flag\"]\n\
         ```\n\
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
         ```\n",
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
