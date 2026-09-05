//! Parses `providers.toml` -- the shipped defaults and the user's overlay
//! alike -- into rows [`builtin`](super::builtin) turns into [`Preset`]s.
//!
//! `build.rs` compiles this file a second time via `#[path]`, so the parser
//! that validates the shipped table at `cargo build` time is byte for byte
//! the parser the running application uses for both the shipped table and
//! the user's own overlay -- there is no second copy to drift. This is the
//! same idiom `postio-gtk/src/tokens.rs` established first for design
//! tokens (`docs/ARCHITECTURE.md` §10), and for the same reason this module
//! names no type from elsewhere in `postio-account`: `build.rs`'s own
//! compilation has no `crate::discovery` to resolve one against, only the
//! external crates listed in `[build-dependencies]`. `Security` duplicates
//! [`crate::discovery::settings::Encryption`](super::settings::Encryption)'s
//! shape for exactly that reason -- `builtin.rs` converts between them,
//! where `crate::` paths are available again.
//!
//! [`Preset`]: super::builtin::Preset

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

/// Transport security for one server, kebab-case exactly like
/// [`crate::discovery::settings::Encryption`](super::settings::Encryption).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Security {
    Tls,
    StartTls,
    None,
}

/// One `[provider.<id>]` table.
///
/// `#[allow(dead_code)]`: `build.rs`'s own compilation only ever calls
/// [`parse`] to validate the shipped file and inspect
/// [`stripped_secrets`](Parsed::stripped_secrets) -- every field below
/// except `auth`/`oauth` (which [`validate`] reads) is otherwise read only
/// by `builtin.rs`'s `Preset` wrapper, which is not part of that
/// compilation at all. The main crate's own build does read every field,
/// through that wrapper.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRow {
    pub display_name: String,
    #[serde(default)]
    pub domains: Vec<String>,
    /// MX host suffixes that identify this provider (#94).
    ///
    /// A custom domain delegated to a provider usually says so in its MX
    /// records, and that is exactly the case the convention guess gets
    /// wrong: mail for a custom domain hosted elsewhere is not at
    /// `imap.<that-domain>`.
    ///
    /// Data, like `domains`, for the same reason -- an MX suffix is a fact
    /// about a provider, and a provider is a row rather than a branch
    /// (`PRODUCT.md` §3). Matched as a suffix on a label boundary, never as
    /// a whole host: providers hand out one inbound host per customer.
    #[serde(default)]
    pub mx_suffixes: Vec<String>,
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
    /// Preference order among the ways this provider can be authenticated.
    /// `"app-password"` and `"oauth2"` are the two tokens understood today.
    pub auth: Vec<String>,
    #[serde(default)]
    pub oauth: Option<OAuthRow>,
    /// Preference order among the protocols this provider is reached over
    /// (ADR 0018 Q5). `"imap"` and `"jmap"` are the two backends Postio
    /// ships; providers are data, so a new backend is a new token here,
    /// never a named constant in code.
    #[serde(default = "default_backend")]
    pub backend: Vec<String>,
    #[serde(default)]
    pub jmap: Option<JmapRow>,
    /// `[provider.<id>.mailboxes]` -- role to this provider's own folder
    /// name, read the same shape a user's `[mailboxes]` writes (#959).
    ///
    /// For a provider whose server advertises no RFC 6154 `SPECIAL-USE`, so
    /// a role can only ever be guessed from a name: when the guess is
    /// contested by a look-alike another client created, the provider's own
    /// name here wins the tie the alphabet would otherwise decide. Empty for
    /// a provider whose folders are unambiguous today -- there is nothing to
    /// disambiguate until a real account demonstrates the look-alike, the
    /// way iCloud's did (#501, #943).
    #[serde(default)]
    pub mailboxes: BTreeMap<String, String>,
}

/// Every row that predates #545, and most rows after it.
fn default_backend() -> Vec<String> {
    vec![BACKEND_IMAP.to_owned()]
}

/// `[provider.<id>.jmap]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JmapRow {
    /// The RFC 8620 session resource URL the adapter resolves everything
    /// else from.
    #[allow(dead_code)]
    pub session_url: String,
}

/// `[provider.<id>.oauth]`.
///
/// No `#[serde(deny_unknown_fields)]`: unlike [`ProviderRow`], this table can
/// legitimately end up holding a `client_secret` a user mistakenly pasted
/// in, and the only way to find and strip that key without also matching
/// `authorize`/`token` -- the endpoint field names RFC 8414 and ADR 0006
/// both use, which contain a secret marker themselves -- is to let anything
/// not named above land in [`extra`](Self::extra) and scan only that.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OAuthRow {
    /// Where a token may come from -- `"builtin"`, `"broker"`,
    /// `"own-client"`. Interpreting this is #192's job; this module only
    /// carries it through -- unread here for the same reason
    /// [`ProviderRow`]'s doc comment explains.
    #[allow(dead_code)]
    #[serde(default)]
    pub sources: Vec<String>,
    /// The RFC 8414 issuer to discover `authorize`/`token` from, when
    /// neither is given directly.
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub authorize: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub scopes: Vec<String>,
    /// How many days this provider's **refresh** token lives, when the
    /// provider states one.
    ///
    /// Not the access token's hour — that arrives in every token response as
    /// `expires_in` and is persisted from there (#870). This is the grant's
    /// own lifetime, the one whose expiry a refresh cannot recover from and
    /// which no response ever mentions, so the only place it can come from is
    /// the provider's documentation, written down here as data (#954).
    ///
    /// Absent means "no known lifetime", which must behave exactly as Postio
    /// did before this field existed: nothing is marked stale early, and a
    /// dead grant is still discovered the way it always was, by a refresh
    /// being refused.
    #[allow(dead_code)]
    #[serde(default)]
    pub refresh_token_lifetime_days: Option<u32>,
    /// Everything this schema does not name -- scanned for secret-looking
    /// keys and stripped in [`parse`], never otherwise read.
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// The `auth` token that requires an `[oauth]` table naming where its
/// endpoints come from.
const OAUTH2: &str = "oauth2";

/// The backend tokens Postio ships (ADR 0018 Q5).
const BACKEND_IMAP: &str = "imap";
const BACKEND_JMAP: &str = "jmap";
const BACKEND_GMAIL: &str = "gmail";

/// What can go wrong turning `providers.toml` text into rows.
#[derive(Debug)]
pub enum ProvidersError {
    /// The text is not valid TOML, or not this schema.
    Toml(toml::de::Error),
    /// It parsed, but a row breaks a rule the schema alone cannot express.
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

/// A successful parse: the rows, and which keys were removed as secrets on
/// the way -- as dotted paths, never the values, the same redaction
/// `postio_config::secrets` promises everywhere else it runs.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    // build.rs's own compilation checks only `stripped_secrets`; `builtin.rs`
    // reads `rows`.
    #[allow(dead_code)]
    pub rows: BTreeMap<String, ProviderRow>,
    pub stripped_secrets: Vec<String>,
}

/// Parse `text` into its `[provider.<id>]` rows.
///
/// Strips anything `postio_config::secrets::is_secret_key` recognizes out
/// of each row's `[oauth.extra]` -- the fields this schema does not name --
/// before returning: a provider's OAuth client secret has no more business
/// in a file `Ctrl+E` opens in a text editor than a password does in
/// `config.toml` (ADR 0006 Q4). Scanning only `extra`, rather than the
/// deserialized `authorize`/`token` fields themselves, is deliberate:
/// `is_secret_key`'s marker list matches any key containing `"token"` once
/// separators are stripped, which the OAuth token endpoint's own field name
/// does too -- scanning the whole table would delete a legitimate,
/// non-secret endpoint URL along with an actual secret. What was stripped
/// is reported, never its value -- the caller decides what a secret in the
/// *shipped* table means (a build the maintainer must fix) versus one in
/// the *user's* overlay (a mistake to warn about and drop).
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

/// The rule #152 amended ADR 0006 with: a row naming `oauth2` in `auth`
/// must say where its endpoints come from, checked at load time rather
/// than discovered the first time someone tries to sign in with it.
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

/// Merge a user's overlay onto the shipped table, the user's own rows
/// winning on a key collision and adding straight in otherwise.
///
/// A `BTreeMap`'s own `extend` already is this: the later value for a
/// repeated key replaces the earlier one, and every other key survives
/// untouched -- there is no rule left to write by hand.
///
/// `build.rs` never merges an overlay -- there is no user config directory
/// at compile time -- so this is unread there; `builtin.rs` calls it.
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
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            requires_app_password: false,
            password_help_url: None,
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
        // The default, and the one that must behave exactly as today: no
        // deadline, so nothing is ever marked stale ahead of a real refusal.
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
        // Deserializing `File` from the *stripped* table, so the field is
        // simply absent from `OAuthRow` -- there is no `client_secret` to
        // assert `None` on, which is the point: the schema has nowhere to
        // put one.
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
        // Unlike `config.toml`, which must round-trip a newer version's
        // unknown keys, this file is a build-time-validated asset with no
        // write-back path -- a typo in a key name should be loud.
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
