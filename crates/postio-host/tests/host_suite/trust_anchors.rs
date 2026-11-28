use base64::Engine;

fn root_pem() -> String {
    let der = std::fs::read(format!(
        "{}/../postio-model/tests/corpus/keys/smime/root.der",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the corpus root");
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|line| std::str::from_utf8(line).expect("ascii"))
        .collect();
    format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        lines.join("\n")
    )
}

#[test]
fn the_first_bundle_present_is_read_and_the_rest_are_not() {
    let dir = tempfile::tempdir().expect("a directory");
    let email = dir.path().join("email-ca-bundle.pem");
    let tls = dir.path().join("ca-bundle.crt");
    std::fs::write(&email, root_pem()).expect("an email bundle");
    std::fs::write(&tls, format!("{0}{0}", root_pem())).expect("a TLS bundle");
    let read = postio_host::anchors::read(&[dir.path().join("absent.pem"), email.clone(), tls]);
    assert_eq!(read.from.as_deref(), Some(email.as_path()));
    assert_eq!(read.certificates.len(), 1);
}

#[test]
fn a_directory_of_certificates_is_read_file_by_file() {
    let dir = tempfile::tempdir().expect("a directory");
    std::fs::write(dir.path().join("one.pem"), root_pem()).expect("one");
    std::fs::write(dir.path().join("two.crt"), root_pem()).expect("two");
    std::fs::write(dir.path().join("README"), "not a certificate").expect("noise");
    let read = postio_host::anchors::read(&[dir.path().to_path_buf()]);
    assert_eq!(read.certificates.len(), 2);
}

#[test]
fn no_anchor_anywhere_is_an_empty_set_not_an_error() {
    let dir = tempfile::tempdir().expect("a directory");
    let read = postio_host::anchors::read(&[dir.path().join("absent.pem")]);
    assert_eq!(read.from, None);
    assert!(read.certificates.is_empty());
}

#[test]
fn the_system_paths_prefer_the_email_bundle() {
    assert_eq!(
        postio_host::anchors::SYSTEM.first().copied(),
        Some("/etc/pki/ca-trust/extracted/pem/email-ca-bundle.pem"),
        "a TLS bundle trusts CAs for servers, not for mail"
    );
}

fn corpus_root() -> Vec<Vec<u8>> {
    vec![
        std::fs::read(format!(
            "{}/../postio-model/tests/corpus/keys/smime/root.der",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("the corpus root"),
    ]
}

#[test]
fn s_mime_mail_is_checked_against_the_anchors_the_host_names() {
    use postio_storage::repository::KeyRepository;
    postio_session::signature::read_anchors_with(corpus_root);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let scene = rt.block_on(crate::source_view::Scene::new());
    let good = rt.block_on(scene.file("smime-signed-good", true));
    let stranger = rt.block_on(scene.file("smime-signed-unknown-anchor", true));
    rt.block_on(postio_session::signature::check_messages(
        &scene.database,
        &scene.blobs,
        &[good, stranger],
    ))
    .expect("checked");
    let verdict = |message| {
        rt.block_on(async {
            let connection = scene.database.connect().await.expect("c");
            KeyRepository::new(&connection)
                .verdict(message)
                .await
                .expect("r")
                .map(|row| (row.format, row.status, row.reason))
        })
    };
    assert_eq!(
        verdict(good),
        Some(("smime".to_owned(), "good".to_owned(), None))
    );
    assert_eq!(
        verdict(stranger),
        Some((
            "smime".to_owned(),
            "cant_check".to_owned(),
            Some("untrusted".to_owned())
        ))
    );
}
