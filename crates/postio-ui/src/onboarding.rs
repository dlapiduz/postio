//! The first-run screen's model: what the servers are, what was submitted,
//! and which state the screen is in.
//!
//! Canvas 3e's shapes, with nothing a toolkit names. They lived in
//! `postio-gtk::onboarding` until the terminal needed the same screen: the
//! same states in the same order, the same sentences, the same sync-window
//! choices. The desktop re-exports them from here.

use postio_model::TransportSecurity;

/// Which of the three steps a status is in — `1 / 3`, the way the drawing
/// writes it.
///
/// Three rather than the two this screen used to count, and the split is
/// real rather than cosmetic: naming the account, proving you own it, and
/// saying how much of it to fetch are three different questions, and the
/// second one can fail and be retried without touching the other two.
/// Pure, so the mapping is tested without a display.
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

/// What a browser sign-in is asking for, so the screen can say so.
///
/// **Postio never draws a provider's login form.** Consent happens in the
/// real browser, against the real domain, where the address bar is the thing
/// a person checks — an in-app web view is how credential phishing is
/// normally taught, and there is no way for a user to tell one from the
/// genuine article. So what this screen can offer instead is an honest
/// account of what is happening while the browser is open, which is what
/// this carries (ADR 0006 Q3, `Design/screens/23`).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrowserSignIn {
    /// Whose consent screen the browser was sent to — `Microsoft`, `Google`.
    pub provider: String,
    /// The scopes the request asks for, in the provider's own spelling.
    /// Rendered through [`plain_scope`], never raw, because a URL is not an
    /// answer to "what is this about to be allowed to do".
    pub scopes: Vec<String>,
    /// Where the browser will be sent back to — `http://127.0.0.1:41337/`.
    pub redirect_uri: String,
    /// The consent URL itself, for `Copy URL` and for opening it again when
    /// the browser swallowed the first one.
    pub authorize_url: String,
}

/// What a scope lets Postio do, in words rather than in a URL.
///
/// A person deciding whether to consent is owed a sentence, not
/// `https://outlook.office.com/IMAP.AccessAsUser.All`. Anything unrecognised
/// falls through verbatim rather than being dropped: an unfamiliar scope is
/// exactly the one worth showing, and silently hiding it would make this
/// list a worse lie than no list at all.
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

/// One server, as the screen shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Server {
    /// Hostname.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Connection security. Carried losslessly from discovery (#534):
    /// flattening this to a bool once turned a provider's own
    /// plaintext-on-loopback answer into a TLS dial.
    pub security: TransportSecurity,
}

impl Server {
    /// `imap.fastmail.com:993 · TLS`, the way the canvas writes it.
    pub fn line(&self) -> String {
        let security = match self.security {
            TransportSecurity::Tls => "TLS",
            TransportSecurity::StartTls => "STARTTLS",
            TransportSecurity::None => "unencrypted",
        };
        format!("{}:{} · {security}", self.host, self.port)
    }
}

/// What Postio found, or what the user typed in instead.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Where mail is read from.
    pub imap: Server,
    /// Where mail is sent through.
    pub smtp: Server,
    /// The login name, which is not always the address — an iCloud custom
    /// domain logs in as the Apple ID.
    pub login: String,
    /// Whether this provider refuses ordinary account passwords.
    pub requires_app_password: bool,
    /// A sentence to show the user, from the provider table.
    pub note: Option<String>,
    /// Where to go and make an app-specific password.
    pub help_url: Option<String>,
    /// Where the settings came from, for the card's heading.
    pub source: String,
    /// Whether the provider prefers a browser sign-in (#534): the wizard
    /// shows the OAuth client fields and `Sign in with your browser`
    /// instead of the password entry. The app side holds the endpoints;
    /// this widget only needs to know which door to draw.
    pub oauth_sign_in: bool,
}

/// Everything the composition root needs to create the account.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Submission {
    /// The address mail arrives at.
    pub address: String,
    /// What to show instead of the bare address — in the `From` header and
    /// the sidebar. Empty means unset, and the composition root falls back
    /// to the address exactly as it did before this field existed.
    pub name: String,
    /// The password, on its way to the keyring and nowhere else. Empty on
    /// an OAuth submission.
    pub password: String,
    /// The servers to use.
    pub settings: Settings,
    /// The OAuth client the user supplied, when the provider's door is the
    /// browser sign-in (#534). `Some` routes the submission through the
    /// authorization flow instead of a password test.
    pub oauth_client: Option<OAuthClientSubmission>,
}

/// The user's own OAuth client (ADR 0006 Q1, `own-client`): what the
/// sign-in flow presents to the provider. Postio ships no client of its
/// own until #195 clears review, so these come from the user's provider
/// console.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OAuthClientSubmission {
    /// The client id, public by definition on a native app.
    pub client_id: String,
    /// The client secret, when the provider issued one — on its way to the
    /// keyring and nowhere else.
    pub client_secret: Option<String>,
}

/// Where the screen is in the one step it has.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Status {
    /// Nothing typed yet.
    #[default]
    Idle,
    /// The probe is out.
    Probing,
    /// The probe answered with something authoritative.
    Found(Settings),
    /// The probe found nothing. The server fields open, because an empty form
    /// the user can fill in is the way forward and a shrug is not.
    Manual {
        /// An unverified guess to prefill with, if there was one.
        suggestion: Option<Settings>,
    },
    /// Testing the credentials against the real server.
    Connecting,
    /// The consent screen is open in the user's browser; Postio is waiting
    /// for the redirect. Cancellable — the screen shows its own Cancel and
    /// `Esc` means the same thing.
    WaitingForBrowser,
    /// It did not work, and this says why in words the user can act on.
    Failed(String),
    /// The account is configured; its password is not.
    ///
    /// Not a first run. The composition root reaches this when the store
    /// holds an account the keyring will not give up a password for — a
    /// credential write that failed, a keyring that was reset, an item
    /// somebody deleted. The address and the servers are already known, so
    /// the screen arrives filled in and asks for the one thing missing.
    ///
    /// It carries the servers rather than reading them back off the form
    /// because the form is empty until something fills it, and the thing
    /// that knows them is the account row.
    Reauthenticate(Settings),
    /// The account is saved; the last question before Postio starts talking
    /// to the server on its own is how far back the first sync reaches.
    SyncWindow,
    /// The account exists and the password is in the keyring.
    Saved,
}

impl Status {
    /// Whether the screen is waiting on something and should not be touched.
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Status::Probing | Status::Connecting | Status::WaitingForBrowser
        )
    }

    /// The sentence under the form, when this state owes the user one.
    ///
    /// Pure, and public, so what the screen *says* can be checked without a
    /// display — the rendering needs one, the wording does not.
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

/// How far back the first sync reaches, chosen once per account on the
/// [`Status::SyncWindow`] step (#876).
///
/// Coarser than [`postio_config::sync::SyncConfig::initial_sync_messages`]
/// itself — a person thinks in a window of time, not a message count — so
/// each variant maps to a fixed count rather than to anything measured: no
/// per-account mailbox statistics exist at this point in onboarding
/// (discovery does not report message counts). `LastYear`'s count matches
/// that field's own default, so picking it changes nothing a fresh install
/// would not already do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SyncWindow {
    /// Roughly a month of ordinary mail.
    LastMonth,
    /// A year — [`SyncConfig`](postio_config::sync::SyncConfig)'s own
    /// default depth.
    #[default]
    LastYear,
    /// No cap: the highest count the field can hold.
    Everything,
}

impl SyncWindow {
    /// Every choice, in the order the picker offers them.
    pub const ALL: [SyncWindow; 3] = [
        SyncWindow::LastMonth,
        SyncWindow::LastYear,
        SyncWindow::Everything,
    ];

    /// What this writes to `SyncConfig::initial_sync_messages`.
    pub fn message_count(self) -> u32 {
        match self {
            SyncWindow::LastMonth => 500,
            SyncWindow::LastYear => 5_000,
            SyncWindow::Everything => u32::MAX,
        }
    }

    /// The picker's own label for this choice.
    pub fn label(self) -> &'static str {
        match self {
            SyncWindow::LastMonth => "Last 30 days",
            SyncWindow::LastYear => "Last year",
            SyncWindow::Everything => "Everything",
        }
    }

    /// A rough size/time readout under the picker.
    ///
    /// Built from a flat per-message estimate (75 KiB — ADR 0017 puts most
    /// of a message's bytes on the lazy attachment axis, so a synced-but-
    /// unopened message is mostly headers and text) and a flat fetch rate,
    /// for the same reason [`message_count`](Self::message_count) is a flat
    /// map rather than a measurement: nothing has synced yet to measure.
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

/// The domain of an address, for the card's heading.
pub fn domain_of(address: &str) -> String {
    address
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether there is enough of an address to be worth probing.
///
/// Deliberately loose: this decides whether to *ask*, and the probe itself
/// decides whether the address is real. Refusing to look up something a
/// server would have accepted is the worse mistake.
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

/// Writes the chosen sync window (#876) to `[sync].initial_sync_messages`,
/// touching only that key — the same [`postio_config::patch_sync`] every
/// other structured write to `[sync]` goes through (#874), so a hand-written
/// comment elsewhere in the file survives.
///
/// A write that fails is logged and otherwise swallowed: the account and its
/// credential are already saved by the time this runs, and the field's own
/// default (5,000, [`SyncWindow::LastYear`](SyncWindow::LastYear)'s
/// own count) is exactly what a fresh install already has, so a failed
/// write here costs the size the user picked, not the account.
pub fn write_sync_window(window: SyncWindow) -> postio_config::Result<()> {
    let path = postio_config::paths::config_path()?;
    let original = std::fs::read_to_string(&path).unwrap_or_default();
    let mut config = postio_config::Config::from_toml_str(&original).unwrap_or_default();
    config.sync.initial_sync_messages = window.message_count();
    let patched = postio_config::patch_sync(&original, &config.sync)?;
    postio_config::Config::write_text_to_path(&patched, &path)
}

/// Without the password: a submission crosses to the daemon over its
/// socket, and anything that prints one must not print that.
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

/// Without the client secret, for the same reason.
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
