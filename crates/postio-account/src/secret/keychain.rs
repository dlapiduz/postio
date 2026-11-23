//! The macOS answer to `KeyringSecretStore`. ADR 0014 keeps the local store's encryption key in the
//! OS keyring with no plaintext fallback, so without this the mail does not open.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use security_framework::os::macos::passwords::find_generic_password;

use super::{AccountKey, Password, SecretError, SecretStore};

const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
const ERR_SEC_AUTH_FAILED: i32 = -25293;

/// Groups Postio's items in Keychain Access and keeps other applications' items for the same
/// account apart.
const SERVICE: &str = "dev.postio.Postio";

/// Holds a path, not an open `SecKeychain`, so the store stays `Send + Sync` without a lock.
#[derive(Clone, Debug, Default)]
pub struct KeychainSecretStore {
    /// Only tests set this: the login keychain would prompt and hang headless runs.
    keychain: Option<Scratch>,
}

#[derive(Clone, Debug)]
struct Scratch {
    path: PathBuf,
    password: String,
}

impl KeychainSecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// For tests. The password is kept so each call can unlock again: a keychain locks on reopen
    /// and a prompt would hang.
    pub fn at(path: &Path, password: &str) -> Result<Self, SecretError> {
        if !path.exists() {
            CreateOptions::new()
                .password(password)
                .create(path)
                .map_err(|error| SecretError::Backend {
                    account: SERVICE.to_owned(),
                    reason: format!("cannot create a keychain at {}: {error}", path.display()),
                })?;
        }
        Ok(Self {
            keychain: Some(Scratch {
                path: path.to_path_buf(),
                password: password.to_owned(),
            }),
        })
    }

    fn keychains(&self) -> Result<Option<Vec<SecKeychain>>, SecretError> {
        let Some(scratch) = &self.keychain else {
            return Ok(None);
        };
        let mut keychain =
            SecKeychain::open(&scratch.path).map_err(|error| SecretError::Backend {
                account: SERVICE.to_owned(),
                reason: format!("cannot open {}: {error}", scratch.path.display()),
            })?;
        keychain
            .unlock(Some(&scratch.password))
            .map_err(|error| SecretError::Backend {
                account: SERVICE.to_owned(),
                reason: format!("cannot unlock {}: {error}", scratch.path.display()),
            })?;
        Ok(Some(vec![keychain]))
    }
}

/// Not found is a first run, and only that mints a key: minting on a transient failure would
/// encrypt the next write under a key the existing store does not know. Locked must reach the
/// surface that asks the user to unlock, not onboarding (ADR 0014).
fn map_error(key: &AccountKey, error: &security_framework::base::Error) -> SecretError {
    match error.code() {
        ERR_SEC_ITEM_NOT_FOUND => SecretError::NotFound {
            account: key.account().to_owned(),
        },
        ERR_SEC_INTERACTION_NOT_ALLOWED | ERR_SEC_AUTH_FAILED => SecretError::Locked {
            keyring: "login".to_owned(),
            account: key.account().to_owned(),
        },
        _ => SecretError::Backend {
            account: key.account().to_owned(),
            reason: error.to_string(),
        },
    }
}

#[async_trait]
impl SecretStore for KeychainSecretStore {
    fn describe(&self) -> &'static str {
        "keychain"
    }

    async fn store(&self, key: &AccountKey, password: &Password) -> Result<(), SecretError> {
        let this = self.clone();
        let key = key.clone();
        let secret = password.expose().to_owned();
        blocking(move || {
            let keychains = this.keychains()?;
            let account = key.account().to_owned();
            // Replace, not add: two items for one account would answer with whichever is found
            // first.
            if let Ok((_, mut item)) =
                find_generic_password(keychains.as_deref(), SERVICE, &account)
            {
                return item
                    .set_password(secret.as_bytes())
                    .map_err(|error| map_error(&key, &error));
            }
            let keychain = match keychains {
                Some(mut found) => found.remove(0),
                None => SecKeychain::default().map_err(|error| map_error(&key, &error))?,
            };
            keychain
                .set_generic_password(SERVICE, &account, secret.as_bytes())
                .map_err(|error| map_error(&key, &error))
        })
        .await
    }

    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError> {
        let this = self.clone();
        let key = key.clone();
        blocking(move || {
            let keychains = this.keychains()?;
            let (found, _item) =
                find_generic_password(keychains.as_deref(), SERVICE, key.account())
                    .map_err(|error| map_error(&key, &error))?;
            let text = std::str::from_utf8(&found).map_err(|error| SecretError::Backend {
                account: key.account().to_owned(),
                reason: format!("the stored secret is not text: {error}"),
            })?;
            Ok(Password::new(text))
        })
        .await
    }

    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError> {
        let this = self.clone();
        let key = key.clone();
        blocking(move || {
            let keychains = this.keychains()?;
            match find_generic_password(keychains.as_deref(), SERVICE, key.account()) {
                Ok((_, item)) => {
                    item.delete();
                    Ok(())
                }
                // Removing an absent password succeeds, as the trait requires.
                Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
                Err(error) => Err(map_error(&key, &error)),
            }
        })
        .await
    }
}

/// `SecItem` is synchronous and can wait on a user prompt.
async fn blocking<T, F>(work: F) -> Result<T, SecretError>
where
    F: FnOnce() -> Result<T, SecretError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) => Err(SecretError::Backend {
            account: SERVICE.to_owned(),
            reason: format!("the keychain task did not finish: {error}"),
        }),
    }
}
