//! Adding an account (#1279, canvas screen 27).
//!
//! **Providers are data, not code.** What an address is recognised as comes
//! from the preset table in `postio-account`, so adding a provider is a row
//! rather than a branch, and neither frontend contains the word "Gmail" in a
//! condition. This module turns that table's answer into the three things
//! step 1 of the sheet draws: a verdict, a route to pre-focus, and the
//! servers to fill in.

/// Which of the sheet's four routes an address suggests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RouteFfi {
    /// Outlook / Microsoft 365 — OAuth in the system browser.
    Outlook,
    /// Gmail / Google Workspace — OAuth in the system browser.
    Gmail,
    /// IMAP / SMTP with a password in the Keychain.
    Imap,
    /// A maildir, mbox or notmuch store that already exists on this machine.
    LocalStore,
}

/// What the preset table knows about an address.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProviderHintFfi {
    /// The verdict strip: `northgate.example uses Microsoft 365 — Outlook
    /// sign-in recommended.`
    pub verdict: String,
    /// The provider's own display name, or the domain when nobody claims it.
    pub provider: String,
    /// Which route to pre-focus.
    pub route: RouteFfi,
    /// The IMAP host, **empty when nothing is known**.
    ///
    /// Deliberately not `imap.<domain>`: deriving a host from an address is
    /// the guess that dials somebody else's server, and an empty field asks
    /// the one person who knows.
    pub imap_host: String,
    /// The IMAP port, defaulted to the implicit-TLS one rather than left at
    /// zero: a port field showing `0` is a form asking a question the user
    /// has no way to answer.
    pub imap_port: u16,
    /// The SMTP host, empty when nothing is known — see `imap_host`.
    pub smtp_host: String,
    /// The submission port.
    pub smtp_port: u16,
    /// Whether this provider wants an app password rather than the account's
    /// own — the difference between "wrong password" and "not that password".
    pub requires_app_password: bool,
}

/// What the sheet should show for `address`.
#[uniffi::export]
pub fn provider_hint(address: String) -> ProviderHintFfi {
    let domain = address
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_ascii_lowercase())
        .unwrap_or_default();

    let Some(preset) = postio_account::discovery::preset_for_domain(&domain) else {
        return ProviderHintFfi {
            verdict: if domain.is_empty() {
                "Type an email address to begin.".to_owned()
            } else {
                // Not a failure: a self-hosted domain is the ordinary case
                // for this route, and saying so is kinder than a blank strip.
                format!("{domain} is not a provider Postio knows — IMAP / SMTP, then.")
            },
            provider: domain,
            route: RouteFfi::Imap,
            imap_host: String::new(),
            imap_port: 993,
            smtp_host: String::new(),
            smtp_port: 465,
            requires_app_password: false,
        };
    };

    let name = preset.display_name().to_owned();
    let oauth = preset.auth().iter().any(|method| method == "oauth2");
    let route = match () {
        _ if !oauth => RouteFfi::Imap,
        // The two the sheet names separately, matched on the preset's own
        // name rather than on the domain: an organisation on Microsoft 365
        // with its own domain has to land here too, which is exactly the case
        // in the canvas.
        _ if name.to_lowercase().contains("outlook")
            || name.to_lowercase().contains("microsoft") =>
        {
            RouteFfi::Outlook
        }
        _ if name.to_lowercase().contains("google") || name.to_lowercase().contains("gmail") => {
            RouteFfi::Gmail
        }
        _ => RouteFfi::Imap,
    };

    let verdict = if oauth {
        format!("{domain} uses {name} — signing in through your browser is recommended.")
    } else if preset.requires_app_password() {
        format!("{domain} is {name}, which needs an app password rather than your own.")
    } else {
        format!("{domain} is {name}.")
    };

    ProviderHintFfi {
        verdict,
        provider: name,
        route,
        imap_host: preset.imap_host().to_owned(),
        imap_port: preset.imap_port(),
        smtp_host: preset.smtp_host().to_owned(),
        smtp_port: preset.smtp_port(),
        requires_app_password: preset.requires_app_password(),
    }
}
