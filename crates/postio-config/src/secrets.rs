//! Passwords and tokens live in the Secret Service keyring; `config.toml` only holds a
//! `keyring_entry`
//! reference. Secret-looking keys are stripped before deserialization and again on the way out, and
//! parser errors are redacted (`redact_secret_lines`).

use std::borrow::Cow;

use toml::{Table, Value};

const SECRET_MARKERS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "credential",
    "privatekey",
];

const SECRET_EXACT: &[&str] = &["pass", "pw"];

/// Whether a TOML key must never be stored in `config.toml`.
///
/// Deliberately generous: a false positive costs a user one renamed key, a
/// false negative writes a password to a world-readable file.
///
/// ```
/// # use postio_config::secrets::is_secret_key;
/// assert!(is_secret_key("app_password"));
/// assert!(is_secret_key("ACCESS_TOKEN"));
/// assert!(!is_secret_key("keyring_entry"));
/// ```
pub fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| *c != '_' && *c != '-' && *c != ' ')
        .flat_map(char::to_lowercase)
        .collect();
    SECRET_EXACT.contains(&normalized.as_str())
        || SECRET_MARKERS.iter().any(|m| normalized.contains(m))
}

pub fn strip_secrets(table: &mut Table) -> Vec<String> {
    let mut removed = Vec::new();
    strip_table(table, "", &mut removed);
    removed
}

fn strip_table(table: &mut Table, prefix: &str, removed: &mut Vec<String>) {
    let secret_keys: Vec<String> = table.keys().filter(|k| is_secret_key(k)).cloned().collect();
    for key in secret_keys {
        table.remove(&key);
        removed.push(join(prefix, &key));
    }
    for (key, value) in table.iter_mut() {
        strip_value(value, &join(prefix, key), removed);
    }
}

fn strip_value(value: &mut Value, path: &str, removed: &mut Vec<String>) {
    match value {
        Value::Table(t) => strip_table(t, path, removed),
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                strip_value(item, &format!("{path}[{index}]"), removed);
            }
        }
        _ => {}
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// TOML parser errors quote the offending source line verbatim; without this a syntax error on a
/// password line would print the password.
pub fn redact_secret_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&redact_line(line));
    }
    out
}

fn redact_line(line: &str) -> Cow<'_, str> {
    let Some(eq) = line.find('=') else {
        return Cow::Borrowed(line);
    };
    let head = &line[..eq];
    let trailing: String = head
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '"' | '\''))
        .collect();
    let key: String = trailing.chars().rev().collect();
    let key = key.trim_matches(['"', '\'']);
    if is_secret_key(key) {
        Cow::Owned(format!("{head}= <redacted>"))
    } else {
        Cow::Borrowed(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_matched_regardless_of_shape() {
        for key in [
            "password",
            "PASSWORD",
            "app-password",
            "app_password",
            "Passwd",
            "pass",
            "PW",
            "secret",
            "client_secret",
            "token",
            "refresh_token",
            "api_key",
            "apiKey",
            "credentials",
            "private_key",
            "passphrase",
        ] {
            assert!(is_secret_key(key), "{key}");
        }
    }

    #[test]
    fn ordinary_schema_keys_are_not_secrets() {
        for key in [
            "keyring_entry",
            "host",
            "port",
            "security",
            "auth",
            "email",
            "display_name",
            "density",
            "theme",
            "idle",
            "query",
        ] {
            assert!(!is_secret_key(key), "{key}");
        }
    }

    #[test]
    fn stripping_reports_dotted_paths() {
        let mut table: Table = toml::from_str(
            r#"
            password = "x"
            [accounts.personal.imap]
            host = "h"
            app_password = "x"
            "#,
        )
        .unwrap();
        let mut removed = strip_secrets(&mut table);
        removed.sort();
        assert_eq!(
            removed,
            vec![
                "accounts.personal.imap.app_password".to_string(),
                "password".to_string()
            ]
        );
        assert!(toml::to_string(&table).unwrap().contains("host"));
    }

    #[test]
    fn stripping_reaches_into_arrays() {
        let mut table: Table =
            toml::from_str("[[accounts]]\nhost = \"h\"\ntoken = \"x\"\n").unwrap();
        let removed = strip_secrets(&mut table);
        assert_eq!(removed, vec!["accounts[0].token".to_string()]);
    }

    #[test]
    fn redaction_keeps_the_key_and_drops_the_value() {
        let text = "2 | password = \"hunter2\"\nhost = \"imap.example.com\"";
        let out = redact_secret_lines(text);
        assert!(!out.contains("hunter2"), "{out}");
        assert!(out.contains("password = <redacted>"), "{out}");
        assert!(out.contains("imap.example.com"), "{out}");
    }
}
