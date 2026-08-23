//! Providers Postio knows about without asking the network.
//!
//! This table exists for one reason: iCloud publishes no Thunderbird
//! autoconfig document and no RFC 6186 SRV records for `icloud.com`, so the
//! generic probe chain finds nothing for the provider v1 targets. Hard-coding
//! the published settings also makes the first screen instant and works with
//! no network at all.

use crate::discovery::settings::{AccountSettings, Encryption, ServerSettings, SettingsSource};

/// Apple's published iCloud Mail settings.
const ICLOUD_IMAP_HOST: &str = "imap.mail.me.com";
const ICLOUD_SMTP_HOST: &str = "smtp.mail.me.com";

/// The three domains Apple issues iCloud Mail addresses on.
const ICLOUD_DOMAINS: [&str; 3] = ["icloud.com", "me.com", "mac.com"];

/// iCloud has no OAuth path for third-party clients, so the only credential
/// that works is an app-specific password minted at appleid.apple.com.
const ICLOUD_NOTE: &str = "iCloud requires an app-specific password: sign in at \
                           appleid.apple.com, generate one under App-Specific Passwords, \
                           and paste it here. Your Apple ID password will not work.";

/// Returns settings for `domain` when Postio ships them, without any I/O.
///
/// `domain` is matched case-insensitively.
pub fn lookup(email: &str, local_part: &str, domain: &str) -> Option<AccountSettings> {
    let domain = domain.to_ascii_lowercase();

    if ICLOUD_DOMAINS.contains(&domain.as_str()) {
        return Some(icloud(email, local_part));
    }

    None
}

fn icloud(email: &str, _local_part: &str) -> AccountSettings {
    AccountSettings {
        email: email.to_owned(),
        imap: ServerSettings::new(ICLOUD_IMAP_HOST, 993, Encryption::Tls),
        smtp: ServerSettings::new(ICLOUD_SMTP_HOST, 465, Encryption::Tls),
        // Apple wants the full address, including the alias domain the user
        // actually typed, not a rewritten @example.com form.
        login: email.to_owned(),
        source: SettingsSource::Builtin,
        requires_app_password: true,
        note: Some(ICLOUD_NOTE.to_owned()),
        display_name: Some("iCloud".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_icloud_domains_are_known() {
        for domain in ["icloud.com", "me.com", "mac.com"] {
            let email = format!("a@{domain}");
            let settings = lookup(&email, "a", domain).expect("known provider");
            assert_eq!(settings.imap.host, ICLOUD_IMAP_HOST);
            assert!(settings.requires_app_password);
        }
    }

    #[test]
    fn an_unknown_domain_is_not_in_the_table() {
        assert!(lookup("a@example.org", "a", "example.org").is_none());
        // Not a substring match: a lookalike domain must not be claimed.
        assert!(lookup("a@notexample.invalid", "a", "noticloud.com").is_none());
        assert!(lookup("a@example.com.evil.test", "a", "icloud.com.evil.test").is_none());
    }

    #[test]
    fn the_login_keeps_the_domain_the_user_typed() {
        let settings = lookup("someone@example.com", "someone", "me.com").unwrap();
        assert_eq!(settings.login, "someone@example.com");
    }
}
