use std::path::{Path, PathBuf};

use postio_signature::{KeyInfo, KeyRead};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Keyboxd,
    Keybox,
    Pubring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keyring {
    Read {
        format: Format,
        keys: Vec<KeyInfo>,
        skipped: u32,
        opened: PathBuf,
    },
    Unreadable {
        opened: Option<PathBuf>,
    },
    Absent,
}

pub fn home_of_this_user() -> Option<PathBuf> {
    home(
        std::env::var_os("GNUPGHOME").as_deref(),
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
    )
}

pub fn home(gnupghome: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    match gnupghome.filter(|value| !value.is_empty()) {
        Some(value) => Some(PathBuf::from(value)),
        None => home.map(|home| home.join(".gnupg")),
    }
}

pub async fn read(home: &Path) -> Keyring {
    let keyboxd = home.join("public-keys.d").join("pubring.db");
    if keyboxd.is_file() {
        return match postio_storage::keyboxd::openpgp_keyblobs(&keyboxd).await {
            Ok(blobs) => {
                let mut keys = Vec::new();
                let mut skipped = 0;
                for blob in blobs {
                    match postio_signature::keys_in(&blob).into_iter().next() {
                        Some(key) => keys.push(key),
                        None => skipped += 1,
                    }
                }
                Keyring::Read {
                    format: Format::Keyboxd,
                    keys,
                    skipped,
                    opened: keyboxd,
                }
            }
            Err(_) => Keyring::Unreadable {
                opened: Some(keyboxd),
            },
        };
    }
    for (name, format, parse) in [
        (
            "pubring.kbx",
            Format::Keybox,
            postio_signature::keyblocks_from_keybox as fn(&[u8]) -> KeyRead,
        ),
        (
            "pubring.gpg",
            Format::Pubring,
            postio_signature::keyblocks_from_pubring_gpg,
        ),
    ] {
        let path = home.join(name);
        if !path.is_file() {
            continue;
        }
        let mut read = read_whole(&path).map(|bytes| parse(&bytes));
        if read.as_ref().is_some_and(|read| read.skipped > 0) {
            read = read_whole(&path).map(|bytes| parse(&bytes));
        }
        return match read {
            Some(read) if !read.blocks.is_empty() || read.skipped == 0 => Keyring::Read {
                format,
                keys: read.blocks,
                skipped: read.skipped,
                opened: path,
            },
            _ => Keyring::Unreadable { opened: Some(path) },
        };
    }
    Keyring::Absent
}

fn read_whole(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

pub async fn sync(
    database: &postio_storage::Store,
    blobs: &postio_storage::BlobStore,
    home: Option<&Path>,
    on: bool,
) -> Result<postio_ui::settings::GnupgKeyring, postio_storage::Error> {
    use postio_storage::repository::{KeyKind, KeyRepository, KeySource, NewKey};
    use postio_ui::settings::GnupgKeyring;
    let connection = database.connect().await?;
    let keys = KeyRepository::new(&connection);
    if !on {
        keys.drop_source(KeySource::Gnupg).await?;
        return Ok(GnupgKeyring::Off);
    }
    let Some(home) = home else {
        return Ok(GnupgKeyring::Absent {
            home: "~/.gnupg".to_owned(),
        });
    };
    let (found, skipped) = match read(home).await {
        Keyring::Read { keys, skipped, .. } => (keys, skipped),
        Keyring::Unreadable { .. } => return Ok(GnupgKeyring::Unreadable),
        Keyring::Absent => {
            return Ok(GnupgKeyring::Absent {
                home: home.display().to_string(),
            });
        }
    };
    keys.drop_source(KeySource::Gnupg).await?;
    let now = chrono::Utc::now().timestamp();
    let mut senders: Vec<postio_model::EmailAddress> = Vec::new();
    for key in &found {
        let addresses: Vec<postio_model::EmailAddress> = key
            .addresses
            .iter()
            .map(|address| postio_model::EmailAddress::new(None::<String>, address.as_str()))
            .collect();
        keys.record(&NewKey {
            fingerprint: &key.fingerprint,
            kind: KeyKind::OpenPgp,
            keyblock: &key.keyblock,
            primary_uid: key.primary_uid.as_deref(),
            revoked: key.revoked,
            addresses: &addresses,
            source: KeySource::Gnupg,
            message: None,
            at: now,
        })
        .await?;
        senders.extend(addresses);
    }
    let mut again = Vec::new();
    for sender in &senders {
        again.extend(keys.signed_messages_from(sender).await?);
    }
    drop(connection);
    again.sort_unstable();
    again.dedup();
    postio_session::signature::check_messages(database, blobs, &again).await?;
    tracing::info!(
        keys = found.len(),
        skipped,
        rechecked = again.len(),
        "GnuPG keyring read"
    );
    Ok(GnupgKeyring::Read {
        keys: found.len(),
        skipped,
    })
}
