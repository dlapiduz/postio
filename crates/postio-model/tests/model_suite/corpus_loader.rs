//! Acceptance for postio-nux: the `.eml` corpus and the loader that hands it out.
//!
//! These tests prove three things, in order of how much they matter:
//!
//! 1. The loader finds and reads **every** fixture, and the embedded bytes are
//!    the bytes on disk — a fixture that exists but is unreachable is worse than
//!    no fixture at all.
//! 2. The corpus actually covers the categories every downstream epic depends
//!    on, so MIME parsing, threading, search and the reader can be written
//!    test-first against it.
//! 3. The corpus contains no real personal data, only invented content under
//!    RFC 2606 reserved domains.

#![cfg(feature = "test-corpus")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use postio_model::test_corpus::{self, Category};
use postio_model::{AccountId, MailboxId, mime};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn files_on_disk() -> BTreeSet<String> {
    std::fs::read_dir(corpus_dir())
        .expect("corpus directory must exist")
        .map(|entry| entry.expect("readable dir entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".eml"))
        .collect()
}

// --------------------------------------------------------------- the loader

#[test]
fn loader_reaches_every_file_on_disk_and_nothing_else() {
    let on_disk = files_on_disk();
    let in_table: BTreeSet<String> = test_corpus::all()
        .iter()
        .map(|f| f.file_name().to_owned())
        .collect();

    let missing: Vec<&String> = on_disk.difference(&in_table).collect();
    assert!(
        missing.is_empty(),
        "these .eml files are in {} but not in the loader's table — add them to \
         the `corpus!` macro in src/test_corpus.rs and to the corpus README: {missing:?}",
        test_corpus::CORPUS_DIR
    );

    let phantom: Vec<&String> = in_table.difference(&on_disk).collect();
    assert!(
        phantom.is_empty(),
        "the loader lists fixtures that do not exist on disk: {phantom:?}"
    );
}

#[test]
fn every_fixture_reads_back_exactly_as_stored() {
    for fixture in test_corpus::all() {
        let path = corpus_dir().join(fixture.file_name());
        let on_disk =
            std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        assert_eq!(
            fixture.bytes(),
            on_disk.as_slice(),
            "embedded bytes differ from disk for {}",
            fixture.name()
        );
        assert_eq!(fixture.len(), on_disk.len());
        assert!(!fixture.is_empty());
    }
}

#[test]
fn every_fixture_looks_like_a_message() {
    for fixture in test_corpus::all() {
        let text = fixture.text_lossy();
        let first = text.lines().next().unwrap_or_default();
        assert!(
            first.contains(':'),
            "{} does not start with a header field: {first:?}",
            fixture.name()
        );
        assert!(
            text.contains("From:"),
            "{} has no From header",
            fixture.name()
        );
        assert!(
            !fixture.description().is_empty(),
            "{} has no description; every fixture must say why it exists",
            fixture.name()
        );
        assert!(
            !fixture.categories().is_empty(),
            "{} is untagged; tests ask for fixtures by category",
            fixture.name()
        );
        let unique: BTreeSet<Category> = fixture.categories().iter().copied().collect();
        assert_eq!(
            unique.len(),
            fixture.categories().len(),
            "{} repeats a category",
            fixture.name()
        );
    }
}

#[test]
fn names_are_unique_stable_and_sorted() {
    let names: Vec<&str> = test_corpus::names().collect();
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len(), "duplicate fixture name");

    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "fixtures must be listed in file-name order");

    for fixture in test_corpus::all() {
        assert_eq!(
            fixture.file_name(),
            format!("{}.eml", fixture.name()),
            "name and file name disagree"
        );
        assert!(
            fixture.source_path().ends_with(fixture.file_name()),
            "source_path must point at the fixture"
        );
    }
}

#[test]
fn lookup_accepts_a_name_with_or_without_the_suffix() {
    let by_stem = test_corpus::load("plain-text-simple");
    let by_file = test_corpus::load("plain-text-simple.eml");
    assert_eq!(by_stem, by_file);
    assert_eq!(by_stem.name(), "plain-text-simple");
    assert!(by_stem.bytes().starts_with(b"Return-Path:"));
    assert!(by_stem.has_category(Category::PlainText));
}

#[test]
fn lookup_of_an_unknown_fixture_is_none() {
    assert!(test_corpus::get("no-such-fixture").is_none());
}

#[test]
#[should_panic(expected = "no fixture named `no-such-fixture`")]
fn load_of_an_unknown_fixture_panics_with_the_known_names() {
    let _ = test_corpus::load("no-such-fixture");
}

#[test]
fn count_matches_the_table_and_the_disk() {
    assert_eq!(test_corpus::count(), test_corpus::all().len());
    assert_eq!(test_corpus::count(), files_on_disk().len());
}

// ------------------------------------------------------------- categories

#[test]
fn category_names_round_trip() {
    for category in Category::ALL {
        assert_eq!(Category::from_name(category.as_str()), Some(*category));
        assert_eq!(category.to_string(), category.as_str());
    }
    assert_eq!(Category::from_name("not-a-category"), None);
}

#[test]
fn every_category_has_at_least_one_fixture() {
    let unused: Vec<&str> = Category::ALL
        .iter()
        .filter(|c| test_corpus::by_category(**c).is_empty())
        .map(|c| c.as_str())
        .collect();
    assert!(
        unused.is_empty(),
        "categories with no fixture (either add one or drop the category): {unused:?}"
    );
}

#[test]
fn by_category_selects_exactly_the_tagged_fixtures() {
    for category in Category::ALL {
        let selected = test_corpus::by_category(*category);
        for fixture in &selected {
            assert!(fixture.has_category(*category));
        }
        let expected = test_corpus::all()
            .iter()
            .filter(|f| f.has_category(*category))
            .count();
        assert_eq!(selected.len(), expected);
    }
}

#[test]
fn by_categories_intersects_and_defaults_to_everything() {
    assert_eq!(test_corpus::by_categories(&[]).len(), test_corpus::count());

    let both = test_corpus::by_categories(&[Category::Threading, Category::BrokenReferences]);
    assert!(
        both.len() >= 3,
        "threading work needs several half-linked replies, found {}",
        both.len()
    );
    for fixture in both {
        assert!(fixture.has_category(Category::Threading));
        assert!(fixture.has_category(Category::BrokenReferences));
    }
}

// --------------------------------------------------------------- coverage

#[test]
fn corpus_covers_every_category_the_downstream_epics_need() {
    assert!(
        test_corpus::count() >= 20,
        "the corpus must hold at least 20 fixtures, found {}",
        test_corpus::count()
    );

    // Minimum counts, not just presence: one fixture per category is enough to
    // compile a test but not enough to characterize a behaviour.
    let required: &[(Category, usize)] = &[
        (Category::PlainText, 3),
        (Category::Html, 3),
        (Category::MultipartAlternative, 3),
        (Category::MultipartRelated, 2),
        (Category::InlineImage, 2),
        (Category::Attachment, 4),
        (Category::LargeAttachment, 1),
        (Category::NonUtf8Charset, 4),
        (Category::Base64, 4),
        (Category::QuotedPrintable, 3),
        (Category::EncodedWord, 4),
        (Category::MalformedHeaders, 2),
        (Category::MalformedStructure, 2),
        (Category::MissingHeaders, 2),
        (Category::Threading, 8),
        (Category::BrokenReferences, 4),
        (Category::MailingList, 8),
        (Category::Pgp, 2),
        (Category::RemoteContent, 1),
    ];
    for (category, minimum) in required {
        let found = test_corpus::by_category(*category).len();
        assert!(
            found >= *minimum,
            "category {category} needs at least {minimum} fixtures, found {found}"
        );
    }
}

#[test]
fn non_utf8_fixtures_really_are_not_utf8() {
    let mut invalid = 0;
    for fixture in test_corpus::by_category(Category::NonUtf8Charset) {
        // The lossy view always works; that is the point of having it.
        assert!(!fixture.text_lossy().is_empty());
        if fixture.as_str().is_none() {
            invalid += 1;
        }
    }
    assert!(
        invalid >= 2,
        "at least two fixtures must carry genuinely non-UTF-8 bytes, found {invalid}"
    );
    // And a UTF-8 fixture must round-trip through `as_str`.
    assert!(test_corpus::load("plain-text-simple").as_str().is_some());
}

#[test]
fn fixtures_carry_the_headers_their_categories_promise() {
    let checks: &[(&str, &str)] = &[
        ("attachment-large", "Content-Transfer-Encoding: base64"),
        ("broken-references", "In-Reply-To:"),
        ("bounce-delivery-status", "message/delivery-status"),
        ("calendar-invite", "text/calendar"),
        ("charset-iso-8859-1", "charset=iso-8859-1"),
        ("charset-shift-jis", "charset=Shift_JIS"),
        ("charset-utf-7", "charset=utf-7"),
        ("encoded-word-subject-and-names", "=?utf-8?B?"),
        ("html-newsletter", "List-Unsubscribe:"),
        (
            "html-tracking-pixel-remote-images",
            "pixel.tracker.example.org",
        ),
        ("inline-image-cid", "cid:reader-left.44b1@example.com"),
        ("malformed-headers", "This-Line-Has-No-Colon-At-All"),
        ("multipart-alternative", "multipart/alternative"),
        ("nested-multipart", "multipart/related"),
        ("pgp-encrypted", "-----BEGIN PGP MESSAGE-----"),
        ("pgp-signed", "-----BEGIN PGP SIGNATURE-----"),
        (
            "transfer-encoding-quoted-printable",
            "Content-Transfer-Encoding: quoted-printable",
        ),
    ];
    for (name, needle) in checks {
        let fixture = test_corpus::load(name);
        // Undo quoted-printable soft line breaks so a needle cannot be split
        // in half by the 76-column limit.
        let text = fixture.text_lossy().replace("=\r\n", "").replace("=\n", "");
        assert!(text.contains(needle), "{name} should contain {needle:?}");
    }
}

/// #597: `duplicate-message-id` declared `charset=us-ascii` and then put two
/// UTF-8 em-dashes in its body, which is not what its `[Threading,
/// PlainText]` categories or its description are about -- a fixture that
/// lies about its own charset teaches a dedup/threading test to expect
/// mojibake for a reason that has nothing to do with what the fixture is
/// named for. Corpus-wide, not just the one fixture, so nothing else can
/// grow the same mismatch unnoticed.
#[test]
fn a_fixture_declaring_us_ascii_contains_only_ascii_bytes() {
    for fixture in test_corpus::all() {
        if !fixture
            .text_lossy()
            .to_lowercase()
            .contains("charset=us-ascii")
        {
            continue;
        }
        assert!(
            fixture.bytes().is_ascii(),
            "{}: declares charset=us-ascii but contains a byte above 0x7f",
            fixture.name()
        );
    }
}

#[test]
fn the_large_attachment_is_actually_large() {
    let large = test_corpus::load("attachment-large");
    assert!(
        large.len() > 200 * 1024,
        "the large-attachment fixture is only {} bytes",
        large.len()
    );
    // ...and nothing else is, so the default suite stays quick.
    for fixture in test_corpus::all() {
        if fixture.has_category(Category::LargeAttachment) {
            continue;
        }
        assert!(
            fixture.len() < 64 * 1024,
            "{} is {} bytes but is not tagged LargeAttachment",
            fixture.name(),
            fixture.len()
        );
    }
}

#[test]
fn the_mailing_list_thread_is_a_real_thread() {
    let thread = test_corpus::by_categories(&[Category::MailingList, Category::Threading]);
    assert!(thread.len() >= 7, "the list thread needs some depth");

    let mut with_references = 0;
    let mut with_in_reply_to = 0;
    for fixture in &thread {
        let text = fixture.text_lossy();
        assert!(
            text.contains("List-Id:"),
            "{} claims MailingList but has no List-Id",
            fixture.name()
        );
        if text.contains("References:") {
            with_references += 1;
        }
        if text.contains("In-Reply-To:") {
            with_in_reply_to += 1;
        }
    }
    assert!(with_references >= 3, "need well-linked replies");
    assert!(
        with_in_reply_to > with_references,
        "need at least one reply with In-Reply-To and no References — that is \
         the case JWZ threading has to recover from"
    );

    // One reply must have neither, so subject matching is exercised too.
    let orphan = test_corpus::load("list-thread-06-reply-subject-only");
    let text = orphan.text_lossy();
    assert!(!text.contains("References:"));
    assert!(!text.contains("In-Reply-To:"));
    assert!(text.contains("Subject: Re: "));
}

// --------------------------------------------------------- no personal data

/// TLDs that are reserved outright, so anything under them is safe to invent
/// addresses in (RFC 6761).
const RESERVED_TLDS: &[&str] = &["test", "invalid", "example", "localhost"];

/// TLDs under which the second-level name `example` is reserved (RFC 2606).
const RESERVED_UNDER_EXAMPLE: &[&str] = &["com", "net", "org"];

/// Whether `domain` is one nobody can register, and so one the corpus may use.
///
/// This is the same rule `scripts/checks/check-no-personal-data.py` applies to the
/// whole repository, restated here so the corpus cannot drift away from it.
/// Deliberately narrower than "RFC 2606 second-level name": `example.de` reads
/// like a reserved domain and is not one — it is registrable, and an address
/// there could belong to somebody.
fn is_reserved(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    let Some(tld) = labels.last() else {
        return false;
    };
    if RESERVED_TLDS.contains(tld) {
        return true;
    }
    labels.len() >= 2
        && labels[labels.len() - 2] == "example"
        && RESERVED_UNDER_EXAMPLE.contains(tld)
}

#[test]
fn every_address_in_the_corpus_is_an_invented_reserved_domain() {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for fixture in test_corpus::all() {
        let text = fixture.text_lossy();
        let bytes: Vec<char> = text.chars().collect();
        for (i, ch) in bytes.iter().enumerate() {
            if *ch != '@' {
                continue;
            }
            // `@media` in a stylesheet is not an address: a real one has a
            // local part immediately before the `@`.
            let is_local_part_char =
                |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-');
            if i == 0 || !is_local_part_char(bytes[i - 1]) {
                continue;
            }
            let domain: String = bytes[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || **c == '.' || **c == '-')
                .collect();
            let domain = domain.trim_end_matches(['.', '-']).to_ascii_lowercase();
            if domain.is_empty() {
                continue;
            }
            seen.entry(domain).or_insert(fixture.name());
        }
    }
    assert!(!seen.is_empty(), "the corpus should contain addresses");

    for (domain, fixture) in &seen {
        assert!(
            domain.contains('.'),
            "{fixture}: bare domain {domain:?} in an address"
        );
        assert!(
            is_reserved(domain),
            "{fixture}: {domain:?} is registrable, so an address there could \
             belong to somebody — the corpus must never contain a real address"
        );
    }
}

#[test]
fn the_corpus_mentions_nothing_from_a_real_mailbox() {
    // Cheap tripwire against a fixture ever being pasted in from a real inbox
    // or from the developer's own mail configuration.
    const FORBIDDEN: &[&str] = &[
        "himalaya",
        "dlapiduz",
        "gmail.com",
        "icloud.com",
        "me.com",
        "outlook.com",
        "yahoo.com",
        "protonmail",
        "app-specific",
        "/home/",
    ];
    for fixture in test_corpus::all() {
        let text = fixture.text_lossy().to_ascii_lowercase();
        for needle in FORBIDDEN {
            assert!(
                !text.contains(needle),
                "{} contains {needle:?}: the corpus must be entirely invented",
                fixture.name()
            );
        }
    }
}

// ------------------------------------------------------ the parsed convenience

#[test]
fn every_fixture_parses_through_the_convenience_without_panicking() {
    for fixture in test_corpus::all() {
        let message = fixture.parse();
        assert_eq!(message.account_id, AccountId::new(1));
        assert_eq!(message.mailbox_id, MailboxId::new(1));
        assert!(
            !message.is_persisted(),
            "{}: a fixture is never a stored row",
            fixture.name()
        );
    }
}

#[test]
fn the_convenience_agrees_with_parsing_by_hand() {
    let fixture = test_corpus::load("plain-text-simple");

    let message = fixture.parse();
    let by_hand = mime::parse(fixture.bytes()).into_message(
        message.account_id,
        message.mailbox_id,
        message.received_at,
    );

    assert_eq!(message, by_hand);
}

// ------------------------------------------------------------ documentation

#[test]
fn the_readme_documents_the_parse_convenience() {
    let readme = corpus_dir().join("README.md");
    let text = std::fs::read_to_string(&readme)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", readme.display()));
    assert!(
        text.contains("Fixture::parse"),
        "the parsed-message convenience is undocumented in {}",
        readme.display()
    );
}

#[test]
fn the_readme_documents_every_fixture() {
    let readme = corpus_dir().join("README.md");
    let text = std::fs::read_to_string(&readme)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", readme.display()));
    for fixture in test_corpus::all() {
        assert!(
            text.contains(fixture.file_name()),
            "{} is undocumented; describe it in {}",
            fixture.file_name(),
            readme.display()
        );
    }
    for category in Category::ALL {
        assert!(
            text.contains(category.as_str()),
            "category {category} is not mentioned in the corpus README"
        );
    }
}
