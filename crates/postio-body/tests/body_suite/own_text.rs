use postio_body::own_text;
use postio_model::{MessageBody, mime, test_corpus};

fn body_of(name: &str) -> MessageBody {
    mime::parse(test_corpus::load(name).bytes()).body
}

#[test]
fn a_top_posted_reply_is_its_answer_without_the_history_or_the_signature() {
    let body = body_of("top-posted-reply-signature");
    let expected = "Hi Ada,\n\nThanks for sending the lease over. Could you initial page \
                    four as well? The agent needs it back by Friday.";
    assert_eq!(own_text(&body), expected);
    assert_eq!(own_text(&body), own_text(&body));
}

#[test]
fn a_plain_reply_leaves_out_a_signature_written_above_its_quote() {
    // As a text-only client writes it: the signature sits above the quoted history, and the history
    // is left out first, so here it is a signature.
    let plain = MessageBody {
        html: None,
        text: body_of("top-posted-reply-signature").text,
    };
    assert_eq!(
        own_text(&plain),
        "Hi Ada,\n\nThanks for sending the lease over. Could you initial page four as well?\n\
         The agent needs it back by Friday."
    );
}

#[test]
fn a_bottom_posted_reply_is_what_follows_its_quote() {
    let own = own_text(&body_of("plain-text-flowed-reply"));
    for said in [
        "That order works for me",
        "Keeping the second reader.",
        "smaller than we assumed.",
    ] {
        assert!(own.contains(said), "{said:?} is missing from {own:?}");
    }
    assert!(
        !own.contains("Short note before the walkthrough"),
        "the quoted history was read: {own:?}"
    );
    assert!(
        !own.lines().any(|line| line.trim_start().starts_with('>')),
        "a quoted line was read: {own:?}"
    );
}

#[test]
fn a_signature_is_not_the_message() {
    let own = own_text(&body_of("plain-text-simple"));
    assert!(
        own.ends_with("Nothing else needs to happen this week."),
        "{own:?}"
    );
    assert!(!own.contains("-- "), "{own:?}");
}

#[test]
fn an_html_message_is_read_without_its_title_or_hidden_preheader() {
    let own = own_text(&body_of("html-transactional-receipt"));
    for said in [
        "Order 4410, placed 21 September 2026.",
        "Field Guide to Lichens",
        "Questions? Reply to this message or visit our help pages.",
    ] {
        assert!(own.contains(said), "{said:?} is missing from {own:?}");
    }
    assert!(
        !own.contains("Receipt 4410"),
        "the <title> was read: {own:?}"
    );
    assert!(
        !own.contains("Preheader"),
        "the hidden preheader was read: {own:?}"
    );
}

#[test]
fn an_image_is_not_read_as_its_alt_text() {
    let own = own_text(&body_of("html-legacy-table-attrs"));
    assert!(own.contains("Rota posted on the shed door."), "{own:?}");
    assert!(
        !own.contains("Brookside badge"),
        "an image's alt text was read: {own:?}"
    );
}
