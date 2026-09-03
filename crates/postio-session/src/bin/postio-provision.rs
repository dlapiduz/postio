//! Add the first account, so a frontend with no onboarding screen has mail.
//!
//! macOS has no way to configure an account: onboarding is GTK top to bottom,
//! and a Mac has no Linux Postio to have set things up with (#649). This is
//! the smallest thing that closes that gap, and it makes exactly the two
//! writes the onboarding screen makes — see
//! [`postio_session::provision`](../provision/index.html) for the order they
//! go in and why that order is not negotiable.
//!
//! It builds on both platforms because `postio-session` does. The
//! `postio-app` example this replaces did not: ADR 0019 measured the one
//! boundary in the workspace, and it falls on `postio-gtk` and `postio-app`,
//! which is to say on the only crates a Mac cannot compile.
//!
//! # Use
//!
//! ```sh
//! export POSTIO_ADDRESS='you@your-provider.example'
//! read -rs POSTIO_APP_PASSWORD && export POSTIO_APP_PASSWORD
//! cargo run -p postio-session --bin postio-provision
//! ```
//!
//! The domain has to be one the provider preset table recognises; when it is
//! not, this names the variables to set instead. A reserved domain in the
//! usage above rather than a real provider because
//! `scripts/checks/check-no-personal-data.py` holds every address in this
//! repository to RFC 2606, usage strings included.
//!
//! **The password comes from the environment**, never from a file and never
//! from argv — argv is visible to every process on the machine through `ps`,
//! and a password in a file is the one thing #649's acceptance rules out. It
//! is not printed, not logged, and not written to the store: it goes to the
//! OS keyring and nowhere else.
//!
//! On Linux `scripts/run-isolated.sh HEAD --provision` runs this against a
//! throwaway store, because that script points `XDG_DATA_HOME` at a scratch
//! directory first. Run it outside that script and it writes the account you
//! actually use.
//!
//! # iCloud
//!
//! An Apple ID password will not work: iCloud requires an **app-specific
//! password**, created at appleid.apple.com under Sign-In and Security, with
//! two-factor authentication enabled. Revoke it there when you are done
//! testing — that takes effect immediately and costs nothing.

use std::process::ExitCode;

use postio_imap::secret::{Password, platform_keyring};
use postio_session::provision::{Provisioned, account_from, provision, settings_for};

/// A set variable, treating empty as unset — an exported-but-blank variable
/// is a mistake, not an answer.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn main() -> ExitCode {
    let Some(address) = env("POSTIO_ADDRESS") else {
        eprintln!("set POSTIO_ADDRESS to the address to add, e.g. you@your-provider.example");
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

    let settings = match settings_for(&address) {
        Ok(settings) => settings,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    // Everything but the password, so a mistyped host is visible before a
    // keyring prompt rather than after a failed sync.
    let store_path = postio_session::paths::store_path();
    println!("store:   {}", store_path.display());
    println!("address: {address}");
    println!("imap:    {}", settings.imap);
    println!("smtp:    {}", settings.smtp);
    println!("login:   {}", settings.login);

    // The store is encrypted under the keyring's key (ADR 0014), so adding an
    // account needs it exactly as the application does. `platform_keyring`
    // rather than a named store: this is the Keychain on macOS and the Secret
    // Service on freedesktop, and choosing here would be one more place to
    // get it wrong.
    let secrets = platform_keyring();
    let store_key = match postio_session::store_key_blocking(secrets.as_ref()) {
        Ok(key) => key,
        Err(error) => {
            eprintln!("postio: cannot read the store key (is the keyring unlocked?): {error}");
            return ExitCode::FAILURE;
        }
    };
    // `open_store`, not `Database::open`: it is what runs ADR 0014 Q4's
    // plaintext-to-encrypted migration, so this opens the same store the
    // application would rather than failing on one it has not converted yet.
    let (database, _blobs) = match postio_session::open_store(&store_key) {
        Ok(opened) => opened,
        Err(message) => {
            eprintln!("postio: {message}");
            return ExitCode::FAILURE;
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("postio: cannot start a runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = runtime.block_on(provision(
        &database,
        secrets.as_ref(),
        account_from(&settings),
        Password::new(password),
    ));

    match outcome {
        Ok(Provisioned::Created(id)) => {
            println!("\naccount {id:?} created, credential stored in the keyring.");
            println!("Run the app and it will sync on open.");
            ExitCode::SUCCESS
        }
        Ok(Provisioned::AlreadyProvisioned(id)) => {
            println!("\naccount {id:?} is already provisioned; leaving it alone.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("postio: {error}");
            ExitCode::FAILURE
        }
    }
}
