use std::path::Path;

use postio_host::gnupg::{self, Format, Keyring};

fn keyblock(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../postio-model/tests/corpus/keys/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    postio_signature::keys_in(&std::fs::read(path).expect("a corpus key"))
        .remove(0)
        .keyblock
}

fn blob(kind: u8, payload: &[u8]) -> Vec<u8> {
    let length = 20 + payload.len();
    let mut out = (length as u32).to_be_bytes().to_vec();
    out.extend_from_slice(&[kind, 1, 0, 0]);
    out.extend_from_slice(&20u32.to_be_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(payload);
    out
}

fn keybox(names: &[&str]) -> Vec<u8> {
    let mut out = 32u32.to_be_bytes().to_vec();
    out.push(1);
    out.extend_from_slice(&[1; 27]);
    for name in names {
        out.extend(blob(2, &keyblock(name)));
    }
    out
}

fn home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("a GNUPGHOME");
    std::fs::create_dir(home.path().join("private-keys-v1.d")).expect("a secret dir");
    std::fs::write(home.path().join("private-keys-v1.d/key.key"), b"secret").expect("decoy");
    std::fs::write(home.path().join("secring.gpg"), b"secret").expect("decoy");
    std::fs::write(home.path().join("trustdb.gpg"), b"trust").expect("decoy");
    home
}

fn read(home: &Path) -> Keyring {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(gnupg::read(home))
}

fn summary(keyring: &Keyring) -> (Option<Format>, usize, u32, Option<String>) {
    match keyring {
        Keyring::Read {
            format,
            keys,
            skipped,
            opened,
        } => (
            Some(*format),
            keys.len(),
            *skipped,
            opened
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        ),
        Keyring::Unreadable { opened } => (
            None,
            0,
            0,
            opened
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned()),
        ),
        Keyring::Absent => (None, 0, 0, None),
    }
}

#[test]
fn pubring_gpg_is_read_past_its_trust_packets() {
    let home = home();
    let mut bytes = keyblock("ada-a.asc");
    bytes.extend([0xb0, 0x02, 0x06, 0x00]);
    bytes.extend(keyblock("ada-b.asc"));
    std::fs::write(home.path().join("pubring.gpg"), bytes).expect("a keyring");
    assert_eq!(
        summary(&read(home.path())),
        (Some(Format::Pubring), 2, 0, Some("pubring.gpg".to_owned()))
    );
}

#[test]
fn pubring_kbx_is_read_and_wins_over_pubring_gpg() {
    let home = home();
    std::fs::write(home.path().join("pubring.kbx"), keybox(&["ada-a.asc"])).expect("a keybox");
    std::fs::write(home.path().join("pubring.gpg"), keyblock("ada-b.asc")).expect("an old one");
    assert_eq!(
        summary(&read(home.path())),
        (Some(Format::Keybox), 1, 0, Some("pubring.kbx".to_owned()))
    );
}

#[test]
fn keyboxds_database_is_read_read_only_and_left_as_it_was() {
    let home = home();
    let dir = home.path().join("public-keys.d");
    std::fs::create_dir(&dir).expect("keyboxd's dir");
    let db = dir.join("pubring.db");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async {
            let database = turso::Builder::new_local(db.to_str().expect("utf-8"))
                .build()
                .await
                .expect("a database");
            let connection = database.connect().expect("a connection");
            connection
                .execute(
                    "CREATE TABLE pubkey (ubid BLOB NOT NULL PRIMARY KEY, type INTEGER NOT NULL,
                     ephemeral INTEGER NOT NULL DEFAULT 0, revoked INTEGER NOT NULL DEFAULT 0,
                     keyblob BLOB NOT NULL)",
                    (),
                )
                .await
                .expect("the table");
            for (n, name) in ["ada-a.asc", "ada-b.asc"].into_iter().enumerate() {
                connection
                    .execute(
                        "INSERT INTO pubkey (ubid, type, keyblob) VALUES (?1, 1, ?2)",
                        (vec![n as u8], keyblock(name)),
                    )
                    .await
                    .expect("a key");
            }
        });
    let before: Vec<_> = std::fs::read_dir(&dir)
        .expect("listed")
        .map(|entry| {
            let entry = entry.expect("an entry");
            (entry.file_name(), entry.metadata().expect("metadata").len())
        })
        .collect();
    assert_eq!(
        summary(&read(home.path())),
        (Some(Format::Keyboxd), 2, 0, Some("pubring.db".to_owned()))
    );
    let after: Vec<_> = std::fs::read_dir(&dir)
        .expect("listed")
        .map(|entry| {
            let entry = entry.expect("an entry");
            (entry.file_name(), entry.metadata().expect("metadata").len())
        })
        .collect();
    assert_eq!(before, after, "nothing in keyboxd's directory was written");
}

#[test]
fn a_changed_keyring_reads_its_new_keys() {
    let home = home();
    let path = home.path().join("pubring.kbx");
    std::fs::write(&path, keybox(&["ada-a.asc"])).expect("a keybox");
    assert_eq!(summary(&read(home.path())).1, 1);
    std::fs::write(&path, keybox(&["ada-a.asc", "ada-b.asc"])).expect("a key added");
    assert_eq!(summary(&read(home.path())).1, 2);
}

#[test]
fn an_unreadable_keyring_says_so_and_an_empty_home_is_absent() {
    let home = home();
    assert_eq!(read(home.path()), Keyring::Absent);
    std::fs::write(
        home.path().join("pubring.kbx"),
        b"\x00\x00\x00\x30\x02garbage",
    )
    .expect("junk");
    assert!(matches!(read(home.path()), Keyring::Unreadable { .. }));
}

#[test]
fn gnupghome_wins_over_the_home_directory() {
    let explicit = std::ffi::OsString::from("/srv/keys");
    assert_eq!(
        gnupg::home(Some(&explicit), Some(Path::new("/home/ada"))),
        Some(std::path::PathBuf::from("/srv/keys"))
    );
    assert_eq!(
        gnupg::home(None, Some(Path::new("/home/ada"))),
        Some(std::path::PathBuf::from("/home/ada/.gnupg"))
    );
}

#[test]
fn turning_the_keyring_on_records_its_keys_and_off_forgets_the_ones_only_it_held() {
    use postio_storage::repository::{KeyRepository, KeySource};
    use postio_ui::settings::GnupgKeyring;
    let home = home();
    std::fs::write(
        home.path().join("pubring.kbx"),
        keybox(&["ada-a.asc", "ada-b.asc"]),
    )
    .expect("a keybox");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let database = rt.block_on(postio_storage::test_support::memory());
    let dir = tempfile::tempdir().expect("blobs");
    let blobs = postio_storage::BlobStore::open(
        dir.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let ada = postio_model::EmailAddress::new(None::<String>, "ada@example.com");
    let held = || {
        rt.block_on(async {
            let connection = database.connect().await.expect("c");
            KeyRepository::new(&connection)
                .for_address(&ada)
                .await
                .expect("r")
                .into_iter()
                .map(|key| key.fingerprint)
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        rt.block_on(gnupg::sync(&database, &blobs, Some(home.path()), true))
            .expect("read"),
        GnupgKeyring::Read {
            keys: 2,
            skipped: 0
        }
    );
    let both = held();
    assert_eq!(both.len(), 2);
    rt.block_on(async {
        let connection = database.connect().await.expect("c");
        let keys = KeyRepository::new(&connection);
        assert_eq!(keys.sources(&both[0]).await.expect("r"), [KeySource::Gnupg]);
        keys.accept(&ada, &both[0], 1).await.expect("accepted");
    });
    assert_eq!(
        rt.block_on(gnupg::sync(&database, &blobs, Some(home.path()), false))
            .expect("off"),
        GnupgKeyring::Off
    );
    assert_eq!(held(), [both[0].clone()], "only the accepted key stays");
}
