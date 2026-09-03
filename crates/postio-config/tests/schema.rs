//! Schema and typed-deserialization tests for `postio-config`.
//!
//! Written before the implementation, per the TDD rule in `CLAUDE.md`.

use std::path::Path;

use postio_config::{
    Config, Density, Theme,
    keys::{self, KeyBindings},
};

// ---------------------------------------------------------------- defaults --

#[test]
fn missing_file_yields_defaults() {
    let cfg = Config::load_from_path(Path::new("/nonexistent/postio/config.toml"))
        .expect("a missing config file must not be an error");
    assert_eq!(cfg, Config::default());
}

#[test]
fn empty_file_yields_defaults() {
    let cfg = Config::from_toml_str("").expect("an empty file must parse");
    assert_eq!(cfg, Config::default());
}

#[test]
fn empty_sections_yield_defaults() {
    let cfg = Config::from_toml_str("[ui]\n[keys]\n[sync]\n[filters]\n")
        .expect("empty sections must parse");
    assert_eq!(cfg, Config::default());
}

#[test]
fn ui_defaults_match_the_design_canvas() {
    let ui = Config::default().ui;
    assert_eq!(ui.density, Density::Airy, "PLATE is the airy direction");
    assert_eq!(ui.theme, Theme::System);
    assert!(ui.show_hover_actions, "mouse parity is a requirement");
    assert!(ui.thread_drill);
    assert!(
        ui.show_key_hints,
        "the app teaches its own keyboard by default (#422)"
    );
}

#[test]
fn sync_defaults_are_sane() {
    let sync = Config::default().sync;
    assert_eq!(sync.check_for_mail, postio_config::CheckForMail::Idle);
    assert!(sync.sync_on_startup);
    assert!(sync.poll_interval_secs > 0);
    assert!(sync.max_connections >= 1);
}

// -------------------------------------------------------------------- [ui] --

#[test]
fn parses_every_ui_value() {
    let cfg = Config::from_toml_str(
        r#"
        [ui]
        density = "compact"
        theme = "dark"
        show_hover_actions = false
        thread_drill = false
        show_key_hints = false
        "#,
    )
    .unwrap();
    assert_eq!(cfg.ui.density, Density::Compact);
    assert_eq!(cfg.ui.theme, Theme::Dark);
    assert!(!cfg.ui.show_hover_actions);
    assert!(!cfg.ui.thread_drill);
    assert!(!cfg.ui.show_key_hints);
}

#[test]
fn a_partial_ui_section_still_defaults_key_hints_on() {
    let cfg = Config::from_toml_str("[ui]\nshow_key_hints = false\n").unwrap();
    assert!(!cfg.ui.show_key_hints);
    assert!(
        cfg.ui.show_hover_actions,
        "an unrelated field keeps its own default"
    );
}

#[test]
fn ui_density_accepts_all_three_heights() {
    for (text, want) in [
        ("airy", Density::Airy),
        ("comfortable", Density::Comfortable),
        ("compact", Density::Compact),
    ] {
        let cfg = Config::from_toml_str(&format!("[ui]\ndensity = \"{text}\"\n")).unwrap();
        assert_eq!(cfg.ui.density, want);
    }
}

#[test]
fn a_partial_ui_section_keeps_the_other_defaults() {
    let cfg = Config::from_toml_str("[ui]\ndensity = \"compact\"\n").unwrap();
    assert_eq!(cfg.ui.density, Density::Compact);
    assert_eq!(cfg.ui.theme, Theme::System);
    assert!(cfg.ui.show_hover_actions);
}

#[test]
fn an_unknown_enum_value_is_a_parse_error() {
    let err = Config::from_toml_str("[ui]\ndensity = \"enormous\"\n").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("density") || msg.contains("enormous"), "{msg}");
}

// ------------------------------------------------------------------ [keys] --

#[test]
fn default_bindings_match_the_design_canvas() {
    let k = KeyBindings::default();
    assert_eq!(k.binding("reply"), Some("e"));
    assert_eq!(k.binding("archive"), Some("a"));
    assert_eq!(k.binding("archive_thread"), Some("A"));
    assert_eq!(k.binding("undo"), Some("u"));
    assert_eq!(k.binding("thread"), Some("t"));
}

#[test]
fn a_key_override_wins_but_other_defaults_survive() {
    let cfg = Config::from_toml_str("[keys]\narchive = \"x\"\n").unwrap();
    assert_eq!(cfg.keys.binding("archive"), Some("x"));
    assert_eq!(cfg.keys.binding("undo"), Some("u"));
    assert_eq!(cfg.keys.overrides().len(), 1, "only the override is stored");
}

#[test]
fn bindings_for_unknown_commands_are_kept() {
    let cfg = Config::from_toml_str("[keys]\nsummarize_thread = \"g s\"\n").unwrap();
    assert_eq!(cfg.keys.binding("summarize_thread"), Some("g s"));
}

#[test]
fn resolved_bindings_merge_defaults_and_overrides() {
    let cfg = Config::from_toml_str("[keys]\nreply = \"r\"\n").unwrap();
    let resolved = cfg.keys.resolved();
    assert_eq!(resolved.get("reply").map(String::as_str), Some("r"));
    assert_eq!(resolved.get("thread").map(String::as_str), Some("t"));
    assert_eq!(resolved.len(), keys::DEFAULT_BINDINGS.len());
}

// -------------------------------------------------------------- [accounts] --

const ICLOUD: &str = r#"
[accounts.personal]
email = "ada@example.com"
display_name = "Person"
default = true

[accounts.personal.imap]
host = "imap.example.com"
port = 993
security = "implicit-tls"

[accounts.personal.smtp]
host = "smtp.example.com"
port = 465
security = "implicit-tls"
"#;

// ------------------------------------------------------- [sync] / [filters] --

#[test]
fn parses_the_sync_section() {
    let cfg = Config::from_toml_str(
        r#"
        [sync]
        check_for_mail = "poll"
        poll_interval_secs = 60
        max_connections = 3
        sync_on_startup = false
        body_fetch = "eager"
        "#,
    )
    .unwrap();
    assert_eq!(cfg.sync.check_for_mail, postio_config::CheckForMail::Poll);
    assert_eq!(cfg.sync.poll_interval_secs, 60);
    assert_eq!(cfg.sync.max_connections, 3);
    assert!(!cfg.sync.sync_on_startup);
    assert_eq!(cfg.sync.body_fetch, postio_config::BodyFetch::Eager);
}

#[test]
fn attachments_are_fetched_on_open_unless_the_file_says_otherwise() {
    // ADR 0017's payload axis. ~90% of a mailbox by weight is attachment
    // bytes nothing can index, so the default has to be the one that leaves
    // them where they are -- and it has to hold for a file that has never
    // mentioned `[sync]` at all.
    let cfg = Config::from_toml_str("").unwrap();
    assert_eq!(
        cfg.sync.attachment_fetch,
        postio_config::AttachmentFetch::OnOpen
    );

    for (text, expected) in [
        ("eager", postio_config::AttachmentFetch::Eager),
        ("never", postio_config::AttachmentFetch::Never),
        ("on_open", postio_config::AttachmentFetch::OnOpen),
    ] {
        let cfg = Config::from_toml_str(&format!("[sync]\nattachment_fetch = \"{text}\"\n"))
            .unwrap_or_else(|error| panic!("{text} should parse: {error}"));
        assert_eq!(cfg.sync.attachment_fetch, expected);
    }
}

#[test]
fn parses_named_filters() {
    let cfg = Config::from_toml_str(
        r#"
        [filters.needs-reply]
        query = "is:unread from:team"
        pinned = true
        "#,
    )
    .unwrap();
    let f = cfg.filters.get("needs-reply").unwrap();
    assert_eq!(f.query, "is:unread from:team");
    assert!(f.pinned);
}

// ------------------------------------------------------------- round-trips --

#[test]
fn defaults_round_trip() {
    let cfg = Config::default();
    let text = cfg.to_toml_string().unwrap();
    assert_eq!(Config::from_toml_str(&text).unwrap(), cfg);
}

#[test]
fn a_full_config_round_trips() {
    let cfg = Config::from_toml_str(ICLOUD).unwrap();
    let text = cfg.to_toml_string().unwrap();
    assert_eq!(Config::from_toml_str(&text).unwrap(), cfg);
}

#[test]
fn unknown_keys_survive_a_round_trip() {
    let text = r#"
future_top_level = "keep me"

[ui]
density = "compact"
ui_future = 7

[keys]
summarize = "g s"

[sync]
sync_future = ["a", "b"]

[accounts.personal]
email = "ada@example.com"
account_future = { nested = true }

[accounts.personal.imap]
host = "imap.example.com"
imap_future = "kept"

[filters.needs-reply]
query = "is:unread"
filter_future = 1
"#;
    let cfg = Config::from_toml_str(text).unwrap();
    let out = cfg.to_toml_string().unwrap();

    for needle in [
        "future_top_level",
        "keep me",
        "ui_future",
        "summarize",
        "sync_future",
        "account_future",
        "imap_future",
        "filter_future",
    ] {
        assert!(out.contains(needle), "lost {needle} in:\n{out}");
    }
    assert_eq!(
        Config::from_toml_str(&out).unwrap(),
        cfg,
        "a second round trip must be stable"
    );
}

// -------------------------------------------------------------- file paths --

#[test]
fn loads_from_a_real_file_and_saves_back() {
    let dir = std::env::temp_dir().join(format!("postio-config-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, ICLOUD).unwrap();

    let cfg = Config::load_from_path(&path).unwrap();
    // `[accounts]` is retired (#470), so this file's account tables now
    // round-trip as unknown keys rather than parsing into a schema. That is
    // the property worth asserting here: the file survives a load and save
    // without losing what it holds.
    let out = cfg.to_toml_string().unwrap();
    assert!(
        out.contains("[accounts.personal]"),
        "a retired section must still survive the round trip: {out}"
    );

    let reread = Config::from_toml_str(&cfg.to_toml_string().unwrap()).unwrap();
    assert_eq!(reread, cfg);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_default_path_is_the_xdg_one() {
    use postio_config::paths::Platform;

    let dir = postio_config::paths::config_dir_from(
        |k| match k {
            "XDG_CONFIG_HOME" => Some("/home/x/.conf".into()),
            _ => None,
        },
        Platform::Freedesktop,
    )
    .unwrap();
    assert_eq!(dir, Path::new("/home/x/.conf/postio"));

    let dir = postio_config::paths::config_dir_from(
        |k| match k {
            "HOME" => Some("/home/x".into()),
            _ => None,
        },
        Platform::Freedesktop,
    )
    .unwrap();
    assert_eq!(dir, Path::new("/home/x/.config/postio"));

    assert!(postio_config::paths::config_dir_from(|_| None, Platform::Freedesktop).is_err());
}

// ------------------------------------------------------------- mailboxes --

#[test]
fn parses_a_mailbox_role_mapping() {
    // Keyed by role and valued by the server's own path, which is the way
    // round a person can write: they know what they want archived and they
    // can read the folder name off their own server. `[keys]` is the same
    // shape -- the thing you mean on the left, the spelling on the right.
    let cfg = Config::from_toml_str(
        r#"
        [mailboxes]
        archive = "Vecchia Posta"
        trash = "Cestino"
        "#,
    )
    .unwrap();

    assert_eq!(
        cfg.mailboxes.get("archive").map(String::as_str),
        Some("Vecchia Posta")
    );
    assert_eq!(
        cfg.mailboxes.get("trash").map(String::as_str),
        Some("Cestino")
    );
}

#[test]
fn a_mailbox_mapping_becomes_role_overrides() {
    // The point of the section: it has to arrive at resolution as the model's
    // own type, or every consumer reinvents the role parsing.
    let cfg = Config::from_toml_str(
        r#"
        [mailboxes]
        archive = "Vecchia Posta"
        "#,
    )
    .unwrap();

    let overrides = cfg.role_overrides();
    assert_eq!(
        overrides.role_for("Vecchia Posta"),
        Some(postio_model::MailboxRole::Archive)
    );
    assert_eq!(overrides.role_for("Cestino"), None);
}

#[test]
fn no_mailboxes_section_means_no_overrides() {
    let cfg = Config::from_toml_str("").unwrap();
    assert!(
        cfg.role_overrides().is_empty(),
        "an account that never writes [mailboxes] must resolve exactly as before"
    );
}

#[test]
fn an_unparseable_role_is_dropped_rather_than_guessed() {
    // `role_overrides` cannot report an error -- validation is where problems
    // are reported, with a line number. What it must not do is guess: a typo
    // that silently became Archive would move mail somewhere nobody chose.
    let cfg = Config::from_toml_str(
        r#"
        [mailboxes]
        archiv = "Vecchia Posta"
        "#,
    )
    .unwrap();
    assert!(cfg.role_overrides().is_empty());
}
