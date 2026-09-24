//! A card goes in and comes back out with everything Postio does not model
//! untouched (specs/005-contacts FR-050..FR-054, SC-006, contracts/vcard.md).

use postio_vcard::{ExportPerson, Member, ParsedCard, export, export_group, parse};

fn corpus(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/corpus/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("a corpus file")
}

/// The properties Postio writes itself; every other line must survive.
const MODELLED: &[&str] = &[
    "BEGIN", "END", "VERSION", "UID", "FN", "EMAIL", "ORG", "NOTE", "REV", "KIND", "MEMBER",
];

/// A card's lines that are not Postio's to write, in order.
fn unmodelled(card: &str) -> Vec<String> {
    card.split("\r\n")
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let head = line.split([':', ';']).next().unwrap_or_default();
            let name = head
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            !MODELLED.contains(&name.as_str())
        })
        .map(str::to_owned)
        .collect()
}

fn person(card: &str) -> postio_vcard::ParsedPerson {
    let import = parse(card.as_bytes());
    assert!(import.skipped.is_empty(), "{:?}", import.skipped);
    match import.cards.into_iter().next() {
        Some(ParsedCard::Person(person)) => person,
        other => panic!("not a person: {other:?}"),
    }
}

/// Exports `person` as the store would hand it back, unchanged.
fn round_trip(card: &str) -> String {
    let parsed = person(card);
    let emails: Vec<(String, bool)> = parsed
        .emails
        .iter()
        .map(|(address, preferred)| (address.address.clone(), *preferred))
        .collect();
    export(
        &ExportPerson {
            uid: parsed.uid.as_deref().unwrap_or("urn:uuid:unused"),
            name: parsed.name.as_deref(),
            emails: &emails,
            organization: parsed.organization.as_deref(),
            note: parsed.note.as_deref(),
        },
        Some(&parsed.raw),
    )
}

#[test]
fn a_4_0_card_keeps_every_unmodelled_line_byte_for_byte() {
    let card = corpus("v4.vcf");
    assert_eq!(unmodelled(&round_trip(&card)), unmodelled(&card));
}

#[test]
fn a_3_0_card_changes_only_what_4_0_forbids() {
    let out = round_trip(&corpus("v3.vcf"));
    assert!(out.contains("VERSION:4.0\r\n"), "{out}");
    assert_eq!(
        unmodelled(&out),
        [
            "N:Lovelace;Ada;;;",
            "item1.X-ABLabel:Home",
            "TEL;TYPE=CELL:+1 555 0100",
            "PHOTO:data:image/jpeg;base64,/9j/4AAQSkZJRgABAQ==",
            "X-EXAMPLE-VENDOR:kept as is",
            "CATEGORIES:Friends,Engines",
        ],
        "ENCODING=b became a data: URI and CHARSET went; nothing else moved"
    );
    assert!(
        out.contains("item1.EMAIL;TYPE=INTERNET;PREF=1:ada@home.example\r\n"),
        "TYPE=PREF became PREF=1, the group prefix kept: {out}"
    );
}

#[test]
fn two_emails_are_one_person_and_pref_marks_the_preferred() {
    for file in ["v3.vcf", "v4.vcf"] {
        let ada = person(&corpus(file));
        assert_eq!(ada.name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(ada.organization.as_deref(), Some("Analytical Engines"));
        assert_eq!(ada.note.as_deref(), Some("met at the conference"));
        let emails: Vec<(&str, bool)> = ada
            .emails
            .iter()
            .map(|(a, p)| (a.address.as_str(), *p))
            .collect();
        assert_eq!(
            emails,
            [("ada@home.example", true), ("ada@work.example", false)],
            "{file}"
        );
        assert!(
            ada.raw.contains("CATEGORIES:Friends,Engines"),
            "kept verbatim"
        );
    }
}

#[test]
fn a_group_card_is_a_group_with_its_members() {
    let import = parse(corpus("group.vcf").as_bytes());
    let Some(ParsedCard::Group(group)) = import.cards.into_iter().next() else {
        panic!("not a group");
    };
    assert_eq!(group.name, "Family");
    assert_eq!(
        group.members,
        [
            Member::Uid("urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1".into()),
            Member::Address("grace@example.org".into()),
        ]
    );
    let out = export_group(
        group.uid.as_deref().unwrap_or_default(),
        &group.name,
        &["urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1".to_owned()],
        Some(&group.raw),
    );
    assert!(out.contains("KIND:group\r\n"));
    assert!(out.contains("MEMBER:urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1\r\n"));
    assert!(
        !out.contains("mailto:grace"),
        "members are what the store says now"
    );
}

#[test]
fn a_malformed_card_is_skipped_with_a_reason_and_the_rest_import() {
    let import = parse(corpus("malformed.vcf").as_bytes());
    let names: Vec<String> = import
        .cards
        .iter()
        .filter_map(|card| match card {
            ParsedCard::Person(p) => p.name.clone(),
            ParsedCard::Group(_) => None,
        })
        .collect();
    assert_eq!(names, ["Grace Hopper", "Katherine Johnson"]);
    assert_eq!(import.skipped.len(), 1);
    assert_eq!(import.skipped[0].index, 1, "the second card");
    assert!(!import.skipped[0].reason.is_empty());
}

#[test]
fn a_person_with_no_card_gets_a_fresh_4_0_one() {
    let out = export(
        &ExportPerson {
            uid: "urn:uuid:00000000-0000-4000-8000-000000000001",
            name: Some("Grace Hopper, Rear Admiral"),
            emails: &[
                ("grace@example.org".to_owned(), false),
                ("g@example.org".to_owned(), true),
            ],
            organization: None,
            note: Some("line one\nline two"),
        },
        None,
    );
    assert_eq!(
        out,
        "BEGIN:VCARD\r\nVERSION:4.0\r\n\
         UID:urn:uuid:00000000-0000-4000-8000-000000000001\r\n\
         FN:Grace Hopper\\, Rear Admiral\r\n\
         EMAIL:grace@example.org\r\n\
         EMAIL;PREF=1:g@example.org\r\n\
         NOTE:line one\\nline two\r\n\
         END:VCARD\r\n"
    );
    // And it reads back as it was written.
    let back = person(&out);
    assert_eq!(back.name.as_deref(), Some("Grace Hopper, Rear Admiral"));
    assert_eq!(back.note.as_deref(), Some("line one\nline two"));
}
