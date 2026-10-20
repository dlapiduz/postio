//! What the reader says about who a message really came from (#1928): read
//! the way every frontend does, through the model's parser and the corpus.

use postio_model::mime;
use postio_model::test_corpus;
use postio_ui::reader::authenticity::{self, Verdict};

const MINE: [&str; 1] = ["mx.example.net"];

fn read(name: &str) -> mime::ParsedMessage {
    mime::parse(test_corpus::load(name).bytes())
}

fn verdict_of(name: &str, trusted: &[&str]) -> Verdict {
    let parsed = read(name);
    authenticity::verdict(&parsed.headers, &parsed.from, trusted)
}

#[test]
fn a_message_the_provider_vouched_for_is_verified_and_says_nothing() {
    let parsed = read("auth-results-verified");
    assert_eq!(
        verdict_of("auth-results-verified", &MINE),
        Verdict::Verified
    );
    assert_eq!(
        authenticity::warning(&parsed.headers, &parsed.from, &MINE),
        None
    );
}

#[test]
fn a_passing_header_the_sender_wrote_below_the_providers_failure_is_ignored() {
    let parsed = read("auth-results-forged-lower");
    assert_eq!(
        parsed.headers.get_all("Authentication-Results").len(),
        2,
        "the fixture has both headers"
    );
    assert_eq!(
        verdict_of("auth-results-forged-lower", &MINE),
        Verdict::Failed
    );
    let said = authenticity::warning(&parsed.headers, &parsed.from, &MINE)
        .expect("a failed message warns");
    assert!(said.contains("example.org"), "names the domain: {said}");
}

#[test]
fn a_header_from_a_server_that_is_not_the_providers_is_not_trusted() {
    assert_eq!(
        verdict_of("auth-results-forged-lower", &["mx.example.com"]),
        Verdict::Unknown
    );
    assert_eq!(
        verdict_of("auth-results-forged-lower", &[]),
        Verdict::Unknown
    );
}

#[test]
fn a_name_that_is_another_address_warns_even_when_the_message_authenticates() {
    let parsed = read("display-name-impersonation");
    assert_eq!(
        verdict_of("display-name-impersonation", &MINE),
        Verdict::Verified
    );
    let said = authenticity::warning(&parsed.headers, &parsed.from, &MINE)
        .expect("an impersonation warns");
    assert!(said.contains("billing@bank.example"), "{said}");
    assert!(said.contains("billing-desk@other.example.com"), "{said}");
}

#[test]
fn ordinary_mail_with_no_authentication_header_says_nothing() {
    let parsed = read("plain-text-simple");
    assert_eq!(
        authenticity::warning(&parsed.headers, &parsed.from, &MINE),
        None
    );
}
