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
