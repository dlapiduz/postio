//! The first-run screen's model: servers, what was submitted, and which state the screen is in,
//! shared by the desktop and the terminal.

use postio_model::TransportSecurity;

pub fn step_of(status: &Status) -> &'static str {
    match status {
        Status::Idle | Status::Probing | Status::Found(_) | Status::Manual { .. } => "1 / 3",
        Status::Connecting
        | Status::WaitingForBrowser
        | Status::Failed(_)
        | Status::Reauthenticate(_) => "2 / 3",
        Status::SyncWindow | Status::Saved => "3 / 3",
    }
}

/// Postio never draws a provider's login form: consent happens in the real browser, where the
/// address bar is checkable.
/// This is what the screen says while the browser is open (ADR 0006 Q3).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrowserSignIn {
    pub provider: String,
    pub scopes: Vec<String>,
    pub redirect_uri: String,
    pub authorize_url: String,
}

/// What a scope lets Postio do, in words. An unrecognised scope falls through verbatim: the
/// unfamiliar one is the one worth showing.
pub fn plain_scope(scope: &str) -> String {
    let folded = scope.to_ascii_lowercase();
    if folded.contains("imap") || folded == "https://mail.google.com/" {
        return "Read and change your mail".to_owned();
    }
    if folded.contains("smtp") || folded.contains("gmail.send") {
        return "Send mail as you".to_owned();
    }
    match folded.as_str() {
        "offline_access" => "Stay signed in without asking again".to_owned(),
        "openid" | "email" | "profile" => "Know which address you signed in as".to_owned(),
        _ => scope.to_owned(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    /// Carried losslessly from discovery (#534): flattening to a bool once turned a provider's
    /// plaintext-on-loopback answer into a TLS dial.
    pub security: TransportSecurity,
}

impl Server {
    pub fn line(&self) -> String {
        let security = match self.security {
            TransportSecurity::Tls => "TLS",
            TransportSecurity::StartTls => "STARTTLS",
            TransportSecurity::None => "unencrypted",
        };
        format!("{}:{} · {security}", self.host, self.port)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub imap: Server,
    pub smtp: Server,
    /// The login name, not always the address (an iCloud custom domain logs in as the Apple ID).
    pub login: String,
    pub requires_app_password: bool,
    pub note: Option<String>,
    pub help_url: Option<String>,
    pub source: String,
    /// Whether the provider prefers a browser sign-in (#534): the wizard shows the OAuth client
    /// fields instead of the password entry.
    pub oauth_sign_in: bool,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Submission {
    pub address: String,
    /// Empty means unset; the composition root falls back to the address.
    pub name: String,
    /// On its way to the keyring and nowhere else. Empty on an OAuth submission.
    pub password: String,
    pub settings: Settings,
    /// `Some` routes the submission through the authorization flow instead of a password test
    /// (#534).
    pub oauth_client: Option<OAuthClientSubmission>,
}

/// The user's own OAuth client (ADR 0006 Q1): Postio ships none until #195 clears review.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OAuthClientSubmission {
    pub client_id: String,
    /// On its way to the keyring and nowhere else.
    pub client_secret: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Status {
    #[default]
    Idle,
    Probing,
    Found(Settings),
    /// The probe found nothing; the server fields open, since an empty form to fill in is the way
    /// forward.
    Manual {
        suggestion: Option<Settings>,
    },
    Connecting,
    /// The consent screen is open in the user's browser; cancellable, and `Esc` means Cancel.
    WaitingForBrowser,
    Failed(String),
    /// The keyring will not give up the account's password. The screen arrives filled in; it
    /// carries
    /// the servers because the form is empty until something fills it.
    Reauthenticate(Settings),
    SyncWindow,
    Saved,
}

impl Status {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Status::Probing | Status::Connecting | Status::WaitingForBrowser
        )
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Status::Failed(reason) => Some(reason),
            Status::Reauthenticate(_) => Some(
                "Postio has no password for this account. Sign in again and it \
                 will go back into the keyring.",
            ),
            Status::WaitingForBrowser => Some(
                "Finish signing in in your browser. Postio is waiting for the \
                 redirect — cancel any time.",
            ),
            _ => None,
        }
    }
}

/// Each variant maps to a fixed message count: no per-account statistics exist at this point in
/// onboarding. `LastYear`'s count matches `initial_sync_messages`'s default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SyncWindow {
    LastMonth,
    #[default]
    LastYear,
    Everything,
}

impl SyncWindow {
    pub const ALL: [SyncWindow; 3] = [
        SyncWindow::LastMonth,
        SyncWindow::LastYear,
        SyncWindow::Everything,
    ];

    pub fn message_count(self) -> u32 {
        match self {
            SyncWindow::LastMonth => 500,
            SyncWindow::LastYear => 5_000,
            SyncWindow::Everything => postio_config::sync::ALL_MESSAGES,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SyncWindow::LastMonth => "Last 30 days",
            SyncWindow::LastYear => "Last year",
            SyncWindow::Everything => "Everything",
        }
    }

    /// A flat per-message estimate (75 KiB; ADR 0017 puts most bytes on the lazy attachment axis)
    /// and a flat fetch rate, since nothing has synced yet to measure.
    pub fn estimate(self) -> String {
        const AVERAGE_MESSAGE_BYTES: u64 = 75 * 1024;
        const MESSAGES_PER_MINUTE: u64 = 120;
        if self == SyncWindow::Everything {
            return "Downloads everything the server has — size and time depend \
                     on the mailbox."
                .to_owned();
        }
        let count = u64::from(self.message_count());
        let megabytes = (count * AVERAGE_MESSAGE_BYTES) / (1024 * 1024);
        let minutes = count.div_ceil(MESSAGES_PER_MINUTE).max(1);
        format!(
            "About {megabytes} MB, {minutes} minute{} to sync",
            if minutes == 1 { "" } else { "s" }
        )
    }
}

pub fn domain_of(address: &str) -> String {
    address
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Deliberately loose: this decides whether to ask; the probe decides whether the address is real.
pub fn looks_like_an_address(address: &str) -> bool {
    let address = address.trim();
    match address.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        None => false,
    }
}

/// A silent disabled button is a dead end; the OAuth client ID is not on the form until the
/// provider is known to need it.
pub fn submit_blocker(
    address: &str,
    oauth: bool,
    credential_given: bool,
    servers_known: bool,
) -> Option<&'static str> {
    if !looks_like_an_address(address) {
        return None;
    }
    if !servers_known {
        return None;
    }
    match (oauth, credential_given) {
        (true, false) => Some("Enter your OAuth client ID to sign in with your browser."),
        (false, false) => Some("Enter your password to connect."),
        _ => None,
    }
}

/// Touches only that key, via `postio_config::patch_sync` (#874). A failed write is logged and
/// swallowed: the account is already saved and the default equals `LastYear`'s count.
pub fn write_sync_window(window: SyncWindow) -> postio_config::Result<()> {
    let path = postio_config::paths::config_path()?;
    let original = std::fs::read_to_string(&path).unwrap_or_default();
    let mut config = postio_config::Config::from_toml_str(&original).unwrap_or_default();
    config.sync.initial_sync_messages = window.message_count();
    let patched = postio_config::patch_sync(&original, &config.sync)?;
    postio_config::Config::write_text_to_path(&patched, &path)
}

/// The inverse of `configure`: a repair asks for a password, so it offers the settings the
/// account signed in with; `source` names where they came from, since "entered by hand" would be a
/// lie.
pub fn configured(account: &postio_model::Account) -> Settings {
    let server = |config: &postio_model::account::ServerConfig| Server {
        host: config.host.clone(),
        port: config.port,
        security: config.security,
    };
    Settings {
        imap: server(&account.incoming),
        smtp: server(&account.outgoing),
        login: account.incoming.username.clone(),
        requires_app_password: false,
        note: None,
        help_url: None,
        // An OAuth account's repair is a fresh browser sign-in, not a password prompt (#534).
        oauth_sign_in: account.oauth.is_some()
            || matches!(
                account.auth,
                postio_model::account::AuthMethod::OAuth2
                    | postio_model::account::AuthMethod::XOAuth2
            ),
        source: "saved with this account".to_owned(),
    }
}

/// Without the password: a submission is logged on the way.
impl std::fmt::Debug for Submission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Submission")
            .field("address", &self.address)
            .field("name", &self.name)
            .field("password", &"<withheld>")
            .field("settings", &self.settings)
            .field("oauth_client", &self.oauth_client)
            .finish()
    }
}

impl std::fmt::Debug for OAuthClientSubmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthClientSubmission")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<withheld>"),
            )
            .finish()
    }
}

#[cfg(test)]
mod blocker_tests {
    use super::submit_blocker;

    #[test]
    fn a_disabled_button_says_what_it_is_waiting_for() {
        let browser = submit_blocker("ada@example.com", true, false, true);
        assert!(browser.is_some_and(|said| said.contains("OAuth client ID")));
        let password = submit_blocker("ada@example.com", false, false, true);
        assert!(password.is_some_and(|said| said.contains("password")));
        assert_eq!(submit_blocker("ada@", true, false, true), None);
        assert_eq!(submit_blocker("ada@example.com", true, false, false), None);
        assert_eq!(submit_blocker("ada@example.com", true, true, true), None);
    }
}
