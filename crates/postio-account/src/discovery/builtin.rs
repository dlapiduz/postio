//! Every provider is one row in `data/providers.toml` (#191): no named constant, no special-cased
//! branch.
//! `build.rs` validates the shipped file; a user's `providers.toml` overlay is layered on top and
//! wins on collision.

use std::collections::BTreeMap;

use postio_config::paths::Platform;
use postio_model::MailboxRole;
use std::sync::LazyLock;

use crate::discovery::providers_toml::{self, CaldavRow, OAuthRow, ProviderRow, Security};
use crate::discovery::settings::{AccountSettings, Encryption, ServerSettings, SettingsSource};

/// Worded without naming anybody and derived from `requires_app_password`, so no row can say
/// something different.
const APP_PASSWORD_NOTE: &str = "This provider requires an app-specific password: generate one in your \
     account settings and paste it here. Your ordinary account password will \
     not work.";

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
    pub fn display_name(&self) -> &str {
        &self.row.display_name
    }

    pub fn domains(&self) -> &[String] {
        &self.row.domains
    }

    pub fn mx_suffixes(&self) -> &[String] {
        &self.row.mx_suffixes
    }

    /// Suffix match at a label boundary, unlike `claims`: providers hand out one MX host per
    /// customer, but `evil-mail.protection.example` must not inherit `mail.protection.example`.
    /// Returns the length of the longest matching suffix, which orders rows that both match.
    fn mx_match_len(&self, host: &str) -> Option<usize> {
        let host = host.trim_end_matches('.');
        if host.is_empty() {
            return None;
        }
        self.row
            .mx_suffixes
            .iter()
            .filter_map(|suffix| {
                let suffix = suffix.trim_end_matches('.');
                if suffix.is_empty() {
                    return None;
                }
                if host.eq_ignore_ascii_case(suffix) {
                    return Some(suffix.len());
                }
                let boundary = host.len().checked_sub(suffix.len())?;
                (boundary > 0
                    && host[boundary..].eq_ignore_ascii_case(suffix)
                    && host.as_bytes()[boundary - 1] == b'.')
                    .then_some(suffix.len())
            })
            .max()
    }

    /// The reader trusts an `Authentication-Results` header only when its authserv-id is one of
    /// these (whole or label-bounded suffix); anything else was written by the sender or a relay
    /// (#1928).
    pub fn authserv_ids(&self) -> &[String] {
        &self.row.authserv_ids
    }

    pub fn imap_host(&self) -> &str {
        &self.row.imap_host
    }

    pub fn smtp_host(&self) -> &str {
        &self.row.smtp_host
    }

    pub fn imap_port(&self) -> u16 {
        self.row.imap_port
    }

    pub fn smtp_port(&self) -> u16 {
        self.row.smtp_port
    }

    pub fn change_feed(&self) -> postio_model::ChangeFeed {
        if self.row.condstore_reliable {
            postio_model::ChangeFeed::Reliable
        } else {
            postio_model::ChangeFeed::Unreliable
        }
    }

    pub fn requires_app_password(&self) -> bool {
        self.row.requires_app_password
    }

    /// Preference order, e.g. `"app-password"`, `"oauth2"`.
    pub fn auth(&self) -> &[String] {
        &self.row.auth
    }

    /// `providers_toml::validate` guarantees a row naming `"oauth2"` in `auth` has one.
    pub fn oauth(&self) -> Option<&OAuthRow> {
        self.row.oauth.as_ref()
    }

    fn note(&self) -> Option<&'static str> {
        self.row.requires_app_password.then_some(APP_PASSWORD_NOTE)
    }

    pub fn caldav(&self) -> Option<&CaldavRow> {
        self.row.caldav.as_ref()
    }

    pub fn password_help_url(&self) -> Option<&str> {
        self.row.password_help_url.as_deref()
    }

    /// For servers whose `LIST` offers no `SPECIAL-USE` (#959), e.g. iCloud. Unrecognised role keys
    /// are dropped, not an error: the table is maintainer-authored.
    pub fn role_names(&self) -> BTreeMap<MailboxRole, String> {
        self.row
            .mailboxes
            .iter()
            .filter_map(|(role, path)| Some((MailboxRole::from_name(role)?, path.clone())))
            .collect()
    }

    /// Whole-domain equality, never substring or suffix: a lookalike domain must not inherit a real
    /// provider's servers.
    fn claims(&self, domain: &str) -> bool {
        self.row
            .domains
            .iter()
            .any(|known| known.eq_ignore_ascii_case(domain))
    }

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
            // Keeps the domain the user typed: alias-domain providers expect the address as issued.
            login: email.to_owned(),
            source: SettingsSource::Builtin,
            requires_app_password: self.row.requires_app_password,
            note: self.note().map(str::to_owned),
            password_help_url: self.row.password_help_url.clone(),
            display_name: Some(self.row.display_name.clone()),
            // Only when the row prefers oauth2: listed second, it is a fallback reached through the
            // client-id path.
            jmap: self
                .row
                .backend
                .iter()
                .any(|backend| backend == "jmap")
                .then_some(self.row.jmap.as_ref())
                .flatten()
                .map(|jmap| crate::discovery::settings::JmapOffer {
                    session_url: jmap.session_url.clone(),
                }),
            backends: self.row.backend.clone(),
            oauth: self
                .row
                .auth
                .first()
                .filter(|method| method.as_str() == "oauth2")
                .and(self.row.oauth.as_ref())
                .map(|oauth| crate::discovery::settings::OAuthOffer {
                    issuer: oauth.issuer.clone(),
                    authorize: oauth.authorize.clone(),
                    token: oauth.token.clone(),
                    scopes: oauth.scopes.clone(),
                    refresh_token_lifetime_days: oauth.refresh_token_lifetime_days,
                }),
        }
    }
}

/// `build.rs` already validated this file, so the `expect` cannot fire.
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

/// A missing file is not an error. One that fails to parse is logged and dropped wholesale: worst
/// case a custom provider does not appear and onboarding falls back to manual entry.
fn overlay_rows_from<F>(env: F) -> BTreeMap<String, ProviderRow>
where
    F: Fn(&str) -> Option<String>,
{
    // Beside `config.toml` on every platform, so one moved implies the other (#556).
    let Ok(dir) = postio_config::paths::config_dir_from(env, Platform::host()) else {
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

static PRESETS: LazyLock<Vec<Preset>> = LazyLock::new(|| {
    providers_toml::merged(shipped_rows(), overlay_rows())
        .into_values()
        .map(Preset::from)
        .collect()
});

pub fn presets() -> &'static [Preset] {
    &PRESETS
}

pub fn preset_for_domain(domain: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.claims(domain))
}

/// The longest suffix wins, so a specific row beats a general one regardless of table order.
pub fn preset_for_mx_host(host: &str) -> Option<&'static Preset> {
    PRESETS
        .iter()
        .filter_map(|preset| preset.mx_match_len(host).map(|len| (len, preset)))
        .max_by_key(|(len, _)| *len)
        .map(|(_, preset)| preset)
}

/// For an already-connected account: the host it talks to now, so a folder-role tie-break can ask
/// which provider this is without redoing discovery (#959). Case-insensitive.
pub fn preset_for_imap_host(host: &str) -> Option<&'static Preset> {
    PRESETS
        .iter()
        .find(|preset| preset.imap_host().eq_ignore_ascii_case(host))
}

/// Empty for a server Postio ships no row for, so no `Authentication-Results` is believed (#1928).
pub fn authserv_ids_for_imap_host(imap_host: &str) -> &'static [String] {
    preset_for_imap_host(imap_host).map_or(&[], Preset::authserv_ids)
}

pub fn lookup(email: &str, _local_part: &str, domain: &str) -> Option<AccountSettings> {
    preset_for_domain(domain).map(|preset| preset.settings_for(email))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_providers_known_to_under_report_condstore_say_so_in_their_row() {
        use postio_model::ChangeFeed;

        let feed = |host: &str| {
            preset_for_imap_host(host)
                .expect("a shipped provider")
                .change_feed()
        };

        assert_eq!(feed("imap.mail.me.com"), ChangeFeed::Unreliable);
        assert_eq!(feed("outlook.office365.com"), ChangeFeed::Unreliable);
        assert_eq!(
            feed("imap.gmail.com"),
            ChangeFeed::Reliable,
            "a row that says nothing is believed"
        );
    }

    /// Built, not written out, so no literal address in a provider's real domain appears in source.
    fn address_in(domain: &str) -> String {
        format!("a@{domain}")
    }

    #[test]
    fn an_account_trusts_the_authserv_ids_of_its_providers_row() {
        let google = authserv_ids_for_imap_host("IMAP.gmail.com");
        assert!(google.iter().any(|id| id == "mx.google.com"), "{google:?}");
        assert!(authserv_ids_for_imap_host("imap.example.org").is_empty());
    }

    /// A field nobody filled in is indistinguishable from one nobody reads, so this asserts the
    /// shipped table carries a refresh lifetime through to the offer (#954).
    #[test]
    fn a_shipped_row_carries_its_refresh_lifetime_through_to_the_offer() {
        let stated: Vec<(&str, Option<u32>)> = presets()
            .iter()
            .filter_map(|preset| {
                let offer = preset.settings_for(&address_in("example.test")).oauth?;
                Some((preset.display_name(), offer.refresh_token_lifetime_days))
            })
            .collect();

        assert!(
            stated.iter().any(|(_, days)| *days == Some(7)),
            "a provider stating seven days should say so: {stated:?}"
        );
        assert!(
            stated.iter().any(|(_, days)| *days == Some(90)),
            "and one stating ninety: {stated:?}"
        );
        // Not asserted: that some shipped row states nothing; that depends on the table's contents.
        // The absent case is pinned on an invented row in `providers_toml`.
    }

    #[test]
    fn an_mx_host_resolves_to_the_provider_whose_suffix_it_ends_with() {
        for preset in presets() {
            for suffix in preset.mx_suffixes() {
                let host = format!("something.{suffix}");
                let found = preset_for_mx_host(&host).expect("a table row");
                assert_eq!(
                    found.display_name(),
                    preset.display_name(),
                    "`{host}` should resolve to the row that published `{suffix}`"
                );
            }
        }
    }

    /// A label boundary, not a string suffix: `evil-mail.protection.example` must not inherit
    /// `mail.protection.example`.
    #[test]
    fn a_suffix_match_stops_at_a_label_boundary() {
        let Some((preset, suffix)) = presets()
            .iter()
            .find_map(|p| p.mx_suffixes().first().map(|s| (p, s.clone())))
        else {
            return; // no row publishes one yet
        };
        assert!(
            preset_for_mx_host(&format!("evil{suffix}")).is_none(),
            "`evil{suffix}` is a different host and must not inherit \
             {}'s servers",
            preset.display_name()
        );
        assert!(
            preset_for_mx_host(&suffix).is_some(),
            "the suffix on its own is that provider's own host"
        );
    }

    #[test]
    fn an_mx_host_nothing_publishes_resolves_to_nothing() {
        assert!(preset_for_mx_host("mx.example.invalid").is_none());
        assert!(preset_for_mx_host("").is_none());
    }

    #[test]
    fn an_imap_host_resolves_to_the_provider_that_publishes_it() {
        for preset in presets() {
            let found = preset_for_imap_host(preset.imap_host())
                .expect("a provider's own imap_host should resolve to itself");
            assert_eq!(found.display_name(), preset.display_name());
        }
    }

    #[test]
    fn an_imap_host_match_is_case_insensitive() {
        let Some(preset) = presets().first() else {
            return; // no row shipped yet
        };
        let shouting = preset.imap_host().to_uppercase();
        assert_eq!(
            preset_for_imap_host(&shouting).map(Preset::display_name),
            Some(preset.display_name())
        );
    }

    #[test]
    fn an_imap_host_nothing_publishes_resolves_to_nothing() {
        assert!(preset_for_imap_host("imap.example.invalid").is_none());
        assert!(preset_for_imap_host("").is_none());
    }

    /// Apple documents 587 with STARTTLS and no other port; 465 is unanswered, so every send spent
    /// the whole retry budget. Asserted through the preset because `starttls` parses as nothing
    /// (the spelling is `start-tls`).
    #[test]
    fn icloud_sends_on_the_port_apple_documents() {
        let icloud = preset_for_domain("icloud.com").expect("iCloud should be a shipped row");
        assert_eq!(icloud.smtp_host(), "smtp.mail.me.com");
        assert_eq!(
            icloud.smtp_port(),
            587,
            "587 is the only port Apple documents"
        );
    }

    #[test]
    fn icloud_names_its_own_role_folders() {
        let icloud = preset_for_domain("icloud.com").expect("iCloud should be a shipped row");
        let names = icloud.role_names();
        assert_eq!(
            names.get(&MailboxRole::Sent).map(String::as_str),
            Some("Sent Messages")
        );
        assert_eq!(
            names.get(&MailboxRole::Trash).map(String::as_str),
            Some("Deleted Messages")
        );
        assert_eq!(
            names.get(&MailboxRole::Archive).map(String::as_str),
            // "Archive", not "Archives": the plural names a folder somebody files years under
            // (#1178).
            Some("Archive")
        );
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
            let domain = preset.domains()[0].as_str();
            let upper = domain.to_uppercase();
            let found = lookup(&address_in(&upper), "a", &upper);
            assert!(found.is_some(), "{upper} found no preset");
            // The same settings the domain's own spelling finds, not just some.
            assert_eq!(found, lookup(&address_in(&upper), "a", domain), "{upper}");
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
    fn a_microsoft_365_domain_resolves_to_the_office365_preset() {
        for domain in ["outlook.com", "hotmail.com", "live.com"] {
            let settings = lookup(&address_in(domain), "a", domain)
                .unwrap_or_else(|| panic!("{domain} is not in the table"));
            assert_eq!(settings.imap.host, "outlook.office365.com");
            assert_eq!(settings.smtp.host, "smtp.office365.com");
            assert_eq!(
                settings.display_name.as_deref(),
                Some("Outlook / Microsoft 365")
            );
        }
    }

    #[test]
    fn a_custom_domain_delegated_to_microsoft_365_resolves_by_its_mx_suffix() {
        // A custom domain is not in `domains`, so this resolves through the MX-suffix path (#94).
        let preset =
            preset_for_mx_host("contoso-com.mail.protection.outlook.com").unwrap_or_else(|| {
                panic!("a Microsoft 365 MX host should resolve to the office365 preset")
            });
        assert_eq!(preset.display_name(), "Outlook / Microsoft 365");
    }

    #[test]
    fn the_office365_preset_prefers_oauth_and_names_its_issuer() {
        let preset = presets()
            .iter()
            .find(|p| p.display_name() == "Outlook / Microsoft 365")
            .expect("the office365 row");
        assert_eq!(preset.auth(), ["oauth2"]);
        assert!(!preset.requires_app_password());
        let oauth = preset.oauth().expect("an oauth table");
        assert_eq!(
            oauth.issuer.as_deref(),
            Some("https://login.microsoftonline.com/common/v2.0")
        );
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
        // A row with an empty host or zero port would reach first-run as a form that cannot
        // connect.
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

    #[test]
    fn the_shipped_file_is_the_same_module_the_build_script_already_validated() {
        // Proves the file parses and validates by itself, through the function `build.rs` calls.
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
