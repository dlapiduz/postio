//! Create the first account so the app has something to sync.
//!
//! `postio-hiy` is the screen that will do this properly. Until it lands,
//! `feed_the_window` finds no account and the application opens empty, so
//! there is no way to point Postio at a real mailbox. This is the smallest
//! thing that closes that gap — and it is the same two writes the onboarding
//! screen will make: an account row, and a credential in the keyring.
//!
//! # Use
//!
//! ```sh
//! export POSTIO_ADDRESS='you@icloud.com'
//! read -rs POSTIO_APP_PASSWORD && export POSTIO_APP_PASSWORD
//! scripts/run-isolated.sh HEAD --provision
//! ```
//!
//! The password is read from the environment, never from a file and never
//! from argv — argv is visible to every process on the machine via `ps`. It
//! is not printed, not logged, and not written to the store; it goes to the
//! Secret Service and nowhere else.
//!
//! `scripts/run-isolated.sh` points `XDG_DATA_HOME` at a throwaway directory,
//! so provisioning through it writes a scratch store rather than a real one.
//! Run this outside that script and it writes the account you actually use.
//!
//! # iCloud
//!
//! An Apple ID password will not work: iCloud requires an **app-specific
//! password**, created at appleid.apple.com under Sign-In and Security, with
//! two-factor authentication enabled. Revoke it there when you are done
//! testing — that takes effect immediately and costs nothing.

use std::process::ExitCode;

use postio_imap::secret::{AccountKey, KeyringSecretStore, Password, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress, Identity};
use postio_storage::Database;
use postio_storage::repository::AccountRepository;

/// Where the store lives.
///
/// Duplicated from `postio-app/src/paths.rs` because that crate is a binary
/// with no lib target, so an example cannot import it. Keep the two in step —
/// or give postio-app a lib target and delete this.
fn store_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".local").join("share"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("postio").join("postio.db")
}

/// Providers whose settings we know, so the common case needs no flags.
///
/// This is a convenience for provisioning, not the provider table: `spec.md`
/// §3 wants presets to be data rather than code, and the autoconfig probe
/// (`postio-pco`) is what discovers a server properly.
fn known(domain: &str) -> Option<(&'static str, u16, &'static str, u16)> {
    match domain {
        "icloud.com" | "me.com" | "mac.com" => {
            // 465 with implicit TLS, not 587/STARTTLS: verified against a
            // working iCloud client configuration.
            Some(("imap.mail.me.com", 993, "smtp.mail.me.com", 465))
        }
        "gmail.com" | "googlemail.com" => {
            Some(("imap.gmail.com", 993, "smtp.gmail.com", 465))
        }
        "fastmail.com" | "fastmail.fm" => {
            Some(("imap.fastmail.com", 993, "smtp.fastmail.com", 465))
        }
        _ => None,
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn main() -> ExitCode {
    let Some(address) = env("POSTIO_ADDRESS") else {
        eprintln!("set POSTIO_ADDRESS to the address to add, e.g. you@icloud.com");
        return ExitCode::FAILURE;
    };
    let Some(password) = env("POSTIO_APP_PASSWORD") else {
        eprintln!(
            "set POSTIO_APP_PASSWORD to an app-specific password.\n\
             Read it without putting it in your shell history:\n\
             \x20   read -rs POSTIO_APP_PASSWORD && export POSTIO_APP_PASSWORD"
        );
        return ExitCode::FAILURE;
    };

    let Some(domain) = address.rsplit('@').next().map(str::to_ascii_lowercase) else {
        eprintln!("postio: {address} does not look like an address");
        return ExitCode::FAILURE;
    };

    let (imap_host, imap_port, smtp_host, smtp_port) = match known(&domain) {
        Some(settings) => settings,
        None => {
            eprintln!(
                "postio: no built-in settings for {domain}.\n\
                 This helper only knows a few providers; the autoconfig probe \
                 in postio-imap is what discovers the rest, and postio-hiy is \
                 the screen that will use it."
            );
            return ExitCode::FAILURE;
        }
    };

    let store_path = store_path();
    println!("store:   {}", store_path.display());
    println!("address: {address}");
    println!("imap:    {imap_host}:{imap_port} (implicit TLS)");
    println!("smtp:    {smtp_host}:{smtp_port} (implicit TLS)");

    if let Some(parent) = store_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("postio: cannot create {}: {error}", parent.display());
            return ExitCode::FAILURE;
        }
    }

    let database = match Database::open(&store_path) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("postio: cannot open the store: {error}");
            return ExitCode::FAILURE;
        }
    };
    let connection = match database.connection() {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("postio: cannot open a connection: {error}");
            return ExitCode::FAILURE;
        }
    };
    let accounts = AccountRepository::new(&connection);

    match accounts.list_enabled() {
        Ok(existing) if existing.iter().any(|a| a.address.address == address) => {
            println!("\nalready provisioned; leaving it alone.");
            return ExitCode::SUCCESS;
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("postio: cannot read the accounts: {error}");
            return ExitCode::FAILURE;
        }
    }

    let email = EmailAddress::new(None::<String>, address.clone());
    let mut account = Account::new(address.clone(), email.clone());
    account.incoming.host = imap_host.to_owned();
    account.incoming.port = imap_port;
    account.incoming.security = TransportSecurity::Tls;
    account.outgoing.host = smtp_host.to_owned();
    account.outgoing.port = smtp_port;
    account.outgoing.security = TransportSecurity::Tls;
    account.auth = AuthMethod::Password;
    let mut identity = Identity::new(AccountId::UNASSIGNED, email);
    identity.is_default = true;
    account.identities = vec![identity];

    let id = match accounts.create(&mut account) {
        Ok(id) => id,
        Err(error) => {
            eprintln!("postio: cannot write the account: {error}");
            return ExitCode::FAILURE;
        }
    };

    // The credential goes to the Secret Service and nowhere else. It is never
    // written to the store, never printed, and never logged.
    let secrets = KeyringSecretStore::default();
    let key = AccountKey::new(address.clone());
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("postio: cannot start a runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let stored = runtime.block_on(secrets.store(&key, &Password::new(password)));
    if let Err(error) = stored {
        eprintln!(
            "postio: the account was written but the password was not stored: {error}\n\
             Without it sync cannot authenticate. Is the keyring unlocked?"
        );
        return ExitCode::FAILURE;
    }

    println!("\naccount {id:?} created, credential stored in the keyring.");
    println!("Run the app and it will sync on open.");
    println!("\nWhen you are done testing, revoke the app-specific password at");
    println!("appleid.apple.com -> Sign-In and Security -> App-Specific Passwords.");
    ExitCode::SUCCESS
}
