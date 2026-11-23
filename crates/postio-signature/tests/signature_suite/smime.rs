use postio_model::test_corpus;
use postio_signature::{Format, KeySet, Policy, Reason, Status, verify};

fn anchor(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../postio-model/tests/corpus/keys/smime/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    postio_signature::smime::anchors_in(&std::fs::read(path).expect("a corpus root")).remove(0)
}

fn trusting(names: &[&str]) -> KeySet {
    let mut keys = KeySet::new();
    for name in names {
        keys.push_anchor(anchor(name));
    }
    keys
}

fn judged(fixture: &str, keys: &KeySet) -> (Status, Option<Reason>) {
    let verdict = verify(test_corpus::load(fixture).bytes(), keys, &Policy).expect("signed");
    assert_eq!(verdict.scope.format(), Some(Format::Smime), "{fixture}");
    (verdict.status, verdict.reason)
}

#[test]
fn a_chain_to_an_anchor_handed_in_for_the_from_address_is_good() {
    let keys = trusting(&["root.der"]);
    let verdict = verify(
        test_corpus::load("smime-signed-good").bytes(),
        &keys,
        &Policy,
    )
    .expect("signed");
    assert_eq!((verdict.status, verdict.reason), (Status::Good, None));
    let signer = verdict.signer.expect("the certificate's fingerprint");
    assert_eq!(signer.len(), 64, "SHA-256 of the certificate: {signer}");
    assert!(verdict.signed_at.is_some(), "signingTime is read");
}

#[test]
fn each_way_an_s_mime_signature_fails_says_why() {
    let keys = trusting(&["root.der"]);
    for (fixture, expected) in [
        (
            "smime-signed-tampered",
            (Status::Bad, Some(Reason::Tampered)),
        ),
        ("smime-signed-expired", (Status::Bad, Some(Reason::Expired))),
        (
            "smime-signed-unknown-anchor",
            (Status::CantCheck, Some(Reason::Untrusted)),
        ),
        (
            "smime-signed-address-mismatch",
            (Status::Bad, Some(Reason::AddressMismatch)),
        ),
    ] {
        assert_eq!(judged(fixture, &keys), expected, "{fixture}");
    }
}

#[test]
fn with_no_anchor_nothing_is_good() {
    assert_eq!(
        judged("smime-signed-good", &KeySet::new()),
        (Status::CantCheck, Some(Reason::Untrusted))
    );
    assert_eq!(
        judged("smime-signed-good", &trusting(&["stranger.der"])),
        (Status::CantCheck, Some(Reason::Untrusted))
    );
}

#[test]
fn a_certificate_bundle_yields_every_anchor_and_junk_yields_none() {
    use base64::Engine;
    let pem = |name: &str| {
        let der = std::fs::read(format!(
            "{}/../postio-model/tests/corpus/keys/smime/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("a corpus root");
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            base64::engine::general_purpose::STANDARD
                .encode(der)
                .as_bytes()
                .chunks(64)
                .map(|line| String::from_utf8_lossy(line).into_owned())
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let bundle = format!(
        "{}# a comment between certificates\n{}",
        pem("root.der"),
        pem("stranger.der")
    );
    assert_eq!(
        postio_signature::smime::anchors_in(bundle.as_bytes()).len(),
        2
    );
    assert!(postio_signature::smime::anchors_in(b"not a certificate").is_empty());
}
