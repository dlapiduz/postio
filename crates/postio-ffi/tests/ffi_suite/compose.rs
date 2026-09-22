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
async fn a_message_to_answer() -> (std::sync::Arc<Session>, postio_storage::Store, i64) {
    let database = test_support::memory().await;
    let message = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.subject = Some("Radon reduction".to_owned());
        message.from = vec![EmailAddress::new(Some("Ada Norwood"), "ada@example.com")];
        message.to = vec![
            account.address.clone(),
            EmailAddress::new(Some("Bo Ferris"), "bo@example.com"),
        ];
        let repository = MessageRepository::new(&connection);
        let id = repository.create(&mut message).await.expect("a message");
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
            .await
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

#[tokio::test(flavor = "multi_thread")]
async fn a_new_message_starts_empty_and_knows_which_account_it_is_from() {
    let (session, _, _) = a_message_to_answer().await;
    let draft = session.new_draft().await.expect("an account to write from");

    assert!(draft.to.is_empty());
    assert!(draft.subject.is_empty());
    assert!(draft.body.is_empty());
    assert!(
        !draft.from.is_empty(),
        "a composer with no From is one nobody can send from"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_goes_to_the_sender_and_quotes_what_was_said() {
    let (session, _, message) = a_message_to_answer().await;
    let draft = session.reply_draft(message, false).await.expect("a reply");

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

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_to_all_keeps_the_others_and_leaves_me_out() {
    let (session, _, message) = a_message_to_answer().await;
    let draft = session.reply_draft(message, true).await.expect("a reply");

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

#[tokio::test(flavor = "multi_thread")]
async fn a_forward_carries_the_message_and_asks_who_to_send_it_to() {
    let (session, _, message) = a_message_to_answer().await;
    let draft = session.forward_draft(message).await.expect("a forward");

    assert!(draft.to.is_empty(), "a forward has nobody until you say so");
    assert_eq!(draft.subject, "Fwd: Radon reduction");
    assert!(draft.body.contains("The gate closes at six."));
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_a_draft_puts_it_in_the_store_and_gives_it_an_id() {
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.body = "Six is fine.".to_owned();

    let saved = session.save_draft(draft).await.expect("a saved draft");
    assert!(
        saved.id > 0,
        "a saved draft has an id to be saved *again* by"
    );

    let connection = database.connect().await.expect("a connection");
    let stored = DraftRepository::new(&connection)
        .get(postio_model::ids::DraftId::new(saved.id))
        .await
        .expect("a read")
        .expect("the draft is in the store");
    assert_eq!(stored.subject, "The gate");
    assert_eq!(stored.state, DraftState::Editing);
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_the_same_draft_twice_updates_it_rather_than_writing_a_second_row() {
    // The autosave shape: a composer saves as you type, and a draft that
    // inserted on every keystroke would fill the Drafts folder with the same
    // half-written message.
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "First".to_owned();

    let mut saved = session.save_draft(draft).await.expect("saved");
    saved.subject = "Second".to_owned();
    let again = session
        .save_draft(saved.clone())
        .await
        .expect("saved again");

    assert_eq!(again.id, saved.id);
    let connection = database.connect().await.expect("a connection");
    assert_eq!(
        DraftRepository::new(&connection)
            .list_for_account(postio_model::ids::AccountId::new(again.account))
            .await
            .expect("a list")
            .len(),
        1,
        "one draft, edited, not two"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sending_queues_the_draft_rather_than_waiting_for_a_server() {
    // Local-first: the write is one transaction and `postio-sync::send`
    // drains it whenever there is a network. Nothing here opens a connection,
    // which is what lets a compose window close on the keystroke.
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.body = "Six is fine.".to_owned();

    assert_eq!(session.send_draft(draft).await, None, "no complaint");

    let connection = database.connect().await.expect("a connection");
    let drafts = DraftRepository::new(&connection);
    let queued = drafts
        .list_for_account(postio_model::ids::AccountId::new(1))
        .await
        .expect("a list");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].state,
        DraftState::Queued,
        "the draft is queued, and the operation row is what sends it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_with_nobody_to_send_it_to_is_refused_out_loud() {
    // The composer would otherwise clear and close, and the queued operation
    // would drain as impossible: the words gone and no message sent.
    let (session, _, _) = a_message_to_answer().await;
    let draft = session.new_draft().await.expect("a draft");

    let complaint = session.send_draft(draft).await.expect("a refusal");
    assert!(
        complaint.to_lowercase().contains("recipient"),
        "and it says what is missing: {complaint}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_footer_says_where_the_draft_lives_and_what_will_be_sent() {
    // Canvas 26's footer. Both halves are claims Postio should be willing to
    // make on screen: a draft is a file in the maildir, and what leaves is a
    // shape the user chose.
    let (session, _, _) = a_message_to_answer().await;
    let draft = session.new_draft().await.expect("a draft");

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

#[tokio::test(flavor = "multi_thread")]
async fn attaching_a_file_puts_its_bytes_in_the_store_and_names_it_on_the_draft() {
    let (session, _, _) = a_message_to_answer().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("gate-plan.txt");
    std::fs::write(&path, b"the gate closes at six").expect("a file to attach");

    let draft = session.new_draft().await.expect("a draft");
    let with_file = session
        .attach_to_draft(draft, path.display().to_string(), "text/plain".to_owned())
        .await
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

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_is_not_there_is_refused_and_the_draft_is_untouched() {
    let (session, _, _) = a_message_to_answer().await;
    let draft = session.new_draft().await.expect("a draft");

    let refusal = session
        .attach_to_draft(
            draft.clone(),
            "/nowhere/at/all/missing.pdf".to_owned(),
            "application/pdf".to_owned(),
        )
        .await
        .expect_err("nothing to attach");

    assert!(
        format!("{refusal}").contains("could not be read"),
        "{refusal}"
    );
    assert!(
        session
            .new_draft()
            .await
            .expect("a draft")
            .attachments
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_an_attachment_off_leaves_the_rest_alone() {
    let (session, _, _) = a_message_to_answer().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let mut draft = session.new_draft().await.expect("a draft");
    for name in ["one.txt", "two.txt"] {
        let path = scratch.path().join(name);
        std::fs::write(&path, name.as_bytes()).expect("a file");
        draft = session
            .attach_to_draft(draft, path.display().to_string(), "text/plain".to_owned())
            .await
            .expect("it attaches");
    }
    assert_eq!(draft.attachments.len(), 2);

    let first = draft.attachments[0].id;
    let left = session
        .detach_from_draft(draft, first)
        .await
        .expect("it detaches");

    assert_eq!(left.attachments.len(), 1);
    assert_eq!(left.attachments[0].filename, "two.txt");
}

// -- an image in the body (#1571) --------------------------------------------

/// A few bytes that say they are a PNG; nothing here decodes them.
const PIXELS: &[u8] = b"\x89PNG\r\n\x1a\nnot really a picture";

/// The `Content-ID` an image script inserts, read back out of the script.
fn inserted_id(script: &str) -> String {
    let start = script.find("postio-cid:").expect("the script names a part") + "postio-cid:".len();
    let end = start + script[start..].find('"').expect("a closing quote");
    script[start..end].replace("%40", "@")
}

#[tokio::test(flavor = "multi_thread")]
async fn inserting_an_image_puts_a_part_on_the_draft_and_a_picture_at_the_caret() {
    let (session, _, _) = a_message_to_answer().await;
    let draft = session.new_draft().await.expect("a draft");

    let inserted = session
        .insert_inline_image(draft, PIXELS.to_vec(), "image/png".to_owned())
        .await
        .expect("it inlines");

    assert_eq!(
        inserted.draft.attachments.len(),
        1,
        "the picture is a part of the message"
    );
    assert!(
        inserted.draft.id > 0,
        "and the draft was saved, so the picture survives the window closing"
    );
    // What the editor runs: the editing shell's scheme, naming a part this
    // draft now holds -- so the picture the window draws is the one it sends.
    assert!(
        inserted.script.contains("insertHTML"),
        "{}",
        inserted.script
    );
    let content_id = inserted_id(&inserted.script);
    let part = session
        .resolve_draft_cid(inserted.draft.id, content_id.clone())
        .await
        .expect("the composer can draw the picture it just inserted");
    assert_eq!(part.bytes, PIXELS);
    assert_eq!(part.mime_type, "image/png");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_resolves_only_its_own_pictures() {
    // A `Content-ID` means something only inside the message that declared
    // it. A composer resolving one against the whole store would draw a
    // picture from somebody else's draft.
    let (session, _, _) = a_message_to_answer().await;
    let mine = session
        .insert_inline_image(
            session.new_draft().await.expect("a draft"),
            PIXELS.to_vec(),
            "image/png".to_owned(),
        )
        .await
        .expect("it inlines");
    let other = session
        .save_draft(session.new_draft().await.expect("another draft"))
        .await
        .expect("saved");

    let content_id = inserted_id(&mine.script);
    assert!(
        session
            .resolve_draft_cid(other.id, content_id)
            .await
            .is_none()
    );
    assert!(
        session
            .resolve_draft_cid(mine.draft.id, "nothing@postio.invalid".to_owned())
            .await
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_is_not_an_image_is_refused_and_the_draft_is_untouched() {
    let (session, _, _) = a_message_to_answer().await;
    let draft = session.new_draft().await.expect("a draft");

    let refusal = session
        .insert_inline_image(draft, b"%PDF-1.7".to_vec(), "application/pdf".to_owned())
        .await
        .expect_err("not a picture");

    assert!(format!("{refusal}").contains("not an image"), "{refusal}");
    assert!(
        session
            .new_draft()
            .await
            .expect("a draft")
            .attachments
            .is_empty(),
        "nothing was attached on the way to refusing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_inserted_picture_survives_the_draft_being_saved() {
    // The editor reports the body with the `<img>` in it, and the store keeps
    // what `postio_body::parse` makes of that. A picture the dialect dropped
    // would vanish at the first autosave, leaving a part nothing shows.
    let (session, _, _) = a_message_to_answer().await;
    let mut inserted = session
        .insert_inline_image(
            session.new_draft().await.expect("a draft"),
            PIXELS.to_vec(),
            "image/png".to_owned(),
        )
        .await
        .expect("it inlines");
    let content_id = inserted_id(&inserted.script);
    let src = format!("postio-cid:{}", content_id.replace('@', "%40"));
    inserted.draft.rich = true;
    inserted.draft.body_html = Some(format!(
        "<p>The gate:</p><p><img src=\"{src}\" alt=\"gate\"></p>"
    ));

    let saved = session.save_draft(inserted.draft).await.expect("saved");

    let body = saved.body_html.expect("a rich body");
    assert!(
        body.contains(&src),
        "the picture did not survive the save: {body}"
    );
}

// -- handing a draft to another editor (#1270) -------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn handing_a_draft_out_saves_it_first_and_writes_what_was_typed() {
    // An editor opened on a body Postio has not written down is one crash
    // away from having been the only copy.
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.body = "The gate closes at six.".to_owned();
    draft.to = "bo@example.com".to_owned();

    let path = session
        .begin_handoff(draft.clone())
        .await
        .expect("it hands out");

    assert_eq!(
        std::fs::read_to_string(&path).expect("the file is there"),
        "The gate closes at six."
    );
    std::fs::remove_file(&path).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_other_editor_wrote_comes_back_onto_the_draft() {
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.body = "before".to_owned();
    let path = session
        .begin_handoff(draft.clone())
        .await
        .expect("it hands out");

    std::fs::write(&path, "after, edited elsewhere").expect("the editor saves");
    // The draft has an id now; the frontend holds the saved one.
    let saved = session.save_draft(draft).await.expect("saved");
    let back = session
        .end_handoff(saved, path.clone())
        .await
        .expect("it comes back");

    assert_eq!(back.body, "after, edited elsewhere");
    assert!(
        !std::path::Path::new(&path).exists(),
        "and the file is taken away: a draft's text must not be left on disk"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_edit_is_refused_and_the_draft_keeps_its_words() {
    // The truncate-and-write window: believing it would throw away
    // everything the user had written.
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.body = "everything they wrote".to_owned();
    let saved = session.save_draft(draft).await.expect("saved");
    let path = session
        .begin_handoff(saved.clone())
        .await
        .expect("it hands out");

    std::fs::write(&path, "\n\n").expect("a save caught mid-write");
    let refusal = session
        .end_handoff(saved.clone(), path.clone())
        .await
        .expect_err("nothing is taken back");

    assert!(format!("{refusal}").contains("empty"), "{refusal}");
    std::fs::remove_file(&path).ok();
}

// -- Rich composition (#1271) ------------------------------------------------
//
// The switch existed and decided what would be *built*; the body was a text
// field, so there was never an HTML part to build one out of. `rich` was
// therefore a claim the composer could not keep, which is why its footer said
// "plain" whatever the switch was set to.

#[tokio::test(flavor = "multi_thread")]
async fn a_rich_draft_keeps_its_marks_across_a_save() {
    // The acceptance line: marks apply to the body and survive save. They
    // survive by being *stored*, so this reads the draft back out of the
    // store rather than trusting what save handed back.
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("an account");
    draft.to = "bo@example.com".to_owned();
    draft.rich = true;
    draft.body_html = Some("<p>The gate closes at <strong>six</strong>.</p>".to_owned());
    draft.body = "The gate closes at six.".to_owned();

    let saved = session.save_draft(draft).await.expect("a saved draft");
    let reopened = session
        .draft(saved.id)
        .await
        .expect("the draft is in the store");

    assert!(reopened.rich, "the draft stopped being rich on the way in");
    assert_eq!(
        reopened.body_html.as_deref(),
        Some("<p>The gate closes at <strong>six</strong>.</p>"),
        "the marks did not survive the round trip"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rich_body_is_stored_as_the_dialect_not_as_whatever_was_typed() {
    // The editing surface hands over a DOM's innerHTML, which is a working
    // copy and not the record (ADR 0004 Q3). What is kept is what `parse`
    // makes of it -- so a `<div>` from the browser becomes a paragraph, and
    // a `<script>` cannot be stored at all.
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("an account");
    draft.to = "bo@example.com".to_owned();
    draft.rich = true;
    draft.body_html = Some("<div>hello<script>alert(1)</script></div>".to_owned());

    let saved = session.save_draft(draft).await.expect("a saved draft");
    let html = saved.body_html.expect("a rich draft has an HTML part");
    assert!(
        !html.contains("script"),
        "a script survived into the stored draft: {html}"
    );
    assert!(html.contains("hello"), "the words were lost too: {html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_rich_off_stops_building_html_without_throwing_the_words_away() {
    // The switch is on the document, not on the window: turning it off
    // changes what will be built, and must not silently discard what was
    // written in case it is turned back on.
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("an account");
    draft.to = "bo@example.com".to_owned();
    draft.rich = false;
    draft.body = "The gate closes at six.".to_owned();
    draft.body_html = Some("<p>The gate closes at <strong>six</strong>.</p>".to_owned());

    let saved = session.save_draft(draft).await.expect("a saved draft");
    assert!(!saved.rich, "the switch was ignored");
    assert_eq!(saved.body, "The gate closes at six.");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rich_draft_reports_the_shape_it_will_actually_leave_as() {
    // The footer's claim, and the acceptance line "rich sends text/html plus
    // a text/plain fallback, always". `postio_ui::compose::outgoing_shape` is
    // the wording; what matters here is that the boundary now agrees the
    // draft *is* rich, which it could not before.
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("an account");
    draft.to = "bo@example.com".to_owned();
    draft.rich = true;
    draft.body_html = Some("<p>hi</p>".to_owned());

    let saved = session.save_draft(draft).await.expect("a saved draft");
    assert!(saved.rich);
    assert!(
        postio_ui::compose::outgoing_shape(saved.rich).contains("text/plain"),
        "rich must still carry a plain alternative"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rich_draft_always_carries_a_plain_alternative_of_its_own_words() {
    // Not an empty `text/plain`: the fallback is the message for anyone
    // reading in a terminal or with a screen reader, so it has to say what
    // the HTML says. Derived rather than asked for, because a composer that
    // relied on the frontend to send both would eventually send one.
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("an account");
    draft.to = "bo@example.com".to_owned();
    draft.rich = true;
    draft.body_html = Some("<p>The gate closes at <strong>six</strong>.</p>".to_owned());
    draft.body = String::new();

    let saved = session.save_draft(draft).await.expect("a saved draft");
    assert!(
        saved.body.contains("gate closes at six"),
        "the plain alternative is empty, so half the recipients get nothing: {:?}",
        saved.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paste_says_what_the_dialect_could_not_hold() {
    // The acceptance line. Against `postio_body`'s own answer rather than a
    // sentence typed here: both composers show this string, and a copy in a
    // test would drift exactly as a copy in the code would.
    let (session, _, _) = a_message_to_answer().await;
    let pasted = session.narrow_paste(
        "<p style='color:red'>hi</p><table><tr><td>a</td></tr></table>\
         <img src='https://example.com/a.png'>"
            .to_owned(),
    );

    let expected = postio_body::narrow(
        "<p style='color:red'>hi</p><table><tr><td>a</td></tr></table>\
         <img src='https://example.com/a.png'>",
    );
    assert_eq!(pasted.dropped, expected.lost.summary());
    assert!(
        pasted.dropped.is_some(),
        "a paste that lost a table, an image and its colours said nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paste_the_dialect_holds_whole_says_nothing_at_all() {
    // Silence is the right answer when nothing was lost. A composer that
    // announced every paste would train people to ignore the one that
    // mattered.
    let (session, _, _) = a_message_to_answer().await;
    let pasted = session.narrow_paste("<p>Hello <em>there</em>.</p>".to_owned());
    assert_eq!(pasted.dropped, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paste_comes_back_as_the_dialect_and_as_plain_text() {
    // Both, because the composer needs the first to put in the document and
    // the second to keep the plain alternative honest.
    let (session, _, _) = a_message_to_answer().await;
    let pasted = session.narrow_paste("<div>Hello <b>there</b>.</div>".to_owned());

    assert!(
        pasted.html.contains("<strong>"),
        "not the dialect: {}",
        pasted.html
    );
    assert!(
        !pasted.html.contains("<b>"),
        "element form was not normalised"
    );
    assert_eq!(pasted.text.trim(), "Hello there.");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_editing_bridge_crosses_so_the_two_composers_run_one_dialect() {
    // #1271's last acceptance line. The script is the thing that decides
    // whether the surface emits `<p>` or `<div>`, and a second copy on the
    // macOS side would be a second dialect that still round-trips through a
    // Document -- drift nothing would catch.
    let (session, _, _) = a_message_to_answer().await;
    assert_eq!(session.editor_script(), postio_ui::compose::editor_script());
    assert!(
        session
            .editor_script()
            .contains("defaultParagraphSeparator"),
        "the setting that pins the dialect is not in what crossed"
    );
    // What crossed has to be *runnable*. The body reads `POSTIO_MARKDOWN`
    // and does not define it, so a frontend handed the body alone gets a
    // ReferenceError on the first keystroke -- inside a WebView, where
    // nothing on this side of the boundary would ever hear about it. GTK
    // prepends the table itself; macOS takes whatever crosses.
    assert!(
        session.editor_script().contains("const POSTIO_MARKDOWN"),
        "the markdown table did not cross, so the script cannot run"
    );
    assert!(
        session
            .editor_script()
            .contains(r#"marker: "**", command: "bold""#),
        "the table crossed empty"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_sent_as_plain_puts_no_html_on_the_wire_even_while_it_keeps_its_marks() {
    // The two halves of "the switch is on the document" pulling against each
    // other, and the place they have to be reconciled.
    //
    // Storage keeps the marks, so turning Rich back on does not cost them.
    // But `postio_model::outgoing` builds a `multipart/alternative` from
    // `body.html.is_some()` -- so a queued draft that kept its marks would
    // send HTML while its own footer said "text/plain, format=flowed". The
    // footer is a claim about what leaves; this is what keeps it true.
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.rich = false;
    draft.body = "Six is fine.".to_owned();
    draft.body_html = Some("<p>Six is <strong>fine</strong>.</p>".to_owned());

    assert_eq!(session.send_draft(draft).await, None, "no complaint");

    let connection = database.connect().await.expect("a connection");
    let queued = DraftRepository::new(&connection)
        .list_for_account(postio_model::ids::AccountId::new(1))
        .await
        .expect("a list");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].body.html, None,
        "a draft queued as plain still carries an HTML part, so it will \
         leave as multipart/alternative and the footer lied"
    );
    assert_eq!(queued[0].body.text.as_deref(), Some("Six is fine."));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_sent_as_rich_carries_both_parts_onto_the_queue() {
    // The other direction, and the acceptance line: rich sends `text/html`
    // plus a `text/plain` fallback, always.
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.rich = true;
    draft.body_html = Some("<p>Six is <strong>fine</strong>.</p>".to_owned());
    draft.body = String::new();

    assert_eq!(session.send_draft(draft).await, None, "no complaint");

    let connection = database.connect().await.expect("a connection");
    let queued = DraftRepository::new(&connection)
        .list_for_account(postio_model::ids::AccountId::new(1))
        .await
        .expect("a list");
    let body = &queued[0].body;
    assert!(
        body.html
            .as_deref()
            .is_some_and(|html| html.contains("<strong>")),
        "the marks did not reach the queue: {:?}",
        body.html
    );
    assert!(
        body.text
            .as_deref()
            .is_some_and(|text| text.contains("Six is fine")),
        "no plain alternative, so a terminal reader gets nothing: {:?}",
        body.text
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn writing_rich_and_switching_to_plain_sends_the_words_rather_than_nothing() {
    // #1293, and the sequence a person actually performs: everything is
    // typed into the rich surface, so `body` -- the plain field -- was never
    // touched. Every other test in this file sets both fields by hand, which
    // is exactly what hid this.
    //
    // `send_draft` clears the HTML part when the switch says plain, which is
    // right: the footer promises `text/plain, format=flowed`. So if the text
    // was never derived, what is queued has no body at all.
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.rich = true;
    draft.body_html = Some("<p>The gate closes at six.</p>".to_owned());
    // Deliberately not set: nothing typed into a `TextEditor` that was not on
    // screen.
    draft.body = String::new();

    // The switch goes to Plain, and nothing else changes.
    draft.rich = false;

    assert_eq!(session.send_draft(draft).await, None, "no complaint");

    let connection = database.connect().await.expect("a connection");
    let queued = DraftRepository::new(&connection)
        .list_for_account(postio_model::ids::AccountId::new(1))
        .await
        .expect("a list");
    let body = &queued[0].body;
    assert_eq!(body.html, None, "plain must not put HTML on the wire");
    assert!(
        body.text
            .as_deref()
            .is_some_and(|text| text.contains("gate closes at six")),
        "an empty message was queued: {:?}",
        body.text
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_plain_text_of_a_document_is_available_on_its_own() {
    // What the switch needs at the moment it flips, and it needs a name that
    // is true at that call site: `narrowPaste` returns the same string but
    // reading `narrowPaste` there would say this was a paste.
    let (session, _, _) = a_message_to_answer().await;
    let text = session.plain_text_of("<p>The gate closes at <strong>six</strong>.</p>");
    assert_eq!(text.trim(), "The gate closes at six.");
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_the_other_way_keeps_the_words_too() {
    // Plain -> Rich -> send. The document is built from the plain text, so
    // nothing typed is lost in that direction either.
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.body = "The gate closes at six.".to_owned();
    draft.rich = true;
    draft.body_html = None;

    assert_eq!(session.send_draft(draft).await, None, "no complaint");

    let connection = database.connect().await.expect("a connection");
    let queued = DraftRepository::new(&connection)
        .list_for_account(postio_model::ids::AccountId::new(1))
        .await
        .expect("a list");
    let body = &queued[0].body;
    assert!(
        body.text
            .as_deref()
            .is_some_and(|text| text.contains("gate closes at six")),
        "the plain alternative lost the words: {:?}",
        body.text
    );
    assert!(
        body.html
            .as_deref()
            .is_some_and(|html| html.contains("gate closes at six")),
        "rich was asked for and no HTML part was built: {:?}",
        body.html
    );
}

// --- a draft from a `mailto:` link (#1156 follow-on) ------------------------

/// A session with an account to write from.
///
/// `a_message_to_answer` already builds one; this drops the message, because
/// a `mailto:` answers nothing.
async fn a_session_with_an_account() -> (std::sync::Arc<Session>, postio_storage::Store) {
    let (session, database, _) = a_message_to_answer().await;
    (session, database)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mailto_link_becomes_a_draft_addressed_the_way_it_asked() {
    // RFC 6068's fields, assembled here rather than in either frontend, so
    // clicking the same link on both platforms opens the same draft.
    let (session, _) = a_session_with_an_account().await;

    let draft = session
        .mailto_draft(
            vec!["ada@example.com".to_owned(), "grace@example.net".to_owned()],
            vec!["alan@example.org".to_owned()],
            vec![],
            Some("About invoice 4021".to_owned()),
            Some("Reference: 4021".to_owned()),
        )
        .await
        .expect("an account to write from");

    assert_eq!(draft.to, "ada@example.com, grace@example.net");
    assert_eq!(draft.cc, "alan@example.org");
    assert_eq!(draft.bcc, "");
    assert_eq!(draft.subject, "About invoice 4021");
    assert_eq!(
        draft.body, "Reference: 4021",
        "a link may prefill a body — it lands in a composer the user is \
         looking at, unsent, and refusing it would break every \
         `email us about this` link that carries a reference"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bare_mailto_opens_an_empty_draft_rather_than_refusing() {
    // `mailto:` with nothing after it is a valid link and means "write a new
    // message". It must not read as a failure.
    let (session, _) = a_session_with_an_account().await;

    let draft = session
        .mailto_draft(vec![], vec![], vec![], None, None)
        .await
        .expect("an empty link is still a draft");

    assert_eq!(draft.to, "");
    assert_eq!(draft.subject, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mailto_with_no_account_to_write_from_answers_none() {
    // A fresh install. The frontend says so out loud rather than opening a
    // composer that cannot send.
    let session = Session::open(SessionOptions::in_memory()).expect("a session");

    assert!(
        session
            .mailto_draft(
                vec!["ada@example.com".to_owned()],
                vec![],
                vec![],
                None,
                None
            )
            .await
            .is_none()
    );
}

// --- replying to HTML-only mail (user report) -------------------------------

/// A message whose body is HTML and nothing else — the ordinary shape of most
/// real mail, and the one a reply used to lose entirely.
async fn an_html_only_message() -> (std::sync::Arc<Session>, postio_storage::Store, i64) {
    let database = test_support::memory().await;
    let message = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.subject = Some("The gate".to_owned());
        message.from = vec![EmailAddress::new(Some("Ada Norwood"), "ada@example.com")];
        message.to = vec![account.address.clone()];
        let repository = MessageRepository::new(&connection);
        let id = repository.create(&mut message).await.expect("a message");
        repository
            .set_body(
                id,
                &postio_storage::repository::StoredBody {
                    // No `text` at all. `postio_model::mime` fills that field
                    // from a `text/plain` part and never invents one.
                    text: None,
                    html: Some("<p>The gate closes at six.</p><p>Bring the key.</p>".to_owned()),
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::message::BodyState::Full,
            )
            .await
            .expect("the body is stored");
        id.get()
    };

    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");
    let session = Session::open(
        SessionOptions::in_memory_with(database.clone()).with_blobs_for_test(blobs, scratch),
    )
    .expect("a session");
    (session, database, message)
}

#[tokio::test(flavor = "multi_thread")]
async fn replying_to_html_only_mail_quotes_what_it_is_answering() {
    // The report: "reply doesn't quote a message correctly". It quoted the
    // attribution line and nothing else, because `plain_quote` reads
    // `body.text` and an HTML-only message has none — which is most mail.
    let (session, _database, message) = an_html_only_message().await;

    let draft = session.reply_draft(message, false).await.expect("a reply");

    assert!(
        draft.body.contains("Ada Norwood wrote:"),
        "the attribution is still there: {:?}",
        draft.body
    );
    assert!(
        draft.body.contains("> The gate closes at six."),
        "the message being answered has to be in the quote: {:?}",
        draft.body
    );
    assert!(
        draft.body.contains("> Bring the key."),
        "every line of it, not just the first: {:?}",
        draft.body
    );
    assert!(
        !draft.body.contains('<'),
        "quoted as text, not as markup: {:?}",
        draft.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_text_message_is_still_quoted_from_its_own_text() {
    // The rendering is a *fallback*. A message that has real plain text must
    // keep using it — it is what the sender wrote, and a round trip through
    // HTML would not be.
    let (session, _database, message) = a_message_to_answer().await;

    let draft = session.reply_draft(message, false).await.expect("a reply");

    assert!(
        draft.body.contains("> The gate closes at six."),
        "{:?}",
        draft.body
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_saved_on_this_machine_is_queued_for_the_server_too() {
    // A local write without its queue row never reaches the server. Every
    // macOS save path -- autosave, attach, detach, the editor hand-off both
    // ways -- goes through one `write_draft`, and it called
    // `DraftRepository::save`, which writes the row and stops. So a reply
    // begun on the Mac was in Drafts on the Mac and nowhere else: not on the
    // phone, not on the Linux client, not on the server it was written
    // against. `postio-app` has used `save_and_sync` since drafts existed,
    // for exactly the reason that repository method records.
    let (session, database, message) = a_message_to_answer().await;

    // The account needs a Drafts folder to enqueue against -- `save_and_sync`
    // finds it by role, and an account with none is a real state it answers
    // with `None` rather than by failing.
    let account = {
        let connection = database.connect().await.expect("a connection");
        let accounts = postio_storage::repository::AccountRepository::new(&connection)
            .list()
            .await
            .expect("a read");
        let account = accounts.first().expect("the seeded account").clone();
        let mut drafts = postio_model::Mailbox::new(account.id, "Drafts", Some('/'));
        drafts.role = postio_model::MailboxRole::Drafts;
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut drafts)
            .await
            .expect("a Drafts folder");
        account.id
    };

    // Built, then saved: `reply_draft` assembles it in memory and the write
    // is `save_draft`, which is what a composer calls when it autosaves.
    let draft = session
        .reply_draft(message, false)
        .await
        .expect("a reply to save");
    session.save_draft(draft).await.expect("the draft is saved");

    let queued = {
        let connection = database.connect().await.expect("a connection");
        postio_storage::repository::OperationQueueRepository::new(&connection)
            .pending(account, Utc::now())
            .await
            .expect("a read of the queue")
    };
    assert!(
        queued.iter().any(|operation| matches!(
            operation.operation,
            postio_model::Operation::SaveDraft { .. }
        )),
        "the draft was written locally and nothing will carry it to the \
         server: {queued:?}"
    );
    session.shutdown();
}

#[test]
fn a_reply_all_to_a_list_does_not_look_like_a_reply() {
    // FR-023, and the whole reason the Cc and Bcc fields had to arrive on
    // macOS at all: `reply_all` puts every other original recipient in `cc`,
    // so a window showing one To address is a window about to write to
    // everybody. The counts say *which field*, because "42 recipients" reads
    // very differently from "1 To, 41 Cc".
    let draft = postio_ffi::DraftFfi {
        id: 0,
        account: 1,
        kind: postio_ffi::DraftKindFfi::ReplyAll,
        from: "Mara Ostwald <mara@example.com>".to_owned(),
        to: "Ada Norwood <ada@example.com>".to_owned(),
        cc: "Bo Ferris <bo@example.com>, Quinn Vale <quinn@example.net>".to_owned(),
        bcc: String::new(),
        subject: "Re: Radon reduction".to_owned(),
        body: String::new(),
        body_html: None,
        rich: false,
        in_reply_to: None,
        path: "/tmp/mail".to_owned(),
        attachments: Vec::new(),
    };
    assert_eq!(
        postio_ffi::recipient_summary(draft.clone()).as_deref(),
        Some("1 To, 2 Cc")
    );

    // One person is the ordinary case and says nothing.
    let alone = postio_ffi::DraftFfi {
        cc: String::new(),
        ..draft
    };
    assert_eq!(postio_ffi::recipient_summary(alone), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_sessions_do_not_hand_drafts_out_into_the_same_directory() {
    // An in-memory session has no store on disk, so `handoff_dir` falls back
    // to a temp directory -- and it fell back to *one*, shared by every
    // process on the machine. Draft ids start at 1 in every fresh store, so
    // two sessions handing out their first draft wrote to the same file. In
    // the test suite that is two binaries racing: it passes alone and in its
    // own crate, and fails when `postio-session` happens to run beside it.
    //
    // Which is the worst kind of red, because it is read as noise. It is not
    // noise here either: the fallback is what a Postio with no store yet
    // uses, and two Postios on one machine would trade drafts through it.
    let (first, _, _) = a_message_to_answer().await;
    let (second, _, _) = a_message_to_answer().await;

    let one = first
        .begin_handoff(first.new_draft().await.expect("a draft"))
        .await
        .expect("the first hands out");
    let two = second
        .begin_handoff(second.new_draft().await.expect("a draft"))
        .await
        .expect("the second hands out");

    assert_ne!(
        one, two,
        "two sessions handed their drafts to the same file"
    );
    std::fs::remove_file(&one).ok();
    std::fs::remove_file(&two).ok();
    first.shutdown();
    second.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_can_be_thrown_away() {
    // Until #1571 there was no way to: the only exit from a macOS compose
    // window was `onDisappear`, which *saves*. A draft written on the Mac was
    // write-once — it landed in Drafts as an unremarkable row and nothing
    // could open it or remove it again.
    //
    // `DraftRepository::discard` has existed all along and also queues the
    // server copy's removal; what was missing was a way to reach it.
    let (session, database, message) = a_message_to_answer().await;
    let draft = session
        .reply_draft(message, false)
        .await
        .expect("a reply to discard");
    let saved = session.save_draft(draft).await.expect("saved");

    assert!(
        session.draft(saved.id).await.is_some(),
        "the fixture never saved anything to discard"
    );

    assert_eq!(session.discard_draft(saved.id).await, None, "no complaint");
    assert!(
        session.draft(saved.id).await.is_none(),
        "the draft is still there"
    );

    // Twice is not an error: a retried discard, or one racing a send that
    // already cleared the row, is the expected case rather than a failure.
    assert_eq!(session.discard_draft(saved.id).await, None);
    let _ = database;
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_send_is_queued_with_the_hour_it_is_for() {
    // *Schedule send…* — the same queue as an immediate send, with a time on
    // the row. `postio-sync::send` is what holds it back until then, so the
    // composer closes on the keystroke exactly as it does for `⌘↵`.
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.body = "Six is fine.".to_owned();

    let when = chrono::Utc::now() + chrono::Duration::hours(3);
    assert_eq!(
        session
            .send_draft_later(draft, when.timestamp_millis())
            .await,
        None,
        "no complaint"
    );

    let connection = database.connect().await.expect("a connection");
    let queued = DraftRepository::new(&connection)
        .list_for_account(postio_model::ids::AccountId::new(1))
        .await
        .expect("a list");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].state, DraftState::Queued);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_send_is_refused_now_rather_than_at_the_hour() {
    // The whole reason this repeats `send_draft`'s checks instead of trusting
    // them to the drain: refusing an unaddressed message at 8am tomorrow,
    // when nobody is watching the composer, is strictly worse than refusing
    // it while somebody is looking at it.
    let (session, _, _) = a_message_to_answer().await;
    let draft = session.new_draft().await.expect("a draft");

    let when = chrono::Utc::now() + chrono::Duration::hours(3);
    let complaint = session
        .send_draft_later(draft, when.timestamp_millis())
        .await
        .expect("a refusal");
    assert!(
        complaint.to_lowercase().contains("recipient"),
        "and it says what is missing: {complaint}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_hour_already_gone_is_not_a_schedule() {
    // A picker left open overnight, or a clock that moved. Sending it at
    // once would be a different command than the one that was chosen, and
    // silently is worse still.
    let (session, _, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();

    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let complaint = session
        .send_draft_later(draft, past.timestamp_millis())
        .await
        .expect("a refusal");
    assert!(
        complaint.to_lowercase().contains("past"),
        "and it says why: {complaint}"
    );
}
