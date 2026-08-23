//! Security tests: `config.toml` must never be able to hold a secret.
//!
//! CLAUDE.md: "Secrets go in the Secret Service keyring. Never in
//! `config.toml`, never logged." These tests are the enforcement.

use postio_config::{Config, secrets};

const PASSWORD: &str = "hunter2-do-not-persist";

fn config_with_secrets_everywhere() -> String {
    format!(
        r#"
password = "{PASSWORD}"

[ui]
density = "compact"
secret = "{PASSWORD}"

[sync]
api_key = "{PASSWORD}"

[accounts.personal]
email = "ada@example.com"
password = "{PASSWORD}"

[accounts.personal.imap]
host = "imap.example.com"
password = "{PASSWORD}"
app_password = "{PASSWORD}"
access_token = "{PASSWORD}"

[accounts.personal.smtp]
host = "smtp.example.com"
passwd = "{PASSWORD}"
client_secret = "{PASSWORD}"
"#
    )
}

#[test]
fn secrets_are_dropped_at_parse_time() {
    let cfg = Config::from_toml_str(&config_with_secrets_everywhere()).unwrap();
    let debug = format!("{cfg:#?}");
    assert!(
        !debug.contains(PASSWORD),
        "a secret survived into the parsed config:\n{debug}"
    );
}

#[test]
fn stripped_secrets_are_reported_by_path() {
    let cfg = Config::from_toml_str(&config_with_secrets_everywhere()).unwrap();
    let found = cfg.rejected_secrets();
    assert!(found.contains(&"password".to_string()), "{found:?}");
    assert!(
        found.contains(&"accounts.personal.imap.app_password".to_string()),
        "{found:?}"
    );
    assert!(
        found.contains(&"accounts.personal.smtp.client_secret".to_string()),
        "{found:?}"
    );
    assert_eq!(
        found.len(),
        9,
        "every secret key must be reported: {found:?}"
    );
}

#[test]
fn no_secret_can_round_trip_to_disk() {
    let cfg = Config::from_toml_str(&config_with_secrets_everywhere()).unwrap();
    let out = cfg.to_toml_string().unwrap();
    assert!(!out.contains(PASSWORD), "secret value serialized:\n{out}");
    for key in [
        "password",
        "passwd",
        "app_password",
        "access_token",
        "client_secret",
        "api_key",
        "secret",
    ] {
        assert!(!out.contains(key), "secret key `{key}` serialized:\n{out}");
    }
    // The rest of the file is untouched.
    assert!(out.contains("imap.example.com"));
    assert!(out.contains("ada@example.com"));
}

#[test]
fn a_secret_injected_after_parsing_still_cannot_be_written() {
    // Belt and braces: even if some later code pushes a secret into the
    // unknown-key passthrough, serialization drops it.
    let mut cfg = Config::default();
    cfg.extra
        .insert("password".into(), toml::Value::String(PASSWORD.into()));
    cfg.ui
        .extra
        .insert("oauth_token".into(), toml::Value::String(PASSWORD.into()));
    let out = cfg.to_toml_string().unwrap();
    assert!(!out.contains(PASSWORD), "{out}");
    assert!(!out.contains("oauth_token"), "{out}");
}

#[test]
fn keyring_entry_is_not_mistaken_for_a_secret() {
    assert!(!secrets::is_secret_key("keyring_entry"));
    assert!(!secrets::is_secret_key("host"));
    assert!(!secrets::is_secret_key("email"));

    let cfg = Config::from_toml_str(
        r#"
        [accounts.personal.imap]
        keyring_entry = "postio:personal:imap"
        "#,
    )
    .unwrap();
    assert_eq!(
        cfg.account("personal")
            .unwrap()
            .imap
            .keyring_entry
            .as_deref(),
        Some("postio:personal:imap")
    );
    assert!(
        cfg.to_toml_string().unwrap().contains("keyring_entry"),
        "a keyring reference is not a secret and must persist"
    );
}

#[test]
fn secret_key_detection_is_case_insensitive() {
    for key in [
        "password",
        "PASSWORD",
        "Passwd",
        "app_password",
        "secret",
        "client_secret",
        "token",
        "access_token",
        "refresh_token",
        "api_key",
        "apikey",
        "credentials",
    ] {
        assert!(secrets::is_secret_key(key), "{key} must count as a secret");
    }
}

#[test]
fn parse_errors_never_quote_a_secret() {
    // A syntax error on a password line must not echo the value back through
    // the error message, which ends up in logs and in the settings validity line.
    let err =
        Config::from_toml_str(&format!("[accounts.a]\npassword = \"{PASSWORD}\n")).unwrap_err();
    let msg = format!("{err}\n{err:?}");
    assert!(
        !msg.contains(PASSWORD),
        "secret leaked through an error:\n{msg}"
    );
}

#[test]
fn no_type_in_this_crate_declares_a_secret_field() {
    // Guards against a future struct gaining a `password` field, which would
    // make secrets serializable again.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    visit(&src, &mut offenders);
    assert!(
        offenders.is_empty(),
        "secret-bearing fields found: {offenders:?}"
    );

    fn visit(dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, out);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let line = line.trim();
                let Some(rest) = line.strip_prefix("pub ") else {
                    continue;
                };
                let Some((name, _)) = rest.split_once(':') else {
                    continue;
                };
                let name = name.trim();
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && postio_config::secrets::is_secret_key(name)
                {
                    out.push(format!("{}:{}: {line}", path.display(), n + 1));
                }
            }
        }
    }
}
