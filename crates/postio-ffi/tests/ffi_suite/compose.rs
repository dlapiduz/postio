//! Writing mail across the boundary (#1272).
//!
//! The macOS frontend could read mail and not answer it: `postio-ffi` had no
//! composer at all, so `e` resolved to a command with nothing on the other
//! side. What crosses is a **draft**, not an editor — the rules about who a
//! reply is addressed to, what its subject becomes and what a quote looks
//! like are `postio_model::reply`'s, shared with the frontend that already
//! had them, and a second implementation of "reply-all excludes me" is
//! exactly the drift ADR 0019 Q6 is about.

use chrono::Utc;
use postio_core::bridge::Bridge;
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_ffi::{Session, SessionOptions};
use postio_model::{DraftState, EmailAddress, Message};
use postio_storage::repository::{DraftRepository, MessageRepository};
use postio_storage::test_support;

/// A session over a store holding one message from Ada, addressed to the
/// account and to Bo.
fn a_message_to_answer() -> (std::sync::Arc<Session>, postio_storage::Database, i64) {
    let database = test_support::memory();
    let message = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.subject = Some("Radon reduction".to_owned());
        message.from = vec![EmailAddress::new(Some("Ada Norwood"), "ada@example.com")];
        message.to = vec![
            account.address.clone(),
            EmailAddress::new(Some("Bo Ferris"), "bo@example.com"),
        ];
        let repository = MessageRepository::new(&connection);
        let id = repository.create(&mut message).expect("a message");
        // The body is a column of its own (ADR 0020), written separately —
        // which is exactly why a reply that reads the row alone quotes
        // nothing.
        repository
            .set_body(
                id,
                &postio_storage::repository::StoredBody {
                    text: Some("The gate closes at six.".to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::message::BodyState::Full,
            )
            .expect("the body is stored");
        id.get()
    };

    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let state = SharedState::default();
    let bus = postio_session::actions::wire(
        Dispatcher::builder(),
        postio_session::actions::Actions::new(database.clone(), state),
    )
    .build();
    let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
    let bridge = Box::leak(Box::new(bridge));
    let session = Session::open(
        SessionOptions::in_memory_with(database.clone())
            .with_blobs_for_test(blobs, scratch)
            .on_bridge(bridge.handle(), bridge.commands()),
    )
    .expect("a session over the store");
    (session, database, message)
}

#[test]
fn a_new_message_starts_empty_and_knows_which_account_it_is_from() {
    let (session, _, _) = a_message_to_answer();
    let draft = session.new_draft().expect("an account to write from");

    assert!(draft.to.is_empty());
    assert!(draft.subject.is_empty());
    assert!(draft.body.is_empty());
    assert!(
        !draft.from.is_empty(),
        "a composer with no From is one nobody can send from"
    );
}

#[test]
fn a_reply_goes_to_the_sender_and_quotes_what_was_said() {
    let (session, _, message) = a_message_to_answer();
    let draft = session.reply_draft(message, false).expect("a reply");

    assert_eq!(draft.to, "Ada Norwood <ada@example.com>");
    assert!(draft.cc.is_empty(), "a plain reply does not copy the room");
    assert_eq!(draft.subject, "Re: Radon reduction");
    assert!(
        draft.body.contains("> The gate closes at six."),
        "the quote is what makes a reply a reply: {}",
        draft.body
    );
    assert_eq!(draft.in_reply_to, Some(message));
}

#[test]
fn a_reply_to_all_keeps_the_others_and_leaves_me_out() {
    let (session, _, message) = a_message_to_answer();
    let draft = session.reply_draft(message, true).expect("a reply");

    assert_eq!(draft.to, "Ada Norwood <ada@example.com>");
    assert!(
        draft.cc.contains("bo@example.com"),
        "the rest of the conversation stays on it: {}",
        draft.cc
    );
    assert!(
        !draft.cc.contains(&draft.from),
        "and I am not on my own reply: {} in {}",
        draft.from,
        draft.cc
    );
}

#[test]
fn a_forward_carries_the_message_and_asks_who_to_send_it_to() {
    let (session, _, message) = a_message_to_answer();
    let draft = session.forward_draft(message).expect("a forward");

    assert!(draft.to.is_empty(), "a forward has nobody until you say so");
    assert_eq!(draft.subject, "Fwd: Radon reduction");
    assert!(draft.body.contains("The gate closes at six."));
}

#[test]
fn saving_a_draft_puts_it_in_the_store_and_gives_it_an_id() {
    let (session, database, _) = a_message_to_answer();
    let mut draft = session.new_draft().expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.body = "Six is fine.".to_owned();

    let saved = session.save_draft(draft).expect("a saved draft");
    assert!(
        saved.id > 0,
        "a saved draft has an id to be saved *again* by"
    );

    let connection = database.connection().expect("a connection");
    let stored = DraftRepository::new(&connection)
        .get(postio_model::ids::DraftId::new(saved.id))
        .expect("a read")
        .expect("the draft is in the store");
    assert_eq!(stored.subject, "The gate");
    assert_eq!(stored.state, DraftState::Editing);
}

#[test]
fn saving_the_same_draft_twice_updates_it_rather_than_writing_a_second_row() {
    // The autosave shape: a composer saves as you type, and a draft that
    // inserted on every keystroke would fill the Drafts folder with the same
    // half-written message.
    let (session, database, _) = a_message_to_answer();
    let mut draft = session.new_draft().expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "First".to_owned();

    let mut saved = session.save_draft(draft).expect("saved");
    saved.subject = "Second".to_owned();
    let again = session.save_draft(saved.clone()).expect("saved again");

    assert_eq!(again.id, saved.id);
    let connection = database.connection().expect("a connection");
    assert_eq!(
        DraftRepository::new(&connection)
            .list_for_account(postio_model::ids::AccountId::new(again.account))
            .expect("a list")
            .len(),
        1,
        "one draft, edited, not two"
    );
}

#[test]
fn sending_queues_the_draft_rather_than_waiting_for_a_server() {
    // Local-first: the write is one transaction and `postio-sync::send`
    // drains it whenever there is a network. Nothing here opens a connection,
    // which is what lets a compose window close on the keystroke.
    let (session, database, _) = a_message_to_answer();
    let mut draft = session.new_draft().expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.body = "Six is fine.".to_owned();

    assert_eq!(session.send_draft(draft), None, "no complaint");

    let connection = database.connection().expect("a connection");
    let drafts = DraftRepository::new(&connection);
    let queued = drafts
        .list_for_account(postio_model::ids::AccountId::new(1))
        .expect("a list");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].state,
        DraftState::Queued,
        "the draft is queued, and the operation row is what sends it"
    );
}

#[test]
fn a_draft_with_nobody_to_send_it_to_is_refused_out_loud() {
    // The composer would otherwise clear and close, and the queued operation
    // would drain as impossible: the words gone and no message sent.
    let (session, _, _) = a_message_to_answer();
    let draft = session.new_draft().expect("a draft");

    let complaint = session.send_draft(draft).expect("a refusal");
    assert!(
        complaint.to_lowercase().contains("recipient"),
        "and it says what is missing: {complaint}"
    );
}

#[test]
fn the_footer_says_where_the_draft_lives_and_what_will_be_sent() {
    // Canvas 26's footer. Both halves are claims Postio should be willing to
    // make on screen: a draft is a file in the maildir, and what leaves is a
    // shape the user chose.
    let (session, _, _) = a_message_to_answer();
    let draft = session.new_draft().expect("a draft");

    assert!(!draft.path.is_empty(), "a draft is somewhere on this disk");
    // Through the exported function, which is what Swift can actually call:
    // a method on a record compiles in Rust and crosses as nothing.
    assert_eq!(postio_ffi::outgoing_shape(true), "html + text/plain");
    assert_eq!(
        postio_ffi::outgoing_shape(false),
        "text/plain, format=flowed"
    );
}

// -- attachments (#1269) -----------------------------------------------------

#[test]
fn attaching_a_file_puts_its_bytes_in_the_store_and_names_it_on_the_draft() {
    let (session, _, _) = a_message_to_answer();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("gate-plan.txt");
    std::fs::write(&path, b"the gate closes at six").expect("a file to attach");

    let draft = session.new_draft().expect("a draft");
    let with_file = session
        .attach_to_draft(draft, path.display().to_string(), "text/plain".to_owned())
        .expect("it attaches");

    assert_eq!(with_file.attachments.len(), 1);
    assert_eq!(with_file.attachments[0].filename, "gate-plan.txt");
    assert_eq!(with_file.attachments[0].mime_type, "text/plain");
    // Said the way a person thinks about it rather than in bytes.
    assert!(
        with_file.attachments[0].size.contains('B'),
        "{}",
        with_file.attachments[0].size
    );
    assert!(
        with_file.id > 0,
        "and the draft was saved, so the file survives the window closing"
    );
}

#[test]
fn a_file_that_is_not_there_is_refused_and_the_draft_is_untouched() {
    let (session, _, _) = a_message_to_answer();
    let draft = session.new_draft().expect("a draft");

    let refusal = session
        .attach_to_draft(
            draft.clone(),
            "/nowhere/at/all/missing.pdf".to_owned(),
            "application/pdf".to_owned(),
        )
        .expect_err("nothing to attach");

    assert!(
        format!("{refusal}").contains("could not be read"),
        "{refusal}"
    );
    assert!(session.new_draft().expect("a draft").attachments.is_empty());
}

#[test]
fn taking_an_attachment_off_leaves_the_rest_alone() {
    let (session, _, _) = a_message_to_answer();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let mut draft = session.new_draft().expect("a draft");
    for name in ["one.txt", "two.txt"] {
        let path = scratch.path().join(name);
        std::fs::write(&path, name.as_bytes()).expect("a file");
        draft = session
            .attach_to_draft(draft, path.display().to_string(), "text/plain".to_owned())
            .expect("it attaches");
    }
    assert_eq!(draft.attachments.len(), 2);

    let first = draft.attachments[0].id;
    let left = session
        .detach_from_draft(draft, first)
        .expect("it detaches");

    assert_eq!(left.attachments.len(), 1);
    assert_eq!(left.attachments[0].filename, "two.txt");
}
