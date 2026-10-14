//! The one write: a draft, local, and never sent (ADR 0010 Q4).

use postio_model::Operation;
use serde_json::json;

use crate::harness::{INBOX_ONLY, World};

fn sends(world: &World) -> usize {
    world
        .queued()
        .iter()
        .filter(|operation| matches!(operation, Operation::Send { .. }))
        .count()
}

#[test]
fn a_new_draft_is_saved_and_nothing_is_sent() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call(
        "create_draft",
        json!({"to": ["grace@example.org"], "subject": "Lunch", "body": "Are you free Thursday?"}),
    );
    assert!(!result.is_error(), "{}", result.0);

    let drafts = world.drafts();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let draft = &drafts[0];
    assert_eq!(draft.account_id, world.ada);
    assert_eq!(draft.to[0].address, "grace@example.org");
    assert_eq!(draft.subject, "Lunch");
    assert!(
        draft
            .body
            .text
            .as_deref()
            .is_some_and(|text| text.contains("Are you free Thursday?")),
        "{draft:?}"
    );
    assert_eq!(result.mail()["draft"], json!(draft.id.get()));
    assert!(
        result.notice().contains("not been sent"),
        "{}",
        result.notice()
    );
    assert_eq!(sends(&world), 0);
    // The queue is read for real: the draft's copy for the server's Drafts
    // folder is in it, as the composer's autosave puts one there.
    assert!(
        world
            .queued()
            .iter()
            .any(|operation| matches!(operation, Operation::SaveDraft { .. })),
        "{:?}",
        world.queued()
    );
}

#[test]
fn a_reply_goes_to_the_sender_and_quotes_the_original() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call(
        "create_draft",
        json!({"reply_to": world.budget.get(), "body": "Friday works for me."}),
    );
    assert!(!result.is_error(), "{}", result.0);

    let drafts = world.drafts();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let draft = &drafts[0];
    assert_eq!(draft.in_reply_to, Some(world.budget));
    assert_eq!(draft.to[0].address, "grace@example.org");
    assert_eq!(draft.subject, "Re: Atlas budget");
    let text = draft.body.text.as_deref().expect("a text body");
    let own = text
        .find("Friday works for me.")
        .expect("the agent's words");
    let quoted = text
        .find("> The atlas budget is due Friday.")
        .expect("the quote");
    assert!(own < quoted, "{text}");
    assert_eq!(sends(&world), 0);
}

#[test]
fn the_agent_s_words_are_text_and_never_markup() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call(
        "create_draft",
        json!({
            "reply_to": world.html.get(),
            "body": "<script>steal()</script><img src=\"https://tracker.example.net/x.gif\">",
        }),
    );
    assert!(!result.is_error(), "{}", result.0);
    let draft = &world.drafts()[0];
    let html = draft.body.html.as_deref().unwrap_or_default();
    assert!(!html.contains("<script"), "{html}");
    assert!(!html.contains("<img"), "{html}");
}

#[test]
fn a_draft_cannot_reply_to_mail_nobody_shared() {
    let mut world = World::new(INBOX_ONLY);
    for source in [world.archived, world.elsewhere] {
        let result = world.call(
            "create_draft",
            json!({"reply_to": source.get(), "body": "Thanks"}),
        );
        assert!(result.is_error(), "{}", result.0);
        let forwarded = world.call(
            "create_draft",
            json!({"forward": source.get(), "to": ["grace@example.org"], "body": "FYI"}),
        );
        assert!(forwarded.is_error(), "{}", forwarded.0);
    }
    assert!(world.drafts().is_empty(), "{:?}", world.drafts());
}

#[test]
fn a_draft_cannot_be_written_from_an_account_nobody_shared() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call(
        "create_draft",
        json!({"account": world.other.get(), "to": ["grace@example.org"], "body": "Hello"}),
    );
    assert!(result.is_error(), "{}", result.0);
    assert!(world.drafts().is_empty());
}

#[test]
fn a_forward_carries_the_original_below_the_agent_s_words() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call(
        "create_draft",
        json!({"forward": world.budget.get(), "to": ["ben@example.net"], "body": "For your files."}),
    );
    assert!(!result.is_error(), "{}", result.0);
    let draft = &world.drafts()[0];
    assert_eq!(draft.to[0].address, "ben@example.net");
    assert_eq!(draft.subject, "Fwd: Atlas budget");
    let text = draft.body.text.as_deref().expect("a body");
    assert!(text.contains("For your files."), "{text}");
    assert!(text.contains("The atlas budget is due Friday."), "{text}");
    assert_eq!(sends(&world), 0);
}

#[test]
fn an_address_that_is_not_one_is_refused() {
    let mut world = World::new(INBOX_ONLY);
    let result = world.call(
        "create_draft",
        json!({"to": ["not an address"], "body": "Hi"}),
    );
    assert!(result.is_error(), "{}", result.0);
    assert!(world.drafts().is_empty());
}
