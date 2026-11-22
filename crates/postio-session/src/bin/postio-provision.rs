//! Add the first account, so a frontend with no onboarding screen has mail (#649); it makes the two
//! writes onboarding makes.
//! The password comes from `POSTIO_APP_PASSWORD`, never argv or a file, and goes only to the OS
//! keyring.
//! Outside `scripts/run-isolated.sh` it writes the account you actually use.

use std::process::ExitCode;

use postio_account::secret::{Password, platform_keyring};
use postio_session::provision::{Provisioned, account_from, provision, settings_for};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("postio: cannot start a runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run())
}

async fn run() -> ExitCode {
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

    // Everything but the password, so a mistyped host shows before a keyring prompt.
    let store_path = postio_session::paths::store_path();
    println!("store:   {}", store_path.display());
    println!("address: {address}");
    println!("imap:    {}", settings.imap);
    println!("smtp:    {}", settings.smtp);
    println!("login:   {}", settings.login);

    // The store is encrypted under the keyring's key (ADR 0014); `platform_keyring` because the
    // backend differs per platform.
    let secrets = platform_keyring();
    let store_key = match postio_session::store_key_blocking(secrets.as_ref()) {
        Ok(key) => key,
        Err(error) => {
            eprintln!("postio: cannot read the store key (is the keyring unlocked?): {error}");
            return ExitCode::FAILURE;
        }
    };
    // `open_store`, not `Store::open`: it also opens the blob store and builds the search index.
    let (database, _blobs) = match postio_session::open_store(&store_key).await {
        Ok(opened) => opened,
        Err(message) => {
            eprintln!("postio: {message}");
            return ExitCode::FAILURE;
        }
    };

    let outcome = provision(
        &database,
        secrets.as_ref(),
        account_from(&settings),
        Password::new(password),
    )
    .await;

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
