//! Shared with `build.rs` via `#[path]`, so it names no type from elsewhere in this crate;
//! `Security` duplicates `Encryption` for that reason.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Security {
    Tls,
    StartTls,
    None,
}

/// `#[allow(dead_code)]`: `build.rs`'s compilation of this file reads only `auth` and `oauth`;
/// `builtin.rs` reads the rest.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRow {
    pub display_name: String,
    #[serde(default)]
    pub domains: Vec<String>,
    /// A custom domain delegated to a provider is found by MX records, where the `imap.<domain>`
    /// guess fails. Matched as a label-boundary suffix: providers hand out one inbound host per
    /// customer.
    #[serde(default)]
    pub mx_suffixes: Vec<String>,
    /// Matched whole or as a label-bounded suffix; the reader trusts a sender-authentication header
    /// only from one of these (#1928). Empty when the provider stamps none.
    #[serde(default)]
    pub authserv_ids: Vec<String>,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    #[serde(default)]
    pub requires_app_password: bool,
    #[serde(default)]
    pub password_help_url: Option<String>,
    /// `false` for providers that under-report CONDSTORE changes (iCloud, Exchange): every flag in
    /// a folder is then read now and then (`postio_model::SweepPolicy`).
    #[serde(default = "default_condstore_reliable")]
    pub condstore_reliable: bool,
    /// Preference order; `"app-password"` and `"oauth2"` are the tokens understood today.
    pub auth: Vec<String>,
    #[serde(default)]
    pub oauth: Option<OAuthRow>,
    /// Preference order among protocols (ADR 0018 Q5); a new backend is a new token, never a named
    /// constant.
    #[serde(default = "default_backend")]
    pub backend: Vec<String>,
    #[serde(default)]
    pub jmap: Option<JmapRow>,
    /// Role to this provider's own folder name, for servers with no `SPECIAL-USE`: wins the tie
    /// against a look-alike another client created (#959, #501, #943).
    #[serde(default)]
    pub mailboxes: BTreeMap<String, String>,
}

fn default_condstore_reliable() -> bool {
    true
}

fn default_backend() -> Vec<String> {
    vec![BACKEND_IMAP.to_owned()]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JmapRow {
    #[allow(dead_code)]
    pub session_url: String,
}

/// No `deny_unknown_fields`: a pasted `client_secret` must land in `extra` to be found and stripped
/// without matching `authorize`/`token`, which contain a secret marker themselves.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OAuthRow {
    /// `"builtin"`, `"broker"` or `"own-client"`; carried through unread here.
    #[allow(dead_code)]
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub authorize: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub scopes: Vec<String>,
    /// The refresh grant's own lifetime, which no token response mentions (#954). Absent means
    /// unknown: nothing is marked stale early.
    #[allow(dead_code)]
    #[serde(default)]
    pub refresh_token_lifetime_days: Option<u32>,
    /// Scanned for secret-looking keys and stripped in `parse`, never otherwise read.
    #[serde(flatten)]
    pub extra: toml::Table,
}

const OAUTH2: &str = "oauth2";

const BACKEND_IMAP: &str = "imap";
const BACKEND_JMAP: &str = "jmap";
const BACKEND_GMAIL: &str = "gmail";

#[derive(Debug)]
pub enum ProvidersError {
    Toml(toml::de::Error),
    Invalid(String),
}

impl fmt::Display for ProvidersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(error) => write!(f, "{error}"),
            Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for ProvidersError {}

/// Stripped secrets are dotted paths, never values.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    // build.rs's own compilation checks only `stripped_secrets`; `builtin.rs`
    // reads `rows`.
    #[allow(dead_code)]
    pub rows: BTreeMap<String, ProviderRow>,
    pub stripped_secrets: Vec<String>,
}

/// Strips secret-looking keys from each row's `[oauth.extra]` (ADR 0006 Q4) and reports their
/// dotted paths, never values. Only `extra` is scanned: `authorize`/`token` contain a secret marker
/// themselves and would lose a legitimate endpoint URL.
/// The caller decides what a secret means in the shipped table versus a user overlay.
pub fn parse(text: &str) -> Result<Parsed, ProvidersError> {
    let table: toml::Table = text.parse().map_err(ProvidersError::Toml)?;

    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        provider: BTreeMap<String, ProviderRow>,
    }
    let mut file: File = toml::Value::Table(table)
        .try_into()
        .map_err(ProvidersError::Toml)?;

    let mut stripped_secrets = Vec::new();
    for (id, row) in file.provider.iter_mut() {
        if let Some(oauth) = row.oauth.as_mut() {
            for path in postio_config::secrets::strip_secrets(&mut oauth.extra) {
                stripped_secrets.push(format!("provider.{id}.oauth.{path}"));
            }
        }
        validate(id, row)?;
    }

    Ok(Parsed {
        rows: file.provider,
        stripped_secrets,
    })
}

/// A row naming `oauth2` in `auth` must say where its endpoints come from, checked at load time
/// (#152).
fn validate(id: &str, row: &ProviderRow) -> Result<(), ProvidersError> {
    for backend in &row.backend {
        if backend != BACKEND_IMAP && backend != BACKEND_JMAP && backend != BACKEND_GMAIL {
            return Err(ProvidersError::Invalid(format!(
                "provider `{id}` names a backend Postio does not ship: `{backend}`"
            )));
        }
    }
    if row.backend.is_empty() {
        return Err(ProvidersError::Invalid(format!(
            "provider `{id}` has an empty `backend` list; omit the key for the default"
        )));
    }
    if row.backend.iter().any(|backend| backend == BACKEND_JMAP) && row.jmap.is_none() {
        return Err(ProvidersError::Invalid(format!(
            "provider `{id}` advertises `jmap` but has no [jmap] table naming its \
             session_url"
        )));
    }

    if !row.auth.iter().any(|method| method == OAUTH2) {
        return Ok(());
    }
    let names_endpoints = row.oauth.as_ref().is_some_and(|oauth| {
        oauth.issuer.is_some() || (oauth.authorize.is_some() && oauth.token.is_some())
    });
    if names_endpoints {
        return Ok(());
    }
    Err(ProvidersError::Invalid(format!(
        "provider `{id}` lists `oauth2` in `auth` but its [oauth] table names \
         neither `issuer` nor both `authorize` and `token`"
    )))
}

/// `build.rs` never merges an overlay, so this is unread there; `builtin.rs` calls it.
#[allow(dead_code)]
pub fn merged(
    shipped: BTreeMap<String, ProviderRow>,
    overlay: BTreeMap<String, ProviderRow>,
) -> BTreeMap<String, ProviderRow> {
    let mut merged = shipped;
    merged.extend(overlay);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(auth: &[&str]) -> ProviderRow {
        ProviderRow {
            display_name: "Test".to_string(),
            domains: vec!["example.com".to_string()],
            mx_suffixes: Vec::new(),
            authserv_ids: Vec::new(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            requires_app_password: false,
            password_help_url: None,
            condstore_reliable: true,
            auth: auth.iter().map(|s| s.to_string()).collect(),
            oauth: None,
            backend: default_backend(),
            jmap: None,
            mailboxes: BTreeMap::new(),
        }
    }

    #[test]
    fn a_minimal_row_parses() {
        let parsed = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]
            "#,
        )
        .expect("a well-formed row should parse");
        let row = &parsed.rows["test"];
        assert_eq!(row.display_name, "Test");
        assert_eq!(row.domains, ["example.com"]);
        assert_eq!(row.imap_port, 993);
        assert!(parsed.stripped_secrets.is_empty());
        assert!(
            row.mailboxes.is_empty(),
            "a row that names no [mailboxes] table should not invent one"
        );
    }

    #[test]
    fn a_mailboxes_table_names_the_providers_own_folders() {
        let parsed = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]

            [provider.test.mailboxes]
            sent = "Sent Messages"
            trash = "Deleted Messages"
            "#,
        )
        .expect("a [mailboxes] table should parse");
        let row = &parsed.rows["test"];
        assert_eq!(
            row.mailboxes.get("sent").map(String::as_str),
            Some("Sent Messages")
        );
        assert_eq!(
            row.mailboxes.get("trash").map(String::as_str),
            Some("Deleted Messages")
        );
    }

    #[test]
    fn oauth2_without_an_issuer_or_endpoints_is_a_validation_error() {
        let error = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["oauth2"]
            "#,
        )
        .unwrap_err();
        assert!(matches!(error, ProvidersError::Invalid(_)), "{error}");
    }

    #[test]
    fn oauth2_with_an_issuer_is_valid() {
        let parsed = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["oauth2"]

            [provider.test.oauth]
            issuer = "https://example.com"
            "#,
        )
        .expect("an issuer alone should satisfy the rule");
        assert_eq!(
            parsed.rows["test"]
                .oauth
                .as_ref()
                .unwrap()
                .issuer
                .as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn a_row_may_state_how_long_its_refresh_token_lives() {
        let parsed = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["oauth2"]

            [provider.test.oauth]
            issuer = "https://example.com"
            refresh_token_lifetime_days = 7
            "#,
        )
        .expect("a stated refresh lifetime is valid");
        assert_eq!(
            parsed.rows["test"]
                .oauth
                .as_ref()
                .unwrap()
                .refresh_token_lifetime_days,
            Some(7)
        );
    }

    #[test]
    fn a_row_that_states_no_refresh_lifetime_has_none() {
        let parsed = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["oauth2"]

            [provider.test.oauth]
            issuer = "https://example.com"
            "#,
        )
        .expect("valid without one");
        assert_eq!(
            parsed.rows["test"]
                .oauth
                .as_ref()
                .unwrap()
                .refresh_token_lifetime_days,
            None
        );
    }

    #[test]
    fn oauth2_with_explicit_endpoints_and_no_issuer_is_valid() {
        parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["oauth2"]

            [provider.test.oauth]
            authorize = "https://example.com/authorize"
            token = "https://example.com/token"
            "#,
        )
        .expect("authorize + token alone should satisfy the rule");
    }

    #[test]
    fn oauth2_with_only_authorize_and_no_token_is_still_invalid() {
        let error = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["oauth2"]

            [provider.test.oauth]
            authorize = "https://example.com/authorize"
            "#,
        )
        .unwrap_err();
        assert!(matches!(error, ProvidersError::Invalid(_)), "{error}");
    }

    #[test]
    fn a_client_secret_is_stripped_and_reported_not_honoured() {
        let parsed = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["oauth2"]

            [provider.test.oauth]
            issuer = "https://example.com"
            client_secret = "shh"
            "#,
        )
        .expect("a stripped secret must not fail the parse");
        assert!(
            !parsed.stripped_secrets.is_empty(),
            "the client_secret should have been recorded as stripped"
        );
    }

    #[test]
    fn merging_lets_a_user_row_win_on_a_collision() {
        let mut shipped = BTreeMap::new();
        shipped.insert("test".to_string(), row(&["app-password"]));

        let mut overlay = BTreeMap::new();
        let mut replacement = row(&["oauth2"]);
        replacement.oauth = Some(OAuthRow {
            issuer: Some("https://example.com".to_string()),
            ..Default::default()
        });
        overlay.insert("test".to_string(), replacement);

        let merged = merged(shipped, overlay);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged["test"].auth, ["oauth2"]);
    }

    #[test]
    fn merging_adds_a_new_user_row_alongside_the_shipped_ones() {
        let mut shipped = BTreeMap::new();
        shipped.insert("shipped".to_string(), row(&["app-password"]));

        let mut overlay = BTreeMap::new();
        overlay.insert("mine".to_string(), row(&["app-password"]));

        let merged = merged(shipped, overlay);
        assert_eq!(merged.len(), 2);
        assert!(merged.contains_key("shipped"));
        assert!(merged.contains_key("mine"));
    }

    #[test]
    fn an_unknown_field_is_a_parse_error_not_a_silent_drop() {
        // This file has no write-back path, so a typo in a key name should be loud.
        let error = parse(
            r#"
            [provider.test]
            display_name = "Test"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]
            wrong_key = "oops"
            "#,
        )
        .unwrap_err();
        assert!(matches!(error, ProvidersError::Toml(_)), "{error}");
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;

    #[test]
    fn a_row_without_a_backend_list_defaults_to_imap() {
        let parsed = parse(
            r#"
            [provider.plain]
            display_name = "Plain"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]
            "#,
        )
        .expect("a backend-less row is every row that existed before #545");
        assert_eq!(parsed.rows["plain"].backend, vec!["imap".to_owned()]);
    }

    #[test]
    fn a_row_advertising_jmap_must_name_its_session_url() {
        let error = parse(
            r#"
            [provider.half]
            display_name = "Half"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]
            backend = ["jmap", "imap"]
            "#,
        )
        .expect_err("jmap with nowhere to dial is a row nobody can use");
        assert!(error.to_string().contains("session_url"), "{error}");
    }

    #[test]
    fn a_backend_postio_does_not_ship_is_refused_at_load_time() {
        let error = parse(
            r#"
            [provider.exotic]
            display_name = "Exotic"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]
            backend = ["nntp"]
            "#,
        )
        .expect_err("an unknown backend name is a typo to catch now, not at add time");
        assert!(error.to_string().contains("nntp"), "{error}");
    }

    #[test]
    fn a_jmap_row_carries_its_session_url_through() {
        let parsed = parse(
            r#"
            [provider.native]
            display_name = "Native"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]
            backend = ["jmap", "imap"]

            [provider.native.jmap]
            session_url = "https://api.example.com/jmap/session/"
            "#,
        )
        .expect("a complete jmap row parses");
        let row = &parsed.rows["native"];
        assert_eq!(row.backend, vec!["jmap".to_owned(), "imap".to_owned()]);
        assert_eq!(
            row.jmap.as_ref().expect("the jmap table").session_url,
            "https://api.example.com/jmap/session/"
        );
    }
}

#[cfg(test)]
mod gmail_backend_tests {
    use super::*;

    #[test]
    fn the_gmail_token_is_a_backend_postio_ships() {
        let parsed = parse(
            r#"
            [provider.rest]
            display_name = "Rest"
            domains = ["example.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]
            backend = ["imap", "gmail"]
            "#,
        )
        .expect("gmail is a shipped backend token (#546)");
        assert_eq!(
            parsed.rows["rest"].backend,
            vec!["imap".to_owned(), "gmail".to_owned()]
        );
    }
}
