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

/// What a sign-in is doing, for the sheet to draw while somebody is away in
/// their browser.
///
/// The port matters more than it looks. A person who has just been sent to
/// Safari is being asked to trust that the thing waiting for them is Postio;
/// naming the loopback port it is listening on is the only evidence a mail
/// client can offer, and it costs nothing to say.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SignInProgressFfi {
    /// Whether a flow is running at all.
    pub waiting: bool,
    /// The loopback port the answer comes back on, once it is listening.
    pub port: u16,
    /// What is happening, in a sentence.
    pub message: String,
}

/// The scopes a sign-in will ask for, and — as plainly — the ones it will
/// not.
///
/// "State the scopes requested in plain words, and state what is not
/// requested (contacts, calendar, files)." A consent screen lists what an
/// application *can* do; only the application can say what it deliberately
/// left out, and saying so is the difference between asking permission and
/// asking forgiveness.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScopesFfi {
    /// The scopes, as the provider names them.
    pub requested: Vec<String>,
    /// What Postio asks for, in words.
    pub asked_for: String,
    /// What it does not ask for, in the same breath.
    pub not_asked_for: String,
}

/// What signing in to `address` would request.
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
        // Named rather than implied. A provider's consent screen says what
        // an application may do; only Postio can say what it chose not to.
        not_asked_for: "It does not ask for your contacts, your calendar, or your files."
            .to_owned(),
    }
}

// -- the first-run wizard (canvas 09) ---------------------------------------

/// How a connection to one server is secured. Carried as discovery found
/// it rather than flattened to a bool: that flattening once turned a
/// provider's own STARTTLS answer into a TLS dial (#534).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SecurityFfi {
    /// TLS from the first byte.
    Tls,
    /// Plain, upgraded with `STARTTLS`.
    StartTls,
    /// Unencrypted -- what a provider's loopback answer can say.
    None,
}

/// One server: where it is and how it is reached.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerFfi {
    /// Hostname, **empty when nothing is known** -- see
    /// [`ProviderHintFfi::imap_host`].
    pub host: String,
    /// Port.
    pub port: u16,
    /// Connection security.
    pub security: SecurityFfi,
}

/// What looking an address up found, for the "Found settings" card.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiscoveredFfi {
    /// Whether the answer is authoritative -- a provider's own document or
    /// the preset table -- rather than a guess the person should check.
    pub found: bool,
    /// The card's heading: `Found settings for tomlin.dev`, or what to do
    /// when nothing was.
    pub heading: String,
    /// Where mail is read from.
    pub imap: ServerFfi,
    /// Where mail is sent through.
    pub smtp: ServerFfi,
    /// `imap.fastmail.com:993 · TLS`, the way the canvas writes it.
    pub imap_line: String,
    /// The same for `smtp`.
    pub smtp_line: String,
    /// The login, which is not always the address.
    pub login: String,
    /// Whether the provider refuses the account's own password.
    pub requires_app_password: bool,
    /// A sentence from the provider table, if it has one.
    pub note: Option<String>,
    /// Where to make an app password.
    pub help_url: Option<String>,
    /// Where the settings came from.
    pub source: String,
    /// Whether the provider's door is the browser rather than a password.
    pub browser_sign_in: bool,
}

/// What `Connect` submits.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewAccountFfi {
    /// The address mail arrives at.
    pub address: String,
    /// The name to send as. Empty means the address.
    pub name: String,
    /// On its way to the keyring, and nowhere else.
    pub password: String,
    /// What to sign in as.
    pub login: String,
    /// Where mail is read from.
    pub imap: ServerFfi,
    /// Where mail is sent through.
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

/// How far back the first sync reaches (#876).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncWindowFfi {
    /// About a month.
    LastMonth,
    /// A year, the field's own default.
    LastYear,
    /// No cap.
    Everything,
}

/// One choice in the sync-window step.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncWindowChoiceFfi {
    /// Which one.
    pub window: SyncWindowFfi,
    /// The picker's label.
    pub label: String,
    /// The rough size and time under it.
    pub estimate: String,
    /// Whether it is the default.
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

/// The sync-window step's choices, in the order the desktop offers them.
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

/// Write the chosen window to `[sync]` in the installed `config.toml`.
/// `None` when it was written, a sentence when it was not.
///
/// A failure costs the size picked, not the account: the account is saved
/// before this step is shown, and the field's default is `LastYear`'s.
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

/// The card for `address`, from what the shared onboarding answered.
///
/// `Found` is the only authoritative answer; a `Manual` one carries a
/// suggestion to check, or nothing, and then the servers are empty rather
/// than guessed and the ports are the implicit-TLS ones.
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

/// What the shared onboarding is handed for `account`.
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

/// Whether there is enough of an address to be worth looking up -- the
/// desktop's rule (`postio_ui::onboarding::looks_like_an_address`), so the
/// two first runs ask the network about the same strings.
#[uniffi::export]
pub fn looks_like_an_address(address: String) -> bool {
    postio_ui::onboarding::looks_like_an_address(&address)
}
