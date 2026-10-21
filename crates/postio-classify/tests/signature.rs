//! Reading a signature (specs/011-focus-contacts S9, handoff screen 05): the
//! title, company, phone and website a person signs with, taken from the
//! tail of invented mail, never from the sentences above it. Every address
//! and domain is reserved.

use postio_classify::signature::parse;

fn lines(text: &str) -> String {
    text.trim_start_matches('\n').to_owned()
}

#[test]
fn a_signature_after_the_sign_off_gives_title_company_phone_and_website() {
    let details = parse(&lines(
        "
Hi Diego, I help teams like yours move analytics off the warehouse.

Best,
Jonas Weber
Partnerships \u{b7} Harbor Data Co.
+1 555 0142
harbordata.example
",
    ));
    assert_eq!(details.title.as_deref(), Some("Partnerships"));
    assert_eq!(details.company.as_deref(), Some("Harbor Data Co."));
    assert_eq!(details.phones, ["+1 555 0142"]);
    assert_eq!(details.websites, ["harbordata.example"]);
}

#[test]
fn the_dash_dash_line_starts_a_signature_and_title_comma_company_splits() {
    let details = parse(&lines(
        "
Thanks for sending the draft over.

-- 
Ada Example
Finance lead, Northwind Partners
Tel: (555) 010-0199
https://northwind.example/team
",
    ));
    assert_eq!(details.title.as_deref(), Some("Finance lead"));
    assert_eq!(details.company.as_deref(), Some("Northwind Partners"));
    assert_eq!(details.phones, ["(555) 010-0199"]);
    assert_eq!(details.websites, ["northwind.example/team"]);
}

#[test]
fn title_at_company_and_a_pipe_split_the_same_way() {
    let at = parse("Regards\nMae Sorensen\nHead of Design at Larkspur Outfitters\n");
    assert_eq!(at.title.as_deref(), Some("Head of Design"));
    assert_eq!(at.company.as_deref(), Some("Larkspur Outfitters"));
    let pipe = parse("Cheers\nBen\nSenior Engineer | Crate Labs\nwww.crate.example\n");
    assert_eq!(pipe.title.as_deref(), Some("Senior Engineer"));
    assert_eq!(pipe.company.as_deref(), Some("Crate Labs"));
    assert_eq!(pipe.websites, ["crate.example"]);
}

#[test]
fn a_company_alone_is_known_by_its_suffix() {
    let details = parse("Thanks,\nLena\nHarbor Data Co.\n");
    assert_eq!(details.company.as_deref(), Some("Harbor Data Co."));
    assert_eq!(details.title, None);
}

#[test]
fn a_phone_in_the_body_text_is_not_a_signature_phone() {
    let details = parse(
        "Call me on +1 555 0100 whenever, or ring the office.\n\nSee you Monday.\n\nThanks,\nAda\n",
    );
    assert!(details.phones.is_empty(), "{details:?}");
}

#[test]
fn a_quoted_reply_is_not_read() {
    let details = parse(
        "Sounds good.\n\nBest,\nAda\n\n> On Mon, Jonas Weber wrote:\n> Partnerships \u{b7} Harbor Data Co.\n> +1 555 0142\n",
    );
    assert!(details.is_empty(), "{details:?}");
}

#[test]
fn several_phones_and_sites_keep_their_order_without_repeats() {
    let details = parse(
        "Best,\nGrace\nDirector, Navy Yard\nT +1 555 0111\nM +1 555 0112\nT +1 555 0111\nnavy.example\nhttps://navy.example\n",
    );
    assert_eq!(details.phones, ["+1 555 0111", "+1 555 0112"]);
    assert_eq!(details.websites, ["navy.example"]);
}

#[test]
fn a_plain_message_with_no_signature_says_nothing() {
    assert!(parse("Can you send the numbers by Friday?\nThanks!\n").is_empty());
    assert!(parse("").is_empty());
}

#[test]
fn an_email_address_is_not_a_website_and_a_date_is_not_a_phone() {
    let details = parse("Best,\nAda\nada@example.org\nSent 2026-09-26 13:47\n");
    assert!(details.websites.is_empty(), "{details:?}");
    assert!(details.phones.is_empty(), "{details:?}");
}

#[test]
fn the_name_line_under_the_sign_off_is_the_name_and_its_key_needs_all_three_parts() {
    let details = parse(&lines(
        "
Thanks,
Ada Moreno
Finance lead, Northwind Partners
+1 555 0119
",
    ));
    assert_eq!(details.name.as_deref(), Some("Ada Moreno"));
    assert_eq!(
        details.key().as_deref(),
        Some("ada moreno|finance lead|northwind partners")
    );

    // A first name alone, or a line that is not a name, is no name.
    assert_eq!(
        parse("Best,\nAda\nFinance lead, Northwind Partners\n").name,
        None
    );
    assert_eq!(parse("Best,\nada@example.org\n+1 555 0119\n").name, None);

    // Without a title and a company there is no key: it would match
    // colleagues and strangers.
    let named = parse("Best,\nAda Moreno\n+1 555 0119\n");
    assert_eq!(named.name.as_deref(), Some("Ada Moreno"));
    assert_eq!(named.key(), None);
}
