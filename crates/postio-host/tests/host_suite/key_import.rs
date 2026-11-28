use postio_client::protocol::ImportFrom;
use postio_model::ContactView;
use postio_storage::repository::{ContactRepository, KeyRepository, KeySource};
use postio_ui::source::Imported;

use crate::source_view::Scene;

fn current_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

fn corpus_key(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "{}/../postio-model/tests/corpus/keys/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

#[test]
fn a_key_file_is_kept_for_the_address_it_names_and_makes_an_unsaved_contact() {
    let rt = current_thread();
    let scene = rt.block_on(Scene::new());
    let host =
        postio_host::Host::start(scene.database.clone(), scene.blobs.clone(), |wiring| wiring)
            .expect("a host");
    let client = host.connect(postio_client::protocol::ClientKind::Test);
    let everyone = || {
        rt.block_on(async {
            let connection = scene.database.connect().await.expect("c");
            ContactRepository::new(&connection)
                .count(ContactView::Everyone)
                .await
                .expect("a count")
        })
    };
    assert_eq!(everyone(), 0);
    let imported = rt
        .block_on(client.import_keys(ImportFrom::File(corpus_key("ada-a.asc"))))
        .expect("imported");
    assert_eq!(
        imported,
        Imported {
            keys: 1,
            addresses: vec!["ada@example.com".to_owned()]
        }
    );
    assert_eq!(
        postio_ui::source::imported_line(&imported),
        "Imported 1 key for ada@example.com."
    );
    assert_eq!(
        everyone(),
        1,
        "a user id naming no contact makes an unsaved one"
    );
    rt.block_on(async {
        let connection = scene.database.connect().await.expect("c");
        let keys = KeyRepository::new(&connection);
        let held = keys
            .for_address(&postio_model::EmailAddress::new(
                None::<String>,
                "ada@example.com",
            ))
            .await
            .expect("r");
        assert_eq!(held.len(), 1);
        assert_eq!(
            keys.sources(&held[0].fingerprint).await.expect("r"),
            [KeySource::File]
        );
    });
    host.stop();
}

#[test]
fn importing_a_key_attached_to_a_message_checks_its_signed_mail_again() {
    let rt = current_thread();
    let scene = rt.block_on(Scene::new());
    let signed = rt.block_on(scene.file("pgp-signed-good", true));
    let carrier = rt.block_on(scene.file("pgp-key-attached", true));
    rt.block_on(postio_session::signature::check_messages(
        &scene.database,
        &scene.blobs,
        &[signed],
    ))
    .expect("checked");
    let status = || {
        rt.block_on(async {
            let connection = scene.database.connect().await.expect("c");
            KeyRepository::new(&connection)
                .verdict(signed)
                .await
                .expect("r")
                .map(|row| row.status)
        })
    };
    assert_eq!(status().as_deref(), Some("cant_check"));
    let host =
        postio_host::Host::start(scene.database.clone(), scene.blobs.clone(), |wiring| wiring)
            .expect("a host");
    let client = host.connect(postio_client::protocol::ClientKind::Test);
    let imported = rt
        .block_on(client.import_keys(ImportFrom::Message(carrier)))
        .expect("imported");
    assert_eq!(imported.keys, 1);
    assert_eq!(
        status().as_deref(),
        Some("good"),
        "the signature checks now"
    );
    host.stop();
}

#[test]
fn a_file_with_no_key_says_so_and_records_nothing() {
    let rt = current_thread();
    let scene = rt.block_on(Scene::new());
    let host =
        postio_host::Host::start(scene.database.clone(), scene.blobs.clone(), |wiring| wiring)
            .expect("a host");
    let client = host.connect(postio_client::protocol::ClientKind::Test);
    let dir = tempfile::tempdir().expect("a directory");
    let junk = dir.path().join("notes.asc");
    std::fs::write(&junk, b"not a key at all").expect("a file");
    let imported = rt
        .block_on(client.import_keys(ImportFrom::File(junk)))
        .expect("answered");
    assert_eq!(imported, Imported::default());
    assert_eq!(
        postio_ui::source::imported_line(&imported),
        "There's no public key in that file."
    );
    host.stop();
}
