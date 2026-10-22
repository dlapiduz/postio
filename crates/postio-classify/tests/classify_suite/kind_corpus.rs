//! Kind detection, held to SC-003 (specs/011-focus-contacts).
//!
//! `tests/kind_corpus/*.toml` is one invented sender per file, hand-labelled:
//! the headers a sender's newest messages carried, counted, and the kind a
//! person would call it. Every address is on a reserved domain. The detector
//! must agree with the labels at least 95% of the time, and must never read
//! a person as a robot: a person filtered away as automated is mail the user
//! does not see, which is the worse error by far.

use postio_classify::kind::{self, KindRules};
use postio_model::{ContactKind, KindSignals};
use serde::Deserialize;

#[derive(Deserialize)]
struct Labelled {
    label: String,
    #[serde(flatten)]
    signals: KindSignals,
}

fn corpus() -> Vec<(String, Labelled)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/kind_corpus");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("the corpus directory reads")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("a corpus file reads");
            let labelled: Labelled = toml::from_str(&text)
                .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
            (
                path.file_stem().unwrap().to_string_lossy().into_owned(),
                labelled,
            )
        })
        .collect()
}

#[test]
fn the_corpus_is_big_enough_to_mean_something_and_uses_reserved_domains() {
    let corpus = corpus();
    assert!(corpus.len() >= 40, "{} senders", corpus.len());
    for kind in ContactKind::ALL {
        assert!(
            corpus.iter().any(|(_, one)| one.label == kind.as_str()),
            "no {kind:?} in the corpus"
        );
    }
    for (name, one) in &corpus {
        let domain = one.signals.address.rsplit('@').next().unwrap();
        let reserved = domain.ends_with(".example")
            || domain.ends_with(".test")
            || ["example.com", "example.org", "example.net"]
                .iter()
                .any(|reserved| domain == *reserved || domain.ends_with(&format!(".{reserved}")));
        assert!(
            reserved,
            "{name} uses {domain}, which is not a reserved domain"
        );
    }
}

#[test]
fn detection_agrees_with_a_person_s_label_at_least_95_percent_of_the_time() {
    let rules = KindRules::shipped();
    let corpus = corpus();
    let wrong: Vec<String> = corpus
        .iter()
        .filter_map(|(name, one)| {
            let got = kind::detect(&one.signals, rules).kind;
            (got.as_str() != one.label)
                .then(|| format!("{name}: labelled {}, read {got:?}", one.label))
        })
        .collect();
    let agreed = corpus.len() - wrong.len();
    assert!(
        agreed * 100 >= corpus.len() * 95,
        "{agreed} of {} agree (95% is the floor). Wrong: {wrong:#?}",
        corpus.len()
    );
}

#[test]
fn a_person_is_never_read_as_automated() {
    let rules = KindRules::shipped();
    for (name, one) in corpus().iter().filter(|(_, one)| one.label == "person") {
        assert_ne!(
            kind::detect(&one.signals, rules).kind,
            ContactKind::Automated,
            "{name} is a person"
        );
    }
}

fn signals(address: &str) -> KindSignals {
    KindSignals {
        address: address.into(),
        ..KindSignals::default()
    }
}

#[test]
fn writing_to_someone_floors_them_at_person_unless_they_are_a_list_or_newsletter() {
    let rules = KindRules::shipped();
    let robot = KindSignals {
        messages: 4,
        written_to: 1,
        ..signals("noreply@shop.example.com")
    };
    assert_eq!(kind::detect(&robot, rules).kind, ContactKind::Person);
    let newsletter = KindSignals {
        messages: 10,
        with_unsubscribe: 10,
        with_bulk: 10,
        written_to: 1,
        ..signals("news@shop.example.com")
    };
    assert_eq!(
        kind::detect(&newsletter, rules).kind,
        ContactKind::Newsletter
    );
}

#[test]
fn no_evidence_is_a_person_with_low_confidence_and_a_clear_case_is_confident() {
    let rules = KindRules::shipped();
    let unknown = kind::detect(&signals("quinn@example.org"), rules);
    assert_eq!(unknown.kind, ContactKind::Person);
    let list = kind::detect(
        &KindSignals {
            messages: 50,
            with_list_id: 50,
            with_list_precedence: 50,
            ..signals("dev@lists.projectx.example")
        },
        rules,
    );
    assert_eq!(list.kind, ContactKind::List);
    assert!(
        list.confidence > unknown.confidence,
        "{} should exceed {}",
        list.confidence,
        unknown.confidence
    );
}

#[test]
fn the_shipped_rules_load() {
    let rules = KindRules::shipped();
    assert!(
        KindRules::parse("nonsense = 1").is_err(),
        "an unknown key is refused"
    );
    assert!(
        KindRules::parse("").is_err(),
        "an empty table is refused: it would call everyone a person"
    );
    let _ = rules;
}
