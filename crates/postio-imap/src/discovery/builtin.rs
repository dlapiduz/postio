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
//! [`Preset`] to [`PRESETS`] and nothing else; `docs/PRODUCT.md` §3 asks for exactly
//! that, and it is what keeps Postio a mail client rather than one provider's
//! client.

use crate::discovery::settings::{AccountSettings, Encryption, ServerSettings, SettingsSource};

/// One provider's published settings.
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    /// The provider's own name, for the onboarding screen.
    display_name: &'static str,
    /// Every domain the provider issues addresses on, lower case.
    domains: &'static [&'static str],
    /// IMAP host, port and transport security.
    imap: (&'static str, u16, Encryption),
    /// Submission host, port and transport security.
    smtp: (&'static str, u16, Encryption),
    /// Whether the account password is refused and only an application
    /// specific password works.
    requires_app_password: bool,
    /// A sentence the user has to read before typing a password.
    note: Option<&'static str>,
    /// Where to generate an app-specific password, when `requires_app_password`
    /// is set.
    password_help_url: Option<&'static str>,
}

/// What a provider that refuses ordinary account passwords has to say before
/// the password field, worded without naming anybody: the sentence is the
/// same for every such provider, and a row that spelled out a vendor's name
/// here would be a special case wearing a string's clothes.
const APP_PASSWORD_NOTE: &str = "This provider requires an app-specific password: generate one in your \
     account settings and paste it here. Your ordinary account password will \
     not work.";

/// The table. One row per provider, in display order.
///
/// Ordinary IMAP and SMTP with a password only. A provider that has turned
/// password authentication off for good — the ones that now require OAuth —
/// does not belong here while v1 speaks no OAuth: a prefilled form that
/// cannot possibly authenticate is worse than the empty one, because the
/// user blames the password they typed.
static PRESETS: &[Preset] = &[
    Preset {
        display_name: "iCloud",
        domains: &["icloud.com", "me.com", "mac.com"],
        imap: ("imap.mail.me.com", 993, Encryption::Tls),
        smtp: ("smtp.mail.me.com", 465, Encryption::Tls),
        requires_app_password: true,
        note: Some(APP_PASSWORD_NOTE),
        password_help_url: Some("https://appleid.apple.com/account/manage"),
    },
    Preset {
        display_name: "Gmail",
        domains: &["gmail.com", "googlemail.com"],
        imap: ("imap.gmail.com", 993, Encryption::Tls),
        smtp: ("smtp.gmail.com", 465, Encryption::Tls),
        requires_app_password: true,
        note: Some(APP_PASSWORD_NOTE),
        password_help_url: Some("https://myaccount.google.com/apppasswords"),
    },
    Preset {
        display_name: "Fastmail",
        domains: &["fastmail.com", "fastmail.fm"],
        imap: ("imap.fastmail.com", 993, Encryption::Tls),
        smtp: ("smtp.fastmail.com", 465, Encryption::Tls),
        requires_app_password: true,
        note: Some(APP_PASSWORD_NOTE),
        password_help_url: Some("https://app.fastmail.com/settings/security/devicekeys"),
    },
];

impl Preset {
    /// The provider's own display name.
    pub fn display_name(&self) -> &'static str {
        self.display_name
    }

    /// Every domain this provider issues addresses on.
    pub fn domains(&self) -> &'static [&'static str] {
        self.domains
    }

    /// The provider's IMAP host.
    pub fn imap_host(&self) -> &'static str {
        self.imap.0
    }

    /// The provider's submission host.
    pub fn smtp_host(&self) -> &'static str {
        self.smtp.0
    }

    /// The IMAP port to connect on.
    pub fn imap_port(&self) -> u16 {
        self.imap.1
    }

    /// The submission port to connect on.
    pub fn smtp_port(&self) -> u16 {
        self.smtp.1
    }

    /// Whether only an application-specific password will work.
    pub fn requires_app_password(&self) -> bool {
        self.requires_app_password
    }

    /// Whether this provider issues addresses on `domain`.
    ///
    /// Whole-domain equality, never a substring or suffix match: a lookalike
    /// domain must not inherit a real provider's servers.
    fn claims(&self, domain: &str) -> bool {
        self.domains
            .iter()
            .any(|known| known.eq_ignore_ascii_case(domain))
    }

    /// This provider's published settings, for `email`.
    pub fn settings_for(&self, email: &str) -> AccountSettings {
        AccountSettings {
            email: email.to_owned(),
            imap: ServerSettings::new(self.imap.0, self.imap.1, self.imap.2),
            smtp: ServerSettings::new(self.smtp.0, self.smtp.1, self.smtp.2),
            // The login keeps the domain the user actually typed rather than a
            // rewritten primary-domain form: providers with alias domains
            // expect the address as issued.
            login: email.to_owned(),
            source: SettingsSource::Builtin,
            requires_app_password: self.requires_app_password,
            note: self.note.map(str::to_owned),
            password_help_url: self.password_help_url.map(str::to_owned),
            display_name: Some(self.display_name.to_owned()),
        }
    }
}

/// Every provider Postio ships settings for.
pub fn presets() -> &'static [Preset] {
    PRESETS
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
        for preset in PRESETS {
            for domain in preset.domains {
                let email = address_in(domain);
                let settings = lookup(&email, "a", domain).expect("a table row");

                assert_eq!(settings.imap.host, preset.imap.0);
                assert_eq!(settings.smtp.host, preset.smtp.0);
                assert_eq!(settings.display_name.as_deref(), Some(preset.display_name));
                assert_eq!(settings.source, SettingsSource::Builtin);
            }
        }
    }

    #[test]
    fn domain_matching_ignores_case() {
        for preset in PRESETS {
            let domain = preset.domains[0].to_uppercase();
            assert!(lookup(&address_in(&domain), "a", &domain).is_some());
        }
    }

    #[test]
    fn an_unknown_domain_is_not_in_the_table() {
        assert!(lookup("a@example.org", "a", "example.org").is_none());
    }

    #[test]
    fn a_lookalike_domain_does_not_inherit_a_providers_servers() {
        for preset in PRESETS {
            let real = preset.domains[0];
            for lookalike in [format!("not{real}"), format!("{real}.evil.test")] {
                assert!(
                    lookup(&address_in(&lookalike), "a", &lookalike).is_none(),
                    "{lookalike} was matched against {}",
                    preset.display_name
                );
            }
        }
    }

    #[test]
    fn the_login_keeps_the_domain_the_user_typed() {
        for preset in PRESETS {
            // An alias domain, not the provider's primary one.
            let domain = preset.domains.last().expect("at least one domain");
            let email = address_in(domain);
            let settings = lookup(&email, "a", domain).unwrap();

            assert_eq!(settings.login, email);
        }
    }

    #[test]
    fn a_provider_that_refuses_account_passwords_says_so_without_naming_itself() {
        for preset in PRESETS.iter().filter(|p| p.requires_app_password) {
            let note = preset.note.expect("an app-password provider needs a note");
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
        let mut seen: Vec<&str> = Vec::new();
        for preset in PRESETS {
            for domain in preset.domains {
                assert!(
                    !seen.contains(domain),
                    "{domain} appears in more than one row"
                );
                seen.push(domain);
            }
        }
    }

    #[test]
    fn every_row_is_usable_as_written() {
        // The table is data, and data gets typed. A row with an empty host or
        // a zero port would reach the first-run screen as a prefilled form
        // that cannot possibly connect.
        for preset in PRESETS {
            assert!(!preset.display_name.is_empty());
            assert!(!preset.domains.is_empty(), "{}", preset.display_name);
            for (label, (host, port, _)) in [("imap", preset.imap), ("smtp", preset.smtp)] {
                assert!(
                    host.contains('.') && !host.starts_with('.'),
                    "{} {label} host is not a hostname: {host:?}",
                    preset.display_name
                );
                assert!(port > 0, "{} {label} port is zero", preset.display_name);
            }
            for domain in preset.domains {
                assert_eq!(
                    *domain,
                    domain.to_ascii_lowercase(),
                    "{} lists {domain} in mixed case, which `lookup` folds and so never matches",
                    preset.display_name
                );
            }
        }
    }

    #[test]
    fn a_provider_that_requires_an_app_password_links_to_where_to_make_one() {
        for preset in PRESETS.iter().filter(|p| p.requires_app_password) {
            let url = preset
                .password_help_url
                .expect("an app-password provider needs a help link");
            assert!(url.starts_with("https://"));

            let settings = preset.settings_for(&address_in(preset.domains[0]));
            assert_eq!(settings.password_help_url.as_deref(), Some(url));
        }
    }
}
