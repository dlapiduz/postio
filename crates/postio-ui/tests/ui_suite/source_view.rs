use postio_model::test_corpus;
use postio_model::{EmailAddress, headers};
use postio_ui::source::{self, BodyState, Provider, RawState, Section, SourceInput, SourceView};

const PROVIDER: Provider<'static> = Provider {
    name: "Example Mail",
    authserv_ids: &["mx.example.net"],
};

fn header_text(fixture: &str) -> String {
    headers::block_of(test_corpus::load(fixture).bytes())
        .expect("a header block")
        .text
}

fn sender(fixture: &str) -> Vec<EmailAddress> {
    test_corpus::load(fixture).parse().from
}

fn view_of(fixture: &str, provider: Option<Provider<'_>>) -> SourceView {
    let text = header_text(fixture);
    let from = sender(fixture);
    source::view(SourceInput {
        subject: "Notes",
        from: &from,
        headers: Some(&text),
        envelope: None,
        raw: Some(test_corpus::load(fixture).bytes().to_vec()),
        raw_state: RawState::Here,
        body: BodyState::Here,
        provider,
        sent_by_you: false,
        account: "Work",
        signature: None,
    })
}

fn section<'a>(view: &'a SourceView, title: &str) -> &'a Section {
    view.verification
        .sections
        .iter()
        .find(|section| section.title == title)
        .unwrap_or_else(|| panic!("no {title:?} section in {:#?}", view.verification))
}

fn texts(section: &Section) -> Vec<&str> {
    section
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect()
}

const PROVIDER_CHECKED: &str = "What your provider checked";
const NOT_BELIEVED: &str = "Headers Postio does not believe";
const LOCAL: &str = "What Postio checked on this computer";
const LIMITS: &str = "What this does not tell you";

#[test]
fn the_provider_is_named_and_each_result_says_its_domain() {
    let view = view_of("auth-results-verified", Some(PROVIDER));
    assert_eq!(view.title, "Source · Notes");
    let provider = section(&view, PROVIDER_CHECKED);
    assert_eq!(
        provider.intro.as_deref(),
        Some("Checked by Example Mail when the message arrived.")
    );
    assert_eq!(provider.mono.as_deref(), Some("mx.example.net"));
    assert_eq!(
        texts(provider),
        [
            "DKIM passed for example.org, which matches the From domain.",
            "SPF passed for example.org, which is the From domain.",
            "DMARC passed for example.org (policy: reject).",
        ]
    );
}

#[test]
fn a_second_dkim_signature_and_a_sending_service_are_told_apart() {
    let view = view_of("auth-results-two-dkim", Some(PROVIDER));
    assert_eq!(
        texts(section(&view, PROVIDER_CHECKED)),
        [
            "DKIM: 2 signatures. example.com passed and matches the From domain. esp.example passed and is not the From domain.",
            "SPF passed for bounce.esp.example, which is not the From domain.",
            "DMARC passed for example.com (policy: reject).",
        ]
    );
}

#[test]
fn a_list_that_re_signed_names_its_domain_and_the_arc_chain() {
    let view = view_of("auth-results-arc-list", Some(PROVIDER));
    let provider = texts(section(&view, PROVIDER_CHECKED));
    assert_eq!(
        provider,
        [
            "DKIM: 2 signatures. lists.example.org passed and is not the From domain. example.com failed and matches the From domain.",
            "SPF passed for lists.example.org, which is not the From domain.",
            "DMARC failed for example.com (policy: none).",
            "ARC: a forwarder, lists.example.org, sealed the results it saw. Your provider says the chain is intact.",
        ]
    );
    let doubted = section(&view, NOT_BELIEVED);
    assert!(doubted.quiet, "not-believed headers are drawn quieter");
    assert_eq!(
        doubted.intro.as_deref(),
        Some("Written before your provider received the message. Anyone could have written these.")
    );
    assert_eq!(
        doubted
            .lines
            .iter()
            .map(|line| (line.text.as_str(), line.mono.as_deref()))
            .collect::<Vec<_>>(),
        [(
            "ARC set 1: DKIM passed for example.com, the From domain. SPF passed for example.com, the From domain. DMARC passed for example.com.",
            Some("mx.lists.example.org")
        )]
    );
}

#[test]
fn a_forged_header_below_the_providers_is_listed_and_changes_nothing() {
    let view = view_of("auth-results-forged-lower", Some(PROVIDER));
    let provider = texts(section(&view, PROVIDER_CHECKED));
    assert!(
        provider.iter().any(|line| line.starts_with("DMARC failed")),
        "{provider:?}"
    );
    assert!(
        provider.iter().all(|line| !line.contains("passed")),
        "{provider:?}"
    );
    let doubted = section(&view, NOT_BELIEVED);
    assert_eq!(doubted.lines.len(), 1);
    assert!(
        doubted.lines[0].text.contains("DMARC passed"),
        "{doubted:?}"
    );
}

#[test]
fn no_trusted_provider_header_says_so_and_doubts_every_header() {
    let view = view_of("auth-results-verified", None);
    let provider = section(&view, PROVIDER_CHECKED);
    assert_eq!(
        provider.intro.as_deref(),
        Some("Your provider didn't add a result Postio can trust.")
    );
    assert!(provider.lines.is_empty());
    let doubted = section(&view, NOT_BELIEVED);
    assert_eq!(doubted.lines.len(), 1);
    assert_eq!(
        doubted.intro.as_deref(),
        Some("Postio can't tell which of these your provider wrote, so it believes none of them."),
        "with no trusted header, no header is known to predate the provider"
    );
}

#[test]
fn what_postio_checked_says_dkim_was_not_checked_here() {
    let view = view_of("auth-results-verified", Some(PROVIDER));
    assert_eq!(
        texts(section(&view, LOCAL)),
        [
            "The display name is not another address or domain.",
            "DKIM, SPF and DMARC were not checked on this computer. Your provider checked them when the mail arrived.",
        ]
    );
    let impersonating = view_of("display-name-impersonation", Some(PROVIDER));
    let local = texts(section(&impersonating, LOCAL));
    assert!(local[0].starts_with("The name says "), "{local:?}");
}

#[test]
fn the_fixed_line_closes_every_verification_tab() {
    for fixture in [
        "auth-results-verified",
        "plain-text-simple",
        "pgp-signed-good",
    ] {
        let view = view_of(fixture, Some(PROVIDER));
        let last = view.verification.sections.last().expect("a section");
        assert_eq!(last.title, LIMITS, "{fixture}");
        assert_eq!(
            texts(last),
            [
                "These checks are about the sending domain and the message's integrity. They don't tell you the person is who you think."
            ]
        );
    }
}

#[test]
fn a_message_without_its_body_says_the_checks_arrive_with_it() {
    let from = sender("auth-results-verified");
    let view = source::view(SourceInput {
        subject: "Notes",
        from: &from,
        headers: None,
        envelope: Some("From: ada@example.org\r\nSubject: Notes"),
        raw: None,
        raw_state: RawState::Offline,
        body: BodyState::Partial,
        provider: Some(PROVIDER),
        sent_by_you: false,
        account: "Work",
        signature: None,
    });
    assert_eq!(
        section(&view, PROVIDER_CHECKED).intro.as_deref(),
        Some("The headers that carry these checks arrive with the message body.")
    );
    assert_eq!(
        view.raw.state_line.as_deref(),
        Some(
            "This message isn't on this computer yet. Postio will show it when Work is back online."
        )
    );
    assert_eq!(
        view.headers.note.as_deref(),
        Some("More headers arrive with the body.")
    );
}

#[test]
fn a_raw_fetch_in_flight_names_the_account() {
    let from = sender("auth-results-verified");
    let text = header_text("auth-results-verified");
    let view = source::view(SourceInput {
        subject: "Notes",
        from: &from,
        headers: Some(&text),
        envelope: None,
        raw: None,
        raw_state: RawState::Fetching,
        body: BodyState::Here,
        provider: Some(PROVIDER),
        sent_by_you: false,
        account: "Work",
        signature: None,
    });
    assert_eq!(
        view.raw.state_line.as_deref(),
        Some("Fetching the message from Work…")
    );
    assert!(
        !section(&view, PROVIDER_CHECKED).lines.is_empty(),
        "the stored headers still answer the Verification tab"
    );
}

#[test]
fn mail_the_person_sent_is_not_judged() {
    let from = sender("auth-results-forged-lower");
    let text = header_text("auth-results-forged-lower");
    let view = source::view(SourceInput {
        subject: "Notes",
        from: &from,
        headers: Some(&text),
        envelope: None,
        raw: None,
        raw_state: RawState::Here,
        body: BodyState::Here,
        provider: Some(PROVIDER),
        sent_by_you: true,
        account: "Work",
        signature: None,
    });
    let titles: Vec<&str> = view
        .verification
        .sections
        .iter()
        .map(|section| section.title.as_str())
        .collect();
    assert_eq!(titles, ["You sent this", LIMITS]);
}

fn headers_of(fixture: &str, provider: Option<Provider<'_>>) -> source::HeadersView {
    view_of(fixture, provider).headers
}

#[test]
fn headers_are_unfolded_and_decoded_with_the_written_form_kept() {
    let rows = headers_of("encoded-word-subject-and-names", Some(PROVIDER)).rows;
    let row = |name: &str| {
        rows.iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("no {name} row"))
    };
    let subject = row("Subject");
    assert!(
        subject.value.starts_with("Réunion du comité: "),
        "{:?}",
        subject.value
    );
    assert!(
        subject
            .raw
            .as_deref()
            .is_some_and(|raw| raw.starts_with("=?utf-8?Q?R=C3=A9union")),
        "{subject:?}"
    );
    assert_eq!(
        row("From").value,
        "Françoise Lemaître <f.lemaitre@fr.example>"
    );
    assert_eq!(row("Date").raw, None, "decoding changed nothing");
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        &names[..4],
        ["Message-ID", "Date", "From", "To"],
        "the message's own order"
    );
    for row in &rows {
        assert!(
            !row.value.contains(['\r', '\n', '\t']),
            "{} is still folded",
            row.name
        );
    }
}

#[test]
fn received_headers_are_hops_newest_first_with_their_delays() {
    let hops = headers_of("header-folding-received-chain", Some(PROVIDER)).hops;
    let seen: Vec<(Option<&str>, Option<&str>, Option<&str>)> = hops
        .iter()
        .map(|hop| (hop.from.as_deref(), hop.by.as_deref(), hop.delay.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            (
                Some("mx4.example.net"),
                Some("imap.example.net"),
                Some("3 s")
            ),
            (
                Some("relay-b.notify.example.org"),
                Some("mx4.example.net"),
                Some("2 s")
            ),
            (None, Some("relay-b.notify.example.org"), None),
        ]
    );
    assert_eq!(
        hops[0].at.as_deref(),
        Some("Thu, 12 Feb 2026 06:12:44 +0000")
    );
}

#[test]
fn the_trust_boundary_sits_under_the_providers_header() {
    let provider = Provider {
        name: "Example Mail",
        authserv_ids: &["mx4.example.net"],
    };
    let rows = headers_of("header-folding-received-chain", Some(provider)).rows;
    let marked: Vec<(usize, &str, &str)> = rows
        .iter()
        .enumerate()
        .filter_map(|(at, row)| {
            row.boundary_above
                .as_deref()
                .map(|line| (at, row.name.as_str(), line))
        })
        .collect();
    assert_eq!(
        marked,
        [(
            5,
            "DKIM-Signature",
            "Above: written by Example Mail. Below: came with the message."
        )]
    );
    assert!(
        headers_of("header-folding-received-chain", None)
            .rows
            .iter()
            .all(|row| row.boundary_above.is_none()),
        "no trusted header, no boundary"
    );
}

#[test]
fn the_raw_tab_folds_a_large_part_and_loses_no_byte() {
    let raw = test_corpus::load("attachment-large").bytes();
    let view = view_of("attachment-large", Some(PROVIDER)).raw;
    let folds: Vec<(&str, &std::ops::Range<usize>)> = view
        .segments
        .iter()
        .filter_map(|segment| match segment {
            source::RawSegment::Fold { label, range } => Some((label.as_str(), range)),
            source::RawSegment::Text(_) => None,
        })
        .collect();
    assert_eq!(folds.len(), 1, "{folds:?}");
    let (label, range) = folds[0];
    assert!(range.len() > 64 * 1024);
    assert!(
        label.starts_with("base64 · ") && label.ends_with(" KB · site-survey-full.bin"),
        "{label}"
    );
    let mut rebuilt = Vec::new();
    for segment in &view.segments {
        match segment {
            source::RawSegment::Text(text) => rebuilt.extend_from_slice(text.as_bytes()),
            source::RawSegment::Fold { range, .. } => {
                rebuilt.extend_from_slice(&raw[range.clone()])
            }
        }
    }
    assert_eq!(
        rebuilt, raw,
        "the folds and the text are every byte, in order"
    );

    let small = view_of("plain-text-simple", Some(PROVIDER)).raw;
    assert_eq!(
        small.segments,
        [source::RawSegment::Text(
            test_corpus::load("plain-text-simple")
                .text_lossy()
                .into_owned()
        )]
    );
}

#[test]
fn save_offers_the_subject_as_a_file_name() {
    assert_eq!(source::eml_name("Budget"), "Budget.eml");
    assert_eq!(
        source::eml_name("Re: Q3/Q4 numbers?"),
        "Re Q3 Q4 numbers.eml"
    );
    assert_eq!(source::eml_name("../../etc"), "etc.eml");
    assert_eq!(source::eml_name("  "), "message.eml");
}

#[test]
fn each_tab_reads_as_text_for_a_terminal() {
    let view = view_of("attachment-large", Some(PROVIDER));
    let verification = source::text(&view, source::Tab::Verification);
    assert!(
        verification.starts_with("What your provider checked\n"),
        "{verification}"
    );
    assert!(verification.contains(source::FIXED_LINE));
    let headers = source::text(&view, source::Tab::Headers);
    assert!(
        headers.contains("Content-Type: multipart/mixed"),
        "{headers}"
    );
    let raw = source::text(&view, source::Tab::Raw);
    assert!(
        raw.contains("[base64 · "),
        "the fold is one line: {}",
        &raw[..raw.len().min(400)]
    );
    assert!(raw.len() < 64 * 1024, "the folded part is not in the text");
}

#[test]
fn a_body_without_its_headers_on_an_account_that_is_not_syncing_says_so_on_every_tab() {
    let from = sender("auth-results-verified");
    let view = source::view(SourceInput {
        subject: "Notes",
        from: &from,
        headers: None,
        envelope: Some("From: Ada Example <ada@example.org>\r\nSubject: Notes"),
        raw: None,
        raw_state: RawState::Unreachable,
        body: BodyState::Here,
        provider: Some(PROVIDER),
        sent_by_you: false,
        account: "Work",
        signature: None,
    });
    assert_eq!(
        section(&view, PROVIDER_CHECKED).intro.as_deref(),
        Some(
            "The headers that carry these checks aren't on this computer, and Work isn't syncing."
        )
    );
    let names: Vec<&str> = view
        .headers
        .rows
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["From", "Subject"],
        "the envelope, never a blank tab"
    );
    assert_eq!(
        view.headers.note.as_deref(),
        Some("More headers arrive with the body.")
    );
    assert_eq!(
        view.raw.state_line.as_deref(),
        Some(
            "This message isn't on this computer, and Work isn't syncing, so Postio can't fetch it."
        )
    );
}

#[test]
fn the_footer_teaches_the_surfaces_own_keys() {
    let footer = source::footer(postio_core::Keymap::defaults());
    assert_eq!(footer.title, "Source and verification");
    let labels: Vec<&str> = footer
        .keys
        .iter()
        .map(|(_, label)| label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "verification",
            "headers",
            "raw",
            "copy all",
            "save as .eml",
            "close"
        ]
    );
    assert_eq!(footer.keys[0].0, "1");
}

fn signed_view(signature: source::SignatureInput) -> SourceView {
    let from = vec![EmailAddress::new(Some("Ada Moreno"), "ada@example.com")];
    source::view(SourceInput {
        subject: "Rota",
        from: &from,
        headers: None,
        envelope: Some("From: Ada Moreno <ada@example.com>"),
        raw: None,
        raw_state: RawState::Here,
        body: BodyState::Here,
        provider: None,
        sent_by_you: false,
        account: "Work",
        signature: Some(signature),
    })
}

fn good() -> source::SignatureInput {
    source::SignatureInput {
        status: "good".to_owned(),
        reason: None,
        scope: "whole".to_owned(),
        fingerprint: Some("E8DF9987513ECF47E8487A06199C11946D36705D".to_owned()),
        sources: vec!["autocrypt".to_owned()],
        first_seen: Some(1_772_528_400),
        accepted_at: None,
        address: "ada@example.com".to_owned(),
    }
}

const SIGNATURE: &str = "Signature";

#[test]
fn a_good_signature_says_whose_key_where_it_came_from_and_what_it_does_not_prove() {
    let view = signed_view(good());
    let signature = section(&view, SIGNATURE);
    assert_eq!(
        signature.intro.as_deref(),
        Some(
            "Good signature by a key from ada@example.com's Autocrypt header, first seen 3 March 2026."
        )
    );
    assert_eq!(
        signature.mono.as_deref(),
        Some("E8DF 9987 513E CF47 E848 7A06 199C 1194 6D36 705D")
    );
    assert_eq!(
        texts(signature),
        [
            "Signed means this message hasn't changed since this key signed it. It doesn't tell you who holds that key.",
            "Accept this key for ada@example.com",
        ]
    );
    let local = texts(section(&view, LOCAL));
    assert!(
        local.contains(&"The signature was checked on this computer."),
        "{local:?}"
    );
}

#[test]
fn an_accepted_key_says_when_and_offers_nothing_more() {
    let view = signed_view(source::SignatureInput {
        accepted_at: Some(1_772_701_200),
        ..good()
    });
    let lines = texts(section(&view, SIGNATURE));
    assert!(
        lines.contains(&"You accepted this key for ada@example.com on 5 March 2026."),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.starts_with("Accept this key")),
        "{lines:?}"
    );
}

#[test]
fn every_other_verdict_says_why_and_never_good() {
    for (status, reason, scope, said) in [
        (
            "bad",
            Some("tampered"),
            "whole",
            "Bad signature: the message changed after it was signed.",
        ),
        (
            "cant_check",
            Some("no_key"),
            "whole",
            "Can't check: no key for this signature is on this computer.",
        ),
        (
            "cant_check",
            Some("weak_hash:sha1"),
            "whole",
            "Can't check: this signature uses SHA-1, which is no longer safe.",
        ),
        (
            "not_here_yet",
            None,
            "whole",
            "The signed message isn't fully on this computer yet; Postio will check the signature when it is.",
        ),
        (
            "revoked",
            None,
            "whole",
            "Revoked key: the key that made this signature has been revoked.",
        ),
        (
            "good",
            None,
            "part",
            "Part of this message is signed: a list or a forward added text the signature doesn't cover.",
        ),
    ] {
        let view = signed_view(source::SignatureInput {
            status: status.to_owned(),
            reason: reason.map(str::to_owned),
            scope: scope.to_owned(),
            ..good()
        });
        let intro = section(&view, SIGNATURE).intro.clone().unwrap_or_default();
        assert_eq!(intro, said, "{status} {reason:?} {scope}");
        assert!(!intro.contains("Good"), "{status}: never Good");
    }
}

#[test]
fn a_key_from_the_gnupg_keyring_is_signed_until_accepted_whatever_gnupg_trusts() {
    let view = signed_view(source::SignatureInput {
        sources: vec!["gnupg".to_owned()],
        ..good()
    });
    let signature = section(&view, SIGNATURE);
    assert_eq!(
        signature.intro.as_deref(),
        Some("Good signature by a key from your GnuPG keyring, first seen 3 March 2026.")
    );
    assert!(
        texts(signature).contains(&"Accept this key for ada@example.com"),
        "{:?}",
        texts(signature)
    );
    let header = postio_ui::reader::authenticity::header(
        &postio_model::Headers::default(),
        &[postio_model::EmailAddress::new(
            None::<String>,
            "ada@example.com",
        )],
        &[] as &[&str],
        Some(&postio_ui::reader::authenticity::Signature {
            status: "good".to_owned(),
            scope: "whole".to_owned(),
            signer: good().fingerprint,
            accepted: None,
        }),
    );
    assert_eq!(
        header.mark,
        Some(postio_ui::reader::authenticity::Mark::Signed),
        "never Signed · accepted by itself"
    );
}

#[test]
fn an_encrypted_message_is_said_and_a_signature_part_is_no_attachment() {
    use postio_ui::reader::{document, parts};
    for kind in [
        "multipart/encrypted",
        "Multipart/Encrypted",
        "application/pkcs7-mime",
    ] {
        assert!(document::is_encrypted(kind), "{kind}");
    }
    for kind in ["multipart/signed", "text/plain", "multipart/mixed"] {
        assert!(!document::is_encrypted(kind), "{kind}");
    }
    assert_eq!(
        document::ENCRYPTED,
        "This message is encrypted. Postio can't decrypt mail yet."
    );
    for mime in [
        "application/pgp-signature",
        "application/pkcs7-signature",
        "application/x-pkcs7-signature",
        "application/pgp-encrypted",
    ] {
        assert!(!parts::listed_as_attachment(mime), "{mime}");
    }
    assert!(parts::listed_as_attachment("application/pdf"));
    assert!(
        parts::listed_as_attachment("application/pgp-keys"),
        "a key is offered for import"
    );
}

#[test]
fn settings_says_what_the_gnupg_keyring_gave() {
    use postio_ui::settings::{GnupgKeyring, gnupg_line};
    assert_eq!(
        gnupg_line(&GnupgKeyring::Read {
            keys: 312,
            skipped: 2
        }),
        "312 keys read from your GnuPG keyring · 2 skipped"
    );
    assert_eq!(
        gnupg_line(&GnupgKeyring::Read {
            keys: 1,
            skipped: 0
        }),
        "1 key read from your GnuPG keyring"
    );
    assert_eq!(
        gnupg_line(&GnupgKeyring::Unreadable),
        "Postio can't read this GnuPG keyring. Export the keys you need with `gpg --export --armor <address> > keys.asc` and import the file."
    );
    assert_eq!(
        gnupg_line(&GnupgKeyring::Absent {
            home: "/home/ada/.gnupg".to_owned()
        }),
        "No GnuPG keyring was found in /home/ada/.gnupg."
    );
    assert_eq!(
        gnupg_line(&GnupgKeyring::Off),
        "Off: Postio reads no file outside its own store."
    );
}
