//! Providers Postio ships settings for, as data.
//!
//! The table exists because the generic probe chain does not always find
//! anything: some providers publish neither a Thunderbird autoconfig document
//! nor RFC 6186 SRV records, and a first-run screen that has to fall back to
//! manual entry for a mainstream provider is a bad first impression. Shipping
//! the published settings also makes that screen instant, and works offline.
//!
//! **Every provider is one row.** No named constant, no special-cased branch,
//! no identifier that mentions a vendor. Adding the next provider is adding a
//! `[provider.<id>]` table to `data/providers.toml` and nothing else --
//! `docs/PRODUCT.md` §3 asks for exactly that, and issue #191 is what makes
//! it literally true: the row used to have to be a [`Preset`] literal
//! written in Rust. `providers_toml` is the schema and the parser; `build.rs`
//! fails the build if the shipped file does not satisfy it; a user's own
//! `$XDG_CONFIG_HOME/postio/providers.toml` is layered on top at process
//! start, in the same shape, with the user's own rows winning on a
//! collision.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::discovery::providers_toml::{self, OAuthRow, ProviderRow, Security};
use crate::discovery::settings::{AccountSettings, Encryption, ServerSettings, SettingsSource};

/// What a provider that refuses ordinary account passwords has to say before
/// the password field, worded without naming anybody: the sentence is the
/// same for every such provider, and a row that spelled out a vendor's name
/// here would be a special case wearing a string's clothes. Derived from
/// `requires_app_password` rather than carried in `providers.toml` itself,
/// so no row can accidentally say something different.
const APP_PASSWORD_NOTE: &str = "This provider requires an app-specific password: generate one in your \
     account settings and paste it here. Your ordinary account password will \
     not work.";

/// One provider's published settings.
#[derive(Debug, Clone)]
pub struct Preset {
    row: ProviderRow,
}

impl From<Security> for Encryption {
    fn from(security: Security) -> Self {
        match security {
            Security::Tls => Encryption::Tls,
            Security::StartTls => Encryption::StartTls,
            Security::None => Encryption::None,
        }
    }
}

impl From<ProviderRow> for Preset {
    fn from(row: ProviderRow) -> Self {
        Preset { row }
    }
}

impl Preset {
    /// The provider's own display name.
    pub fn display_name(&self) -> &str {
        &self.row.display_name
    }

    /// Every domain this provider issues addresses on.
    pub fn domains(&self) -> &[String] {
        &self.row.domains
    }

    /// The provider's IMAP host.
    pub fn imap_host(&self) -> &str {
        &self.row.imap_host
    }

    /// The provider's submission host.
    pub fn smtp_host(&self) -> &str {
        &self.row.smtp_host
    }

    /// The IMAP port to connect on.
    pub fn imap_port(&self) -> u16 {
        self.row.imap_port
    }

    /// The submission port to connect on.
    pub fn smtp_port(&self) -> u16 {
        self.row.smtp_port
    }

    /// Whether only an application-specific password will work.
    pub fn requires_app_password(&self) -> bool {
        self.row.requires_app_password
    }

    /// The preference order among the ways this provider can be signed
    /// into -- `"app-password"`, `"oauth2"`.
    pub fn auth(&self) -> &[String] {
        &self.row.auth
    }

    /// Where this provider's OAuth endpoints come from, if it offers OAuth
    /// at all. `providers_toml::validate` already guarantees that a row
    /// naming `"oauth2"` in [`auth`](Self::auth) has one of these.
    pub fn oauth(&self) -> Option<&OAuthRow> {
        self.row.oauth.as_ref()
    }

    /// A sentence the user has to read before typing a password, when this
    /// provider requires an app-specific one.
    fn note(&self) -> Option<&'static str> {
        self.row.requires_app_password.then_some(APP_PASSWORD_NOTE)
    }

    /// Where to generate an app-specific password, when
    /// [`requires_app_password`](Self::requires_app_password) is set.
    pub fn password_help_url(&self) -> Option<&str> {
        self.row.password_help_url.as_deref()
    }

    /// Whether this provider issues addresses on `domain`.
    ///
    /// Whole-domain equality, never a substring or suffix match: a lookalike
    /// domain must not inherit a real provider's servers.
    fn claims(&self, domain: &str) -> bool {
        self.row
            .domains
            .iter()
            .any(|known| known.eq_ignore_ascii_case(domain))
    }

    /// This provider's published settings, for `email`.
    pub fn settings_for(&self, email: &str) -> AccountSettings {
        AccountSettings {
            email: email.to_owned(),
            imap: ServerSettings::new(
                &self.row.imap_host,
                self.row.imap_port,
                self.row.imap_security.into(),
            ),
            smtp: ServerSettings::new(
                &self.row.smtp_host,
                self.row.smtp_port,
                self.row.smtp_security.into(),
            ),
            // The login keeps the domain the user actually typed rather than a
            // rewritten primary-domain form: providers with alias domains
            // expect the address as issued.
            login: email.to_owned(),
            source: SettingsSource::Builtin,
            requires_app_password: self.row.requires_app_password,
            note: self.note().map(str::to_owned),
            password_help_url: self.row.password_help_url.clone(),
            display_name: Some(self.row.display_name.clone()),
        }
    }
}

/// The shipped table, embedded at compile time and parsed once.
///
/// `build.rs` already parsed and validated this exact file with the same
/// module before the crate compiled at all -- a parse failure here would
/// mean the build itself had already failed, so the `expect` cannot fire in
/// a build that succeeded.
fn shipped_rows() -> BTreeMap<String, ProviderRow> {
    let parsed = providers_toml::parse(include_str!("../../data/providers.toml"))
        .expect("build.rs already validated data/providers.toml");
    for path in &parsed.stripped_secrets {
        tracing::warn!(
            path,
            "a secret-looking key was stripped from the shipped providers.toml"
        );
    }
    parsed.rows
}

/// The user's own overlay, read from an arbitrary environment lookup so it
/// is testable without touching the process environment.
///
/// A missing file is not an error -- most installs will never have one. A
/// file that exists but fails to parse or validate is logged and dropped
/// wholesale rather than partially applied: this is much lower stakes than
/// `config.toml`, whose accounts and credentials the whole app depends on --
/// worst case here, a custom provider does not appear and onboarding falls
/// back to manual entry, exactly as an unrecognised domain already does.
fn overlay_rows_from<F>(env: F) -> BTreeMap<String, ProviderRow>
where
    F: Fn(&str) -> Option<String>,
{
    let Ok(dir) = postio_config::paths::config_dir_from(env) else {
        return BTreeMap::new();
    };
    let path = dir.join("providers.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    match providers_toml::parse(&text) {
        Ok(parsed) => {
            for stripped in &parsed.stripped_secrets {
                tracing::warn!(
                    path = stripped,
                    "a secret-looking key was stripped from the providers.toml overlay"
                );
            }
            parsed.rows
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "providers.toml overlay could not be used");
            BTreeMap::new()
        }
    }
}

fn overlay_rows() -> BTreeMap<String, ProviderRow> {
    overlay_rows_from(|key| std::env::var(key).ok())
}

/// The table. One row per provider, shipped plus the user's own overlay,
/// merged with the user's rows winning -- computed once and shared for the
/// life of the process.
static PRESETS: LazyLock<Vec<Preset>> = LazyLock::new(|| {
    providers_toml::merged(shipped_rows(), overlay_rows())
        .into_values()
        .map(Preset::from)
        .collect()
});

/// Every provider Postio ships settings for.
pub fn presets() -> &'static [Preset] {
    &PRESETS
}

/// The preset covering `domain`, if Postio ships one.
pub fn preset_for_domain(domain: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.claims(domain))
}

/// Returns settings for `domain` when Postio ships them, without any I/O.
///
/// `domain` is matched case-insensitively.
pub fn lookup(email: &str, _local_part: &str, domain: &str) -> Option<AccountSettings> {
    preset_for_domain(domain).map(|preset| preset.settings_for(email))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Addresses are built rather than written out, so no literal address in a
    /// provider's real domain ever appears in the source. See CLAUDE.md,
    /// "No personal data".
    fn address_in(domain: &str) -> String {
        format!("a@{domain}")
    }

    #[test]
    fn every_domain_in_the_table_resolves_to_its_own_row() {
        for preset in presets() {
            for domain in preset.domains() {
                let email = address_in(domain);
                let settings = lookup(&email, "a", domain).expect("a table row");

                assert_eq!(settings.imap.host, preset.imap_host());
                assert_eq!(settings.smtp.host, preset.smtp_host());
                assert_eq!(
                    settings.display_name.as_deref(),
                    Some(preset.display_name())
                );
                assert_eq!(settings.source, SettingsSource::Builtin);
            }
        }
    }

    #[test]
    fn domain_matching_ignores_case() {
        for preset in presets() {
            let domain = preset.domains()[0].to_uppercase();
            assert!(lookup(&address_in(&domain), "a", &domain).is_some());
        }
    }

    #[test]
    fn an_unknown_domain_is_not_in_the_table() {
        assert!(lookup("a@example.org", "a", "example.org").is_none());
    }

    #[test]
    fn a_lookalike_domain_does_not_inherit_a_providers_servers() {
        for preset in presets() {
            let real = &preset.domains()[0];
            for lookalike in [format!("not{real}"), format!("{real}.evil.test")] {
                assert!(
                    lookup(&address_in(&lookalike), "a", &lookalike).is_none(),
                    "{lookalike} was matched against {}",
                    preset.display_name()
                );
            }
        }
    }

    #[test]
    fn the_login_keeps_the_domain_the_user_typed() {
        for preset in presets() {
            // An alias domain, not the provider's primary one.
            let domain = preset.domains().last().expect("at least one domain");
            let email = address_in(domain);
            let settings = lookup(&email, "a", domain).unwrap();

            assert_eq!(settings.login, email);
        }
    }

    #[test]
    fn a_provider_that_refuses_account_passwords_says_so_without_naming_itself() {
        for preset in presets().iter().filter(|p| p.requires_app_password()) {
            let note = preset
                .note()
                .expect("an app-password provider needs a note");
            assert!(note.contains("app-specific password"));
        }
    }

    #[test]
    fn the_named_providers_the_first_run_screen_has_to_cover_are_in_the_table() {
        // These were a second, hardcoded table in
        // `postio-app/examples/provision.rs::known` -- the same data in two
        // places, only one of which the onboarding screen could reach.
        // Issue #69 asks for one table both callers use.
        for (domain, imap, smtp) in [
            ("icloud.com", "imap.mail.me.com", "smtp.mail.me.com"),
            ("me.com", "imap.mail.me.com", "smtp.mail.me.com"),
            ("mac.com", "imap.mail.me.com", "smtp.mail.me.com"),
            ("gmail.com", "imap.gmail.com", "smtp.gmail.com"),
            ("googlemail.com", "imap.gmail.com", "smtp.gmail.com"),
            ("fastmail.com", "imap.fastmail.com", "smtp.fastmail.com"),
            ("fastmail.fm", "imap.fastmail.com", "smtp.fastmail.com"),
        ] {
            let settings = lookup(&address_in(domain), "a", domain)
                .unwrap_or_else(|| panic!("{domain} is not in the table"));
            assert_eq!(settings.imap.host, imap, "{domain}");
            assert_eq!(settings.smtp.host, smtp, "{domain}");
        }
    }

    #[test]
    fn no_domain_is_claimed_by_two_providers() {
        // Two rows matching one domain is a silent bug: `lookup` takes the
        // first, and which row that is depends on table order.
        let mut seen: Vec<String> = Vec::new();
        for preset in presets() {
            for domain in preset.domains() {
                assert!(
                    !seen.contains(domain),
                    "{domain} appears in more than one row"
                );
                seen.push(domain.clone());
            }
        }
    }

    #[test]
    fn every_row_is_usable_as_written() {
        // The table is data, and data gets typed. A row with an empty host or
        // a zero port would reach the first-run screen as a prefilled form
        // that cannot possibly connect.
        for preset in presets() {
            assert!(!preset.display_name().is_empty());
            assert!(!preset.domains().is_empty(), "{}", preset.display_name());
            for (label, host, port) in [
                ("imap", preset.imap_host(), preset.imap_port()),
                ("smtp", preset.smtp_host(), preset.smtp_port()),
            ] {
                assert!(
                    host.contains('.') && !host.starts_with('.'),
                    "{} {label} host is not a hostname: {host:?}",
                    preset.display_name()
                );
                assert!(port > 0, "{} {label} port is zero", preset.display_name());
            }
            for domain in preset.domains() {
                assert_eq!(
                    domain,
                    &domain.to_ascii_lowercase(),
                    "{} lists {domain} in mixed case, which `lookup` folds and so never matches",
                    preset.display_name()
                );
            }
        }
    }

    #[test]
    fn a_provider_that_requires_an_app_password_links_to_where_to_make_one() {
        for preset in presets().iter().filter(|p| p.requires_app_password()) {
            let url = preset
                .password_help_url()
                .expect("an app-password provider needs a help link");
            assert!(url.starts_with("https://"));

            let settings = preset.settings_for(&address_in(&preset.domains()[0]));
            assert_eq!(settings.password_help_url.as_deref(), Some(url));
        }
    }

    // ── issue #191: the shipped table is a compiled TOML asset ────────────

    #[test]
    fn the_shipped_file_is_the_same_module_the_build_script_already_validated() {
        // Not a tautology: this proves `data/providers.toml` still parses
        // and validates *by itself*, through the exact function `build.rs`
        // calls, independent of whether `presets()` has been touched yet in
        // this test binary. If the two ever disagreed, this is how you'd
        // find out without waiting for a build to fail.
        let parsed = providers_toml::parse(include_str!("../../data/providers.toml"))
            .expect("the checked-in file must parse and validate on its own");
        assert_eq!(parsed.rows.len(), presets().len());
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn a_missing_overlay_yields_no_rows() {
        let rows = overlay_rows_from(env_of(&[("XDG_CONFIG_HOME", "/nonexistent-for-a-test")]));
        assert!(rows.is_empty());
    }

    #[test]
    fn an_overlay_row_wins_over_a_shipped_row_with_the_same_id() {
        let dir =
            std::env::temp_dir().join(format!("postio-providers-overlay-{}", std::process::id()));
        let config_dir = dir.join("postio");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("providers.toml"),
            r#"
            [provider.icloud]
            display_name = "My iCloud Override"
            domains = ["icloud.com", "me.com", "mac.com"]
            imap_host = "imap.example.com"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.com"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]

            [provider.mine]
            display_name = "My Own Provider"
            domains = ["example.org"]
            imap_host = "imap.example.org"
            imap_port = 993
            imap_security = "tls"
            smtp_host = "smtp.example.org"
            smtp_port = 465
            smtp_security = "tls"
            auth = ["app-password"]
            "#,
        )
        .unwrap();

        let overlay = overlay_rows_from(env_of(&[("XDG_CONFIG_HOME", dir.to_str().unwrap())]));
        let shipped = shipped_rows();
        let merged = providers_toml::merged(shipped, overlay);

        assert_eq!(
            merged["icloud"].display_name, "My iCloud Override",
            "the user's own row must win, not the shipped one"
        );
        assert_eq!(merged["mine"].display_name, "My Own Provider");
        assert!(
            merged.contains_key("gmail"),
            "a shipped row the overlay never mentioned must survive"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
