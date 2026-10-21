//! Scoring two records that may be one person (specs/011-focus-contacts S10,
//! handoff screen 07). A pair is offered only with evidence a person can
//! read, and every line the sheet shows is one of the scorer's: no evidence,
//! no candidate. All names and addresses are invented, on reserved domains.

use postio_classify::duplicates::{Evidence, Pair, Side, describe, score};

fn side(name: &str, addresses: &[&str], title: &str, company: &str) -> Side {
    Side {
        name: name.to_owned(),
        addresses: addresses.iter().map(|a| (*a).to_owned()).collect(),
        title: (!title.is_empty()).then(|| title.to_owned()),
        company: (!company.is_empty()).then(|| company.to_owned()),
    }
}

#[test]
fn no_evidence_is_no_candidate() {
    let a = side("Ada Moreno", &["ada@example.org"], "", "");
    let b = side("Ben Adeyemi", &["ben@example.net"], "", "");
    assert_eq!(score(&a, &b, &Pair::default()), None);
}

#[test]
fn the_same_name_ignoring_case_and_spacing_is_evidence() {
    let a = side("Ada  Moreno", &["ada@example.org"], "", "");
    let b = side("ada moreno", &["a.moreno@example.net"], "", "");
    let scored = score(&a, &b, &Pair::default()).expect("a candidate");
    assert!(
        scored
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::SameName(name) if name == "Ada Moreno"))
    );
}

#[test]
fn the_same_title_and_company_in_the_signature_is_evidence_and_a_company_alone_is_not() {
    let a = side(
        "Ada Moreno",
        &["ada@example.org"],
        "Finance lead",
        "Northwind Partners",
    );
    let b = side(
        "A. Moreno",
        &["a.moreno@example.net"],
        "Finance lead",
        "Northwind Partners",
    );
    let scored = score(&a, &b, &Pair::default()).expect("a candidate");
    assert!(scored.evidence.iter().any(|e| matches!(
        e,
        Evidence::SameSignature { title, company } if title == "Finance lead" && company == "Northwind Partners"
    )));
    let colleague = side(
        "Ben",
        &["ben@example.org"],
        "Engineer",
        "Northwind Partners",
    );
    assert_eq!(
        score(&a, &colleague, &Pair::default()),
        None,
        "a colleague is not the same person"
    );
}

#[test]
fn an_address_that_stopped_when_another_began_in_the_same_threads_is_evidence() {
    let a = side("Ada", &["ada@example.org"], "", "");
    let b = side("A", &["a.moreno@example.net"], "", "");
    let pair = Pair {
        took_over_in_threads: 4,
        ..Pair::default()
    };
    let scored = score(&a, &b, &pair).expect("a candidate");
    assert!(
        scored
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::TookOver { threads: 4, .. }))
    );
    // One thread is a coincidence.
    let once = Pair {
        took_over_in_threads: 1,
        ..Pair::default()
    };
    assert_eq!(score(&a, &b, &once), None);
}

#[test]
fn a_resent_from_header_is_evidence_and_answers_across_addresses_are_too() {
    let a = side("Ada", &["ada@example.org"], "", "");
    let b = side("Ada M.", &["ada.moreno@example.com"], "", "");
    let pair = Pair {
        forwards_from: Some("ada@example.org".to_owned()),
        replies: 3,
        ..Pair::default()
    };
    let scored = score(&a, &b, &pair).expect("a candidate");
    assert!(
        scored
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::Forwards { .. }))
    );
    assert!(
        scored
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::Replied { answers: 3 }))
    );
}

#[test]
fn more_evidence_scores_higher() {
    let a = side(
        "Ada Moreno",
        &["ada@example.org"],
        "Finance lead",
        "Northwind Partners",
    );
    let b = side(
        "Ada Moreno",
        &["a@example.net"],
        "Finance lead",
        "Northwind Partners",
    );
    let one = score(
        &side("Ada Moreno", &["ada@example.org"], "", ""),
        &side("Ada Moreno", &["a@example.net"], "", ""),
        &Pair::default(),
    )
    .unwrap();
    let two = score(&a, &b, &Pair::default()).unwrap();
    assert!(two.score > one.score);
}

#[test]
fn the_sheet_says_all_three_when_every_pair_shares_the_signature() {
    let signature = Evidence::SameSignature {
        title: "Finance lead".into(),
        company: "Northwind Partners".into(),
    };
    let lines = describe(
        3,
        &[
            vec![signature.clone()],
            vec![signature.clone()],
            vec![signature],
        ],
        "Ada Moreno",
    );
    assert_eq!(
        lines[0],
        "The same signature on all three: \u{201c}Ada Moreno \u{b7} Finance lead, Northwind Partners\u{201d}"
    );
    let took = Evidence::TookOver {
        old: "a.moreno@example.net".into(),
        new: "ada@example.org".into(),
        threads: 4,
    };
    let lines = describe(2, &[vec![took]], "Ada");
    assert_eq!(
        lines[0],
        "a.moreno@example.net stopped when ada@example.org began, in the same 4 threads"
    );
    let forwards = Evidence::Forwards {
        from: "ada@example.org".into(),
        to: "ada.moreno@example.com".into(),
    };
    assert_eq!(
        describe(2, &[vec![forwards]], "Ada")[0],
        "ada.moreno@example.com forwards from ada@example.org (Resent-From header)"
    );
}
