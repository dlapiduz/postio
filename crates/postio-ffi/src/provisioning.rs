//! Adding an account. Providers are data: what an address is recognised as comes from the preset
//! table in `postio-account`,
//! so neither frontend names a provider in a condition.

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RouteFfi {
    Outlook,
    Gmail,
    Imap,
    LocalStore,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ProviderHintFfi {
    /// The verdict strip: `northgate.example uses Microsoft 365 — Outlook
    /// sign-in recommended.`
    pub verdict: String,
    pub provider: String,
    pub route: RouteFfi,
    /// The IMAP host, empty when nothing is known: deriving `imap.<domain>` is the guess that dials
    /// somebody else's server.
    pub imap_host: String,
    /// The IMAP port, defaulted to the implicit-TLS one rather than zero.
    pub imap_port: u16,
    /// The SMTP host, empty when nothing is known — see `imap_host`.
    pub smtp_host: String,
    pub smtp_port: u16,
    /// Whether this provider wants an app password rather than the account's
    /// own — the difference between "wrong password" and "not that password".
    pub requires_app_password: bool,
}

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
        // Matched on the preset's own name, not the domain: an organisation on Microsoft 365 with
        // its own domain lands here too.
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

/// What a sign-in is doing, for the sheet while somebody is away in their browser. The loopback
/// port is the only evidence a
/// mail client can offer that the waiting thing is Postio.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SignInProgressFfi {
    pub waiting: bool,
    /// The loopback port the answer comes back on, once it is listening.
    pub port: u16,
    pub message: String,
}

/// The scopes a sign-in will ask for, and as plainly the ones it will not (contacts, calendar,
/// files).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScopesFfi {
    pub requested: Vec<String>,
    pub asked_for: String,
    pub not_asked_for: String,
}

#[uniffi::export]
pub fn sign_in_scopes(address: String) -> ScopesFfi {
    let domain = address
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_ascii_lowercase())
        .unwrap_or_default();
    let requested = postio_account::discovery::preset_for_domain(&domain)
        .and_then(|preset| preset.oauth().map(|oauth| oauth.scopes.clone()))
        .unwrap_or_default();
    ScopesFfi {
        requested,
        asked_for: "Postio asks for your mail: reading it, and sending as you.".to_owned(),
        not_asked_for: "It does not ask for your contacts, your calendar, or your files."
            .to_owned(),
    }
}

/// How a connection is secured, as discovery found it: flattening to a bool once turned a
/// provider's STARTTLS answer into a TLS dial (#534).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SecurityFfi {
    Tls,
    StartTls,
    /// Unencrypted -- what a provider's loopback answer can say.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerFfi {
    /// Hostname, **empty when nothing is known** -- see
    /// [`ProviderHintFfi::imap_host`].
    pub host: String,
    pub port: u16,
    pub security: SecurityFfi,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiscoveredFfi {
    /// Whether the answer is authoritative -- a provider's own document or
    /// the preset table -- rather than a guess the person should check.
    pub found: bool,
    /// The card's heading: `Found settings for tomlin.dev`, or what to do
    /// when nothing was.
    pub heading: String,
    pub imap: ServerFfi,
    pub smtp: ServerFfi,
    /// `imap.fastmail.com:993 · TLS`, the way the canvas writes it.
    pub imap_line: String,
    pub smtp_line: String,
    /// The login, which is not always the address.
    pub login: String,
    pub requires_app_password: bool,
    pub note: Option<String>,
    pub help_url: Option<String>,
    pub source: String,
    pub browser_sign_in: bool,
}

#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewAccountFfi {
    pub address: String,
    /// The name to send as. Empty means the address.
    pub name: String,
    /// On its way to the keyring, and nowhere else.
    pub password: String,
    pub login: String,
    pub imap: ServerFfi,
    pub smtp: ServerFfi,
}

/// Without the password: a record that prints one is a log that keeps it.
impl std::fmt::Debug for NewAccountFfi {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NewAccountFfi")
            .field("address", &self.address)
            .field("login", &self.login)
            .field("password", &"<withheld>")
            .field("imap", &self.imap)
            .field("smtp", &self.smtp)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncWindowFfi {
    /// About a month.
    LastMonth,
    /// A year, the field's own default.
    LastYear,
    /// No cap.
    Everything,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncWindowChoiceFfi {
    pub window: SyncWindowFfi,
    pub label: String,
    pub estimate: String,
    pub recommended: bool,
}

impl From<postio_ui::onboarding::SyncWindow> for SyncWindowFfi {
    fn from(window: postio_ui::onboarding::SyncWindow) -> Self {
        use postio_ui::onboarding::SyncWindow as W;
        match window {
            W::LastMonth => Self::LastMonth,
            W::LastYear => Self::LastYear,
            W::Everything => Self::Everything,
        }
    }
}

impl From<SyncWindowFfi> for postio_ui::onboarding::SyncWindow {
    fn from(window: SyncWindowFfi) -> Self {
        match window {
            SyncWindowFfi::LastMonth => Self::LastMonth,
            SyncWindowFfi::LastYear => Self::LastYear,
            SyncWindowFfi::Everything => Self::Everything,
        }
    }
}

#[uniffi::export]
pub fn sync_window_choices() -> Vec<SyncWindowChoiceFfi> {
    postio_ui::onboarding::SyncWindow::ALL
        .into_iter()
        .map(|window| SyncWindowChoiceFfi {
            window: window.into(),
            label: window.label().to_owned(),
            estimate: window.estimate(),
            recommended: window == postio_ui::onboarding::SyncWindow::default(),
        })
        .collect()
}

/// Write the chosen window to `[sync]` in the installed `config.toml`: `None` when written, a
/// sentence when not. A failure
/// costs the size picked, not the account.
#[uniffi::export]
pub fn write_initial_sync_window(window: SyncWindowFfi) -> Option<String> {
    postio_ui::onboarding::write_sync_window(window.into())
        .err()
        .map(|error| format!("Postio could not save how far back to sync: {error}"))
}

impl From<postio_model::TransportSecurity> for SecurityFfi {
    fn from(security: postio_model::TransportSecurity) -> Self {
        use postio_model::TransportSecurity as S;
        match security {
            S::Tls => Self::Tls,
            S::StartTls => Self::StartTls,
            S::None => Self::None,
        }
    }
}

impl From<SecurityFfi> for postio_model::TransportSecurity {
    fn from(security: SecurityFfi) -> Self {
        match security {
            SecurityFfi::Tls => Self::Tls,
            SecurityFfi::StartTls => Self::StartTls,
            SecurityFfi::None => Self::None,
        }
    }
}

impl From<&postio_ui::onboarding::Server> for ServerFfi {
    fn from(server: &postio_ui::onboarding::Server) -> Self {
        Self {
            host: server.host.clone(),
            port: server.port,
            security: server.security.into(),
        }
    }
}

impl From<&ServerFfi> for postio_ui::onboarding::Server {
    fn from(server: &ServerFfi) -> Self {
        Self {
            host: server.host.trim().to_owned(),
            port: server.port,
            security: server.security.into(),
        }
    }
}

/// `Found` is the only authoritative answer; a `Manual` one carries a suggestion to check, or
/// nothing, and then servers are empty rather than guessed.
pub(crate) fn discovered(address: &str, status: postio_ui::onboarding::Status) -> DiscoveredFfi {
    use postio_ui::onboarding::{Server, Settings, Status};
    let domain = postio_ui::onboarding::domain_of(address);
    let (found, settings) = match status {
        Status::Found(settings) => (true, settings),
        Status::Manual {
            suggestion: Some(settings),
        } => (false, settings),
        _ => (
            false,
            Settings {
                imap: Server {
                    port: 993,
                    ..Server::default()
                },
                smtp: Server {
                    port: 465,
                    ..Server::default()
                },
                ..Settings::default()
            },
        ),
    };
    let heading = if found {
        format!("Found settings for {domain}")
    } else if settings.imap.host.is_empty() {
        format!("Postio found no settings for {domain}. Enter the servers your provider gives.")
    } else {
        format!("Postio guessed the settings for {domain}. Check them before connecting.")
    };
    let line = |server: &Server| {
        if server.host.is_empty() {
            String::new()
        } else {
            server.line()
        }
    };
    DiscoveredFfi {
        found,
        heading,
        imap_line: line(&settings.imap),
        smtp_line: line(&settings.smtp),
        imap: (&settings.imap).into(),
        smtp: (&settings.smtp).into(),
        login: if settings.login.is_empty() {
            address.trim().to_owned()
        } else {
            settings.login
        },
        requires_app_password: settings.requires_app_password,
        note: settings.note,
        help_url: settings.help_url,
        source: settings.source,
        browser_sign_in: settings.oauth_sign_in,
    }
}

pub(crate) fn submission(account: NewAccountFfi) -> postio_ui::onboarding::Submission {
    let login = if account.login.trim().is_empty() {
        account.address.trim().to_owned()
    } else {
        account.login.trim().to_owned()
    };
    postio_ui::onboarding::Submission {
        address: account.address.trim().to_owned(),
        name: account.name.trim().to_owned(),
        password: account.password,
        settings: postio_ui::onboarding::Settings {
            imap: (&account.imap).into(),
            smtp: (&account.smtp).into(),
            login,
            source: "entered on this Mac".to_owned(),
            ..postio_ui::onboarding::Settings::default()
        },
        oauth_client: None,
    }
}

/// Whether there is enough of an address to look up
/// (`postio_ui::onboarding::looks_like_an_address`), so both first runs ask about the same strings.
#[uniffi::export]
pub fn looks_like_an_address(address: String) -> bool {
    postio_ui::onboarding::looks_like_an_address(&address)
}
