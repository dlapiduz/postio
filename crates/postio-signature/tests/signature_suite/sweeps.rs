use pgp::composed::{Deserializable, SignedPublicKey};
use pgp::types::KeyDetails;
use postio_model::test_corpus::{self, Category};
use postio_signature::{KeySet, Policy, Reason, Status, verify};

fn key(name: &str) -> SignedPublicKey {
    let path = format!(
        "{}/../postio-model/tests/corpus/keys/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).expect("a corpus key");
    SignedPublicKey::from_string(&text).expect("it parses").0
}

fn subsets() -> Vec<(Vec<&'static str>, KeySet)> {
    let names = ["ada-a.asc", "ada-b.asc"];
    (0..1u8 << names.len())
        .map(|mask| {
            let chosen: Vec<&str> = names
                .iter()
                .enumerate()
                .filter(|(at, _)| mask & (1 << at) != 0)
                .map(|(_, name)| *name)
                .collect();
            let mut set = KeySet::new();
            for name in &chosen {
                set.push(key(name));
            }
            (chosen, set)
        })
        .collect()
}

const UNTOUCHED: &[&str] = &[
    "pgp-signed-good",
    "pgp-signed-qp",
    "pgp-signed-other-key",
    "pgp-signed-list-footer",
    "pgp-inline-signed",
];

#[test]
fn nothing_that_cannot_be_checked_is_ever_good() {
    for fixture in test_corpus::all() {
        for (chosen, keys) in subsets() {
            let Some(verdict) = verify(fixture.bytes(), &keys, &Policy) else {
                continue;
            };
            if verdict.status != Status::Good {
                continue;
            }
            assert!(
                UNTOUCHED.contains(&fixture.name()),
                "{} read as good with {chosen:?}",
                fixture.name()
            );
            let signer = verdict.signer.expect("a good signature names its key");
            assert!(
                keys.keys()
                    .iter()
                    .any(|key| key.fingerprint().to_string().eq_ignore_ascii_case(&signer)),
                "{} is good with {chosen:?}, whose keys did not sign it",
                fixture.name()
            );
        }
    }
}

#[test]
fn every_tampering_reads_as_tampered() {
    let all = subsets().pop().expect("the full set").1;
    for fixture in [
        "pgp-signed-tampered-body",
        "pgp-signed-tampered-header",
        "pgp-signed-reencoded",
    ] {
        let verdict = verify(test_corpus::load(fixture).bytes(), &all, &Policy).expect("signed");
        assert_eq!(
            (verdict.status, verdict.reason),
            (Status::Bad, Some(Reason::Tampered)),
            "{fixture}"
        );
    }
}

#[test]
fn every_signature_fixture_has_a_verdict_and_ordinary_mail_has_none() {
    let all = subsets().pop().expect("the full set").1;
    for fixture in test_corpus::by_category(Category::Pgp) {
        assert!(
            verify(fixture.bytes(), &all, &Policy).is_some()
                || fixture.name() == "pgp-key-attached",
            "{}",
            fixture.name()
        );
    }
    for fixture in test_corpus::by_category(Category::PlainText)
        .into_iter()
        .filter(|fixture| !fixture.categories().contains(&Category::Pgp))
    {
        assert_eq!(
            verify(fixture.bytes(), &all, &Policy),
            None,
            "{}",
            fixture.name()
        );
    }
}
