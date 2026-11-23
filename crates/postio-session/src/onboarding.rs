//! Adding an account: the probe's reading, the proof, and the two writes; the half of the first-run
//! screen that names no toolkit, shared with the terminal.

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

/// How this application probes. `guess_common_names` is on here, though `postio-account` ships it
/// off, because `status_for` can only present a guess as `Status::Manual`, a starting point not a
/// claim.
pub fn probe_options() -> ProbeOptions {
    ProbeOptions {
        guess_common_names: true,
        ..ProbeOptions::default()
    }
}

pub fn status_for(report: &DiscoveryReport) -> Status {
    let found = report.settings().map(shown);
    match (&report.outcome, found) {
        (DiscoveryOutcome::Discovered(_), Some(settings)) => Status::Found(settings),
        (_, suggestion) => Status::Manual { suggestion },
    }
}

/// Both writes, in the order that cannot strand an account: the credential first, then the row,
/// since a failed keyring write after the row left an account that could not sync or be repaired
/// (`postio-67`). A secret with no account is rolled back and harmless.
///
/// Must be polled on the engine runtime, not the GTK main context: the keyring is reached over
/// D-Bus with `tokio::time::timeout`, which panics without a reactor (`postio-66`).
pub async fn persist(
    database: &Store,
    secrets: &dyn SecretStore,
    submission: &Submission,
    backend: postio_model::account::Backend,
) -> Result<(), String> {
    let key = AccountKey::new(submission.address.clone());
    let password = Password::new(submission.password.clone());
    // Reported, not swallowed: a silent failure would read as a Postio bug rather than a locked
    // keyring.
    secrets.store(&key, &password).await.map_err(|error| {
        format!(
            "The password could not be stored in the keyring: {error}. \
             Is the keyring unlocked?"
        )
    })?;

    if let Err(reason) = save(database, submission, backend).await {
        if let Err(error) = secrets.delete(&key).await {
            tracing::warn!(%error, "the rolled-back credential could not be removed");
        }
        return Err(reason);
    }
    Ok(())
}

/// Write the account row, creating it or repairing the one already there. An existing row is
/// updated and its identities left as they are, since a second row for one address would leave
/// `first_account` choosing between two (`postio-67`).
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

fn configure(account: &mut Account, submission: &Submission) {
    account.incoming.host = submission.settings.imap.host.clone();
    account.incoming.port = submission.settings.imap.port;
    account.incoming.security = submission.settings.imap.security;
    account.incoming.username = submission.settings.login.clone();
    account.outgoing.host = submission.settings.smtp.host.clone();
    account.outgoing.port = submission.settings.smtp.port;
    account.outgoing.security = submission.settings.smtp.security;
    account.outgoing.username = submission.settings.login.clone();
    // `persist_oauth` writes an OAuth submission's auth and client; a password submission resets
    // both so no stale client remains.
    if submission.oauth_client.is_none() {
        account.auth = AuthMethod::Password;
        account.oauth = None;
    }
    account.enabled = true;
}

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
        oauth_sign_in: settings.oauth.is_some(),
        // #1115: a `Builtin` preset names itself by display name; autoconfig and ISPDB documents
        // keep naming the source (#877).
        source: match settings.source {
            SettingsSource::Builtin => settings
                .display_name
                .clone()
                .unwrap_or_else(|| settings.source.label().to_owned()),
            _ => settings.source.label().to_owned(),
        },
    }
}

pub fn connection_settings(submission: &Submission) -> ConnectionSettings {
    ConnectionSettings::new(
        submission.settings.imap.host.clone(),
        submission.settings.imap.port,
        submission.settings.imap.security,
        submission.settings.login.clone(),
    )
}

/// Turn a backend error into something the user can act on. A provider that refuses ordinary
/// account passwords says only "rejected", which a typo looks the same as. No variant carries a
/// password.
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

/// Prove `submission`'s credentials and answer which backend to store: JMAP when offered and it
/// signs in, else IMAP (#545). Network work: run it on the runtime.
pub async fn prove(
    submission: &Submission,
    jmap: Option<&postio_account::discovery::JmapOffer>,
) -> Result<postio_model::account::Backend, String> {
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

pub use postio_ui::onboarding::configured;

/// The signed-in account's settings as `signin` takes them. The browser sign-in itself is
/// `crate::signin`'s, the one consent path.
pub fn account_settings(
    submission: &Submission,
    offer: &postio_account::discovery::OAuthOffer,
) -> AccountSettings {
    let server = |server: &Server| {
        postio_account::discovery::ServerSettings::new(
            server.host.clone(),
            server.port,
            match server.security {
                TransportSecurity::Tls => Encryption::Tls,
                TransportSecurity::StartTls => Encryption::StartTls,
                TransportSecurity::None => Encryption::None,
            },
        )
    };
    AccountSettings {
        email: submission.address.clone(),
        imap: server(&submission.settings.imap),
        smtp: server(&submission.settings.smtp),
        login: submission.settings.login.clone(),
        source: SettingsSource::Builtin,
        requires_app_password: false,
        note: None,
        password_help_url: None,
        display_name: None,
        oauth: Some(offer.clone()),
        jmap: None,
        backends: Vec::new(),
    }
}

/// Whose consent screen this is, from the IMAP host: the preset table has no provider name, and
/// mapping domain to brand would make it code.
pub fn provider_name(settings: &Settings) -> String {
    settings.source.clone()
}
