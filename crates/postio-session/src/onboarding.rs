//! Adding an account: the probe's reading, the proof, and the two writes.
//!
//! The first-run screen's half that talks to servers and the store, with
//! nothing a toolkit names. It was `postio-app`'s until the terminal needed
//! the same first run (specs/005-tui-frontend T016): the same probe options,
//! the same proof in the same order, the same sentences for the same
//! failures, the same credential-first write. The desktop calls it; the
//! daemon answers the terminal's onboarding requests with it.

use postio_account::discovery::{
    AccountSettings, DiscoveryOutcome, DiscoveryReport, Encryption, ProbeOptions, SettingsSource,
};
use postio_account::imap::{ConnectionSettings, ImapSession, RustlsConnector};
use postio_account::secret::{AccountKey, Password, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress, Identity};
use postio_storage::Store;
use postio_storage::repository::AccountRepository;
use postio_ui::onboarding::{Server, Settings, Status, Submission};

/// How this application probes, as against how the crate probes by default.
///
/// The one difference is `guess_common_names`, which `postio-account` ships off
/// because "an unverified guess presented as a *discovery* is worse than an
/// empty form" — and it is right about that. The composition root turns it on
/// because it controls what the guess is presented *as*: [`status_for`] can
/// only ever put it in [`Status::Manual`], the state whose heading says no
/// settings were published and whose form is open for editing. That is a
/// starting point, not a claim.
///
/// `postio-69`: without it, a domain publishing no autoconfig — every custom
/// domain, which is exactly the person least able to answer — got five empty
/// boxes.
pub fn probe_options() -> ProbeOptions {
    ProbeOptions {
        guess_common_names: true,
        ..ProbeOptions::default()
    }
}

/// What the screen should show for `report`.
///
/// Split out of [`probe`] so it can be driven without a network: the mapping
/// is where a discovery becomes a sentence, and it is the half that had the
/// bug.
pub fn status_for(report: &DiscoveryReport) -> Status {
    let found = report.settings().map(shown);
    match (&report.outcome, found) {
        (DiscoveryOutcome::Discovered(_), Some(settings)) => Status::Found(settings),
        // Everything else is manual entry, prefilled when there was anything
        // to prefill with. Never `Found`: see `probe_options`.
        (_, suggestion) => Status::Manual { suggestion },
    }
}

/// Both writes, in the order that cannot strand an account.
///
/// **The credential first, then the row.** 0.1.0 did it the other way round
/// and `postio-67` is what that cost: a keyring write that failed after the
/// row was committed left an account with no reachable password, which could
/// not sync, could not authenticate, and could not be repaired from inside
/// the application — onboarding is the only thing that writes a credential,
/// and `first_account().is_some()` meant onboarding never ran again.
///
/// The failure that order *does* leave behind — a secret with no account —
/// is rolled back here, and would be harmless even if the rollback failed:
/// nothing reads a credential no account row names.
///
/// **Must be polled on the engine runtime, not the GTK main context.** The
/// keyring is reached over D-Bus by a future bounded with
/// `tokio::time::timeout`, so awaiting it from `glib::spawn_future_local`
/// panics with "there is no reactor running" — `postio-66`, which shipped.
/// `feed.rs` states the rule: neither loop can drive the other, so runtime
/// work is spawned and answered over a channel.
pub async fn persist(
    database: &Store,
    secrets: &dyn SecretStore,
    submission: &Submission,
    backend: postio_model::account::Backend,
) -> Result<(), String> {
    let key = AccountKey::new(submission.address.clone());
    let password = Password::new(submission.password.clone());
    // Reported rather than swallowed: an account with no password in the
    // keyring cannot sync, and a silent failure would read as a Postio bug
    // rather than as a locked keyring.
    secrets.store(&key, &password).await.map_err(|error| {
        format!(
            "The password could not be stored in the keyring: {error}. \
             Is the keyring unlocked?"
        )
    })?;

    if let Err(reason) = save(database, submission, backend).await {
        if let Err(error) = secrets.delete(&key).await {
            // Safe to log: no `SecretError` carries a password.
            tracing::warn!(%error, "the rolled-back credential could not be removed");
        }
        return Err(reason);
    }
    Ok(())
}

/// Write the account row, creating it or repairing the one already there.
///
/// Synchronous, because rusqlite is; called from [`persist`] on the engine
/// runtime, where one indexed insert is not worth a `spawn_blocking`.
///
/// # Why this can be a repair
///
/// Since `postio-67` the screen is reachable a second time: an account whose
/// credential the keyring will not give up is sent back here rather than
/// opened. That submit arrives over a row that already exists, and a second
/// row would leave `first_account` choosing between two accounts for the
/// same address. So an existing row is *updated* — and its identities are
/// left exactly as they are, because [`AccountRepository::update`] makes the
/// list it is handed authoritative and every saved draft points at one.
pub async fn save(
    database: &Store,
    submission: &Submission,
    backend: postio_model::account::Backend,
) -> Result<(), String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| format!("Postio could not open its local store: {error}"))?;
    let repository = AccountRepository::new(&connection);
    let existing = repository
        .list()
        .await
        .map_err(|error| format!("Postio could not read its local store: {error}"))?
        .into_iter()
        .find(|account| {
            account
                .address
                .address
                .eq_ignore_ascii_case(&submission.address)
        });

    match existing {
        Some(mut account) => {
            configure(&mut account, submission);
            account.backend = backend;
            repository
                .update(&mut account)
                .await
                .map_err(|error| format!("Postio could not update the account: {error}"))
        }
        None => {
            let name =
                (!submission.name.trim().is_empty()).then(|| submission.name.trim().to_owned());
            let email = EmailAddress::new(name.clone(), submission.address.clone());
            let display_name = name.unwrap_or_else(|| submission.address.clone());
            let mut account = Account::new(display_name, email.clone());
            configure(&mut account, submission);
            account.backend = backend;
            let mut identity = Identity::new(AccountId::UNASSIGNED, email);
            identity.is_default = true;
            account.identities = vec![identity];
            repository
                .create(&mut account)
                .await
                .map(|_| ())
                .map_err(|error| format!("Postio could not write the account: {error}"))
        }
    }
}

/// Put the submitted servers on `account`, leaving its identities alone.
fn configure(account: &mut Account, submission: &Submission) {
    account.incoming.host = submission.settings.imap.host.clone();
    account.incoming.port = submission.settings.imap.port;
    account.incoming.security = submission.settings.imap.security;
    account.incoming.username = submission.settings.login.clone();
    account.outgoing.host = submission.settings.smtp.host.clone();
    account.outgoing.port = submission.settings.smtp.port;
    account.outgoing.security = submission.settings.smtp.security;
    account.outgoing.username = submission.settings.login.clone();
    // An OAuth submission's auth and client are written by `persist_oauth`,
    // which is the only caller holding the resolved endpoints; a password
    // submission resets both, so switching a repaired account from OAuth
    // back to a password leaves no stale client behind.
    if submission.oauth_client.is_none() {
        account.auth = AuthMethod::Password;
        account.oauth = None;
    }
    // A repair over an account somebody had disabled is still a repair: the
    // user just proved they want to sign in to it.
    account.enabled = true;
}

/// What the screen shows for an account the store already has.
///
/// The inverse of [`configure`]: a repair is asking for a password, not for
/// server settings, so the ones the account was signed in with last time are
/// what it offers. `source` names where they came from because the card
/// shows it, and "entered by hand" — what an empty form falls back to —
/// would be a lie the second time round.
pub fn configured(account: &Account) -> Settings {
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
        // A repair signs in the way the account did: an OAuth account's
        // repair is a fresh browser sign-in, not a password prompt for a
        // password that never existed (#534).
        oauth_sign_in: account.oauth.is_some()
            || matches!(
                account.auth,
                postio_model::account::AuthMethod::OAuth2
                    | postio_model::account::AuthMethod::XOAuth2
            ),
        source: "saved with this account".to_owned(),
    }
}

/// What the screen shows, from what the probe found.
pub fn shown(settings: &AccountSettings) -> Settings {
    let server = |server: &postio_account::discovery::ServerSettings| Server {
        host: server.host.clone(),
        port: server.port,
        security: match server.encryption {
            Encryption::Tls => TransportSecurity::Tls,
            Encryption::StartTls => TransportSecurity::StartTls,
            Encryption::None => TransportSecurity::None,
        },
    };
    Settings {
        imap: server(&settings.imap),
        smtp: server(&settings.smtp),
        login: settings.login.clone(),
        requires_app_password: settings.requires_app_password,
        note: settings.note.clone(),
        help_url: settings.password_help_url.clone(),
        // The provider's preferred door (#534): a preset row that leads
        // with oauth2 opens the browser sign-in.
        oauth_sign_in: settings.oauth.is_some(),
        // #1115: a preset row names itself -- its own display name says
        // more than the mechanism that found it ("known provider").
        // Gated on `Builtin` specifically rather than on `display_name`
        // being `Some`: an autoconfig or ISPDB document can carry its own
        // `<displayName>` too (`discovery::mod.rs`'s shared XML-shaped
        // builder), and #877 decided those keep naming the *source* --
        // the wizard has verified far less about a scraped document than
        // about a provider Postio ships settings for by hand.
        source: match settings.source {
            SettingsSource::Builtin => settings
                .display_name
                .clone()
                .unwrap_or_else(|| settings.source.label().to_owned()),
            _ => settings.source.label().to_owned(),
        },
    }
}

/// The IMAP connection to test.
pub fn connection_settings(submission: &Submission) -> ConnectionSettings {
    ConnectionSettings::new(
        submission.settings.imap.host.clone(),
        submission.settings.imap.port,
        submission.settings.imap.security,
        submission.settings.login.clone(),
    )
}

/// Turn a backend error into something the user can act on.
///
/// The acceptance criterion is that a failure gives a *specific, actionable*
/// reason, and `BackendError` already distinguishes the cases that need
/// different actions. What it cannot know is the one that matters most here:
/// a provider that refuses ordinary account passwords will simply say the
/// credentials were rejected, and a user who has typed their Apple ID
/// password has no way to tell that from a typo.
///
/// No variant of `BackendError` carries a password, so these are safe to show
/// and safe to log.
pub fn explain(error: &postio_account::backend::BackendError) -> String {
    use postio_account::backend::BackendError as E;
    match error {
        E::Auth { .. } => "The server rejected that address and password.\n\n\
             If this is iCloud, Google or another provider with two-factor \
             authentication, your ordinary account password will not work \
             here — you need an app-specific password."
            .to_owned(),
        E::Tls { host, reason } => format!(
            "The secure connection to {host} could not be established: {reason}.\n\n\
             Postio will not fall back to an unencrypted connection. Check the \
             host name and port."
        ),
        E::TimedOut { after, .. } => format!(
            "The server did not answer within {}s. Check the host name and \
             port, and whether this machine can reach the internet.",
            after.as_secs_f32().round()
        ),
        E::Disconnected { reason, .. } => format!(
            "The connection was lost while signing in: {reason}. That usually \
             means the wrong port, or a server that is not IMAP."
        ),
        E::EmptyCapabilities { host } => format!(
            "{host} answered, but not like an IMAP server. Check the host name \
             and port."
        ),
        other => format!("{other}"),
    }
}

/// Prove `submission`'s credentials against its server, and answer which
/// backend to store: JMAP when the probe offered it and it signs in, else
/// IMAP. The error is [`explain`]'s sentence, for the screen.
///
/// Network work: run it on the runtime, never on a thread that draws. The
/// proof tries backends in the row's preference order and the first that
/// works is the one stored (#545): a credential that only speaks IMAP still
/// lands on a provider advertising JMAP, and one that speaks JMAP gets the
/// native protocol.
pub async fn prove(
    submission: &Submission,
    jmap: Option<&postio_account::discovery::JmapOffer>,
) -> Result<postio_model::account::Backend, String> {
    // The password is used for one login and dropped here.
    let password = Password::new(submission.password.clone());
    if let Some(offer) = jmap
        && let Ok(url) = offer.session_url.parse()
    {
        let proof = postio_jmap::JmapBackend::new(url, password.expose());
        match postio_account::backend::MailBackend::connect(&proof).await {
            Ok(_) => {
                return Ok(postio_model::account::Backend::Jmap {
                    session_url: offer.session_url.clone(),
                });
            }
            Err(error) => {
                tracing::info!(%error, "the JMAP proof failed; trying the next backend");
            }
        }
    }
    let settings = connection_settings(submission);
    match RustlsConnector::new() {
        Ok(connector) => ImapSession::open(&settings, &password, &connector)
            .await
            .map(|_| postio_model::account::Backend::Imap)
            .map_err(|error| explain(&error)),
        Err(error) => Err(format!(
            "Postio could not start a TLS connection on this machine: {error}"
        )),
    }
}

pub use postio_ui::onboarding::write_sync_window;
