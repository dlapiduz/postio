//! Schema and typed-deserialization tests for `postio-config`.
//!
//! Written before the implementation, per the TDD rule in `CLAUDE.md`.

use std::path::Path;

use postio_config::{
    Config, Density, MailSecurity, Theme,
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
    let cfg = Config::from_toml_str("[ui]\n[keys]\n[accounts]\n[sync]\n[filters]\n")
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
}

#[test]
fn sync_defaults_are_sane() {
    let sync = Config::default().sync;
    assert!(sync.idle);
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
        "#,
    )
    .unwrap();
    assert_eq!(cfg.ui.density, Density::Compact);
    assert_eq!(cfg.ui.theme, Theme::Dark);
    assert!(!cfg.ui.show_hover_actions);
    assert!(!cfg.ui.thread_drill);
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

#[test]
fn parses_the_personal_account() {
    let cfg = Config::from_toml_str(ICLOUD).unwrap();
    let acct = cfg.account("personal").expect("account by table key");
    assert_eq!(acct.id, "personal", "the table key becomes the account id");
    assert_eq!(acct.email, "ada@example.com");
    assert_eq!(acct.display_name.as_deref(), Some("Person"));
    assert!(acct.is_default);
    assert_eq!(acct.imap.host, "imap.example.com");
    assert_eq!(acct.imap.port, 993);
    assert_eq!(acct.imap.security, MailSecurity::ImplicitTls);
    assert_eq!(acct.smtp.host, "smtp.example.com");
    assert_eq!(acct.smtp.port, 465);
    assert_eq!(acct.smtp.security, MailSecurity::ImplicitTls);
}

#[test]
fn account_ports_default_to_the_implicit_tls_ports() {
    let cfg = Config::from_toml_str(
        r#"
        [accounts.work]
        email = "a@b.c"
        [accounts.work.imap]
        host = "imap.b.c"
        [accounts.work.smtp]
        host = "smtp.b.c"
        "#,
    )
    .unwrap();
    let acct = cfg.account("work").unwrap();
    assert_eq!(acct.imap.port, 993);
    assert_eq!(acct.smtp.port, 465);
    assert_eq!(acct.imap.security, MailSecurity::ImplicitTls);
    assert_eq!(acct.smtp.security, MailSecurity::ImplicitTls);
}

#[test]
fn security_spellings_are_forgiving() {
    for text in ["implicit-tls", "implicit_tls", "tls", "ssl"] {
        let cfg =
            Config::from_toml_str(&format!("[accounts.a.imap]\nsecurity = \"{text}\"\n")).unwrap();
        assert_eq!(
            cfg.account("a").unwrap().imap.security,
            MailSecurity::ImplicitTls,
            "{text}"
        );
    }
    for text in ["starttls", "start-tls", "start_tls"] {
        let cfg =
            Config::from_toml_str(&format!("[accounts.a.imap]\nsecurity = \"{text}\"\n")).unwrap();
        assert_eq!(
            cfg.account("a").unwrap().imap.security,
            MailSecurity::StartTls,
            "{text}"
        );
    }
    let cfg = Config::from_toml_str("[accounts.a.imap]\nsecurity = \"none\"\n").unwrap();
    assert_eq!(cfg.account("a").unwrap().imap.security, MailSecurity::None);
}

#[test]
fn the_keyring_entry_is_derived_when_absent() {
    let cfg = Config::from_toml_str(ICLOUD).unwrap();
    let acct = cfg.account("personal").unwrap();
    assert_eq!(acct.imap_keyring_entry(), "postio:personal:imap");
    assert_eq!(acct.smtp_keyring_entry(), "postio:personal:smtp");
}

#[test]
fn an_explicit_keyring_entry_is_honored() {
    let cfg = Config::from_toml_str(
        r#"
        [accounts.personal.imap]
        keyring_entry = "my-own-entry"
        "#,
    )
    .unwrap();
    assert_eq!(
        cfg.account("personal").unwrap().imap_keyring_entry(),
        "my-own-entry"
    );
}

#[test]
fn the_default_account_is_the_flagged_one_then_the_first() {
    let cfg = Config::from_toml_str(ICLOUD).unwrap();
    assert_eq!(cfg.default_account().unwrap().id, "personal");

    let cfg = Config::from_toml_str(
        r#"
        [accounts.aaa]
        email = "a@x.c"
        [accounts.zzz]
        email = "z@x.c"
        default = true
        "#,
    )
    .unwrap();
    assert_eq!(cfg.default_account().unwrap().id, "zzz");
}

#[test]
fn a_missing_email_is_left_for_the_validation_pass() {
    // Typed deserialization is deliberately lenient; human-readable validation
    // is a separate bead (postio-9xj).
    let cfg = Config::from_toml_str("[accounts.broken]\n").expect("must still parse");
    assert_eq!(cfg.account("broken").unwrap().email, "");
}

// ------------------------------------------------------- [sync] / [filters] --

#[test]
fn parses_the_sync_section() {
    let cfg = Config::from_toml_str(
        r#"
        [sync]
        idle = false
        poll_interval_secs = 60
        max_connections = 3
        sync_on_startup = false
        body_fetch = "eager"
        "#,
    )
    .unwrap();
    assert!(!cfg.sync.idle);
    assert_eq!(cfg.sync.poll_interval_secs, 60);
    assert_eq!(cfg.sync.max_connections, 3);
    assert!(!cfg.sync.sync_on_startup);
    assert_eq!(cfg.sync.body_fetch, postio_config::BodyFetch::Eager);
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
    assert_eq!(cfg.account("personal").unwrap().imap.port, 993);

    let reread = Config::from_toml_str(&cfg.to_toml_string().unwrap()).unwrap();
    assert_eq!(reread, cfg);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_default_path_is_the_xdg_one() {
    let dir = postio_config::paths::config_dir_from(|k| match k {
        "XDG_CONFIG_HOME" => Some("/home/x/.conf".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(dir, Path::new("/home/x/.conf/postio"));

    let dir = postio_config::paths::config_dir_from(|k| match k {
        "HOME" => Some("/home/x".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(dir, Path::new("/home/x/.config/postio"));

    assert!(postio_config::paths::config_dir_from(|_| None).is_err());
}
