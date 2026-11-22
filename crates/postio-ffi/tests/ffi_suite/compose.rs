use chrono::Utc;
use postio_core::bridge::Bridge;
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_ffi::{Session, SessionOptions};
use postio_model::{DraftState, EmailAddress, Message};
use postio_storage::repository::{DraftRepository, MessageRepository};
use postio_storage::test_support;

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
        // The body is a column of its own (ADR 0020), so a reply that reads the row alone quotes
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
    // Autosave shape: inserting on every keystroke would fill Drafts with copies of one
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
    // Local-first: one transaction that `postio-sync::send` drains later; nothing here opens a
    // connection.
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
    // Else the composer would clear and close and the queued operation would drain as impossible.
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
    let (session, _, _) = a_message_to_answer().await;
    let draft = session.new_draft().await.expect("a draft");

    assert!(!draft.path.is_empty(), "a draft is somewhere on this disk");
    // Through the exported function: a method on a record compiles in Rust and crosses as nothing.
    assert_eq!(postio_ffi::outgoing_shape(true), "html + text/plain");
    assert_eq!(
        postio_ffi::outgoing_shape(false),
        "text/plain, format=flowed"
    );
}

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

const PIXELS: &[u8] = b"\x89PNG\r\n\x1a\nnot really a picture";

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
    // A `Content-ID` means something only inside its own message; resolving it store-wide would
    // draw another draft's picture.
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
    // A picture the dialect dropped would vanish at the first autosave.
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

#[tokio::test(flavor = "multi_thread")]
async fn handing_a_draft_out_saves_it_first_and_writes_what_was_typed() {
    // An editor opened on a body Postio has not written down is one crash from being the only copy.
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
    // Believing the truncate-and-write window would discard everything the user had written.
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

#[tokio::test(flavor = "multi_thread")]
async fn a_rich_draft_keeps_its_marks_across_a_save() {
    // Read the draft back from the store rather than trusting what save returned.
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
async fn turning_rich_off_stops_building_html_without_throwing_the_words_away() {
    // The switch is on the document: turning it off must not discard marks in case it is turned
    // back on.
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
    // The text/plain fallback must say what the HTML says; derived rather than asked of the
    // frontend, which would eventually send only one.
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
async fn a_paste_comes_back_as_the_dialect_and_as_plain_text() {
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
    // The script decides `<p>` vs `<div>`; a macOS copy would be a second dialect that still
    // round-trips.
    let (session, _, _) = a_message_to_answer().await;
    assert_eq!(session.editor_script(), postio_ui::compose::editor_script());
    assert!(
        session
            .editor_script()
            .contains("defaultParagraphSeparator"),
        "the setting that pins the dialect is not in what crossed"
    );
    // The body reads `POSTIO_MARKDOWN` without defining it, so a frontend given the body alone
    // throws in a WebView where nothing hears it.
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
    // Storage keeps the marks, but `postio_model::outgoing` builds multipart/alternative from
    // `body.html.is_some()`,
    // so a Plain draft that kept them would send HTML under a text/plain footer.
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
    // Everything is typed into the rich surface, so `body` is never touched; if the text were not
    // derived, a Plain send would queue no body (#1293).
    let (session, database, _) = a_message_to_answer().await;
    let mut draft = session.new_draft().await.expect("a draft");
    draft.to = "bo@example.com".to_owned();
    draft.subject = "The gate".to_owned();
    draft.rich = true;
    draft.body_html = Some("<p>The gate closes at six.</p>".to_owned());
    draft.body = String::new();

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
    let (session, _, _) = a_message_to_answer().await;
    let text = session.plain_text_of("<p>The gate closes at <strong>six</strong>.</p>");
    assert_eq!(text.trim(), "The gate closes at six.");
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_text_turned_rich_is_a_document_of_the_same_words() {
    let (session, _, _) = a_message_to_answer().await;
    let html = session.rich_html_of("The gate closes at six.\n\nBring <the> key.");
    assert_eq!(
        html,
        "<p>The gate closes at six.</p><p>Bring &lt;the&gt; key.</p>"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_the_other_way_keeps_the_words_too() {
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

async fn a_session_with_an_account() -> (std::sync::Arc<Session>, postio_storage::Store) {
    let (session, database, _) = a_message_to_answer().await;
    (session, database)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mailto_link_becomes_a_draft_addressed_the_way_it_asked() {
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
    // `mailto:` with nothing after it means "write a new message", not a failure.
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
    // `plain_quote` reads `body.text`, which an HTML-only message (most mail) does not have.
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
async fn a_draft_saved_on_this_machine_is_queued_for_the_server_too() {
    // A local write without its queue row never reaches the server: every Mac save path must go
    // through `save_and_sync`, not `DraftRepository::save`.
    let (session, database, message) = a_message_to_answer().await;

    // `save_and_sync` finds the Drafts folder by role; an account with none answers `None`.
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
    // `reply_all` puts the other recipients in `cc`, so the counts must say which field: "1 To, 41
    // Cc".
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
        forwarded_from: None,
        path: "/tmp/mail".to_owned(),
        attachments: Vec::new(),
        remind_at: None,
        labels: Vec::new(),
    };
    assert_eq!(
        postio_ffi::recipient_summary(draft.clone()).as_deref(),
        Some("1 To, 2 Cc")
    );

    let alone = postio_ffi::DraftFfi {
        cc: String::new(),
        ..draft
    };
    assert_eq!(postio_ffi::recipient_summary(alone), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_sessions_do_not_hand_drafts_out_into_the_same_directory() {
    // The in-memory fallback handoff dir was shared by every process and draft ids start at 1, so
    // two sessions wrote the same file and raced.
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
    // `DraftRepository::discard` also queues removal of the server copy; the Mac had no way to
    // reach it (#1571).
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

    // A retried discard, or one racing a send, is expected rather than a failure.
    assert_eq!(session.discard_draft(saved.id).await, None);
    let _ = database;
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_send_is_queued_with_the_hour_it_is_for() {
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
    // Checked at schedule time: refusing an unaddressed message at 8am with nobody watching is
    // worse than refusing it now.
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
    // A stale picker or moved clock: sending at once would be a different command than the one
    // chosen.
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

#[tokio::test(flavor = "multi_thread")]
async fn a_drafts_message_row_leads_back_to_the_draft() {
    let (session, database, message) = a_message_to_answer().await;
    let saved = session
        .save_draft(session.reply_draft(message, false).await.expect("a reply"))
        .await
        .expect("saved");
    let row = {
        let connection = database.connect().await.expect("a connection");
        let replied = MessageRepository::new(&connection)
            .get(postio_model::ids::MessageId::new(message))
            .await
            .expect("a read")
            .expect("the message");
        let mut copy = Message::new(replied.account_id, replied.mailbox_id, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut copy)
            .await
            .expect("the draft's server copy");
        postio_storage::repository::DraftRepository::new(&connection)
            .set_synced_message(postio_model::ids::DraftId::new(saved.id), copy.id)
            .await
            .expect("linked");
        copy.id.get()
    };

    let resumed = session.draft_for_message(row).await;

    assert_eq!(resumed.map(|draft| draft.id), Some(saved.id));
    assert!(
        session.draft_for_message(message).await.is_none(),
        "a message somebody sent is not a draft to resume"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn remind_if_no_reply_is_kept_with_the_draft() {
    let (session, database, message) = a_message_to_answer().await;
    let mut draft = session.reply_draft(message, false).await.expect("a reply");
    assert_eq!(draft.remind_at, None, "a reply asks for no reminder");
    let tuesday = chrono::Utc::now().timestamp_millis() + 3 * 86_400_000;
    draft.remind_at = Some(tuesday);
    let saved = session.save_draft(draft).await.expect("saved");

    let connection = database.connect().await.expect("a connection");
    let stored = DraftRepository::new(&connection)
        .get(postio_model::ids::DraftId::new(saved.id))
        .await
        .expect("a read")
        .expect("the draft is in the store");
    assert_eq!(
        stored.remind_at.map(|at| at.timestamp_millis()),
        Some(tuesday)
    );
    let again = session.draft(saved.id).await.expect("reopened");
    assert_eq!(again.remind_at, Some(tuesday));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replys_quote_folds_and_says_what_it_holds() {
    let (session, _, message) = a_message_to_answer().await;
    let draft = session.reply_draft(message, false).await.expect("a reply");
    let fold = postio_ffi::fold_quote(draft.body.clone()).expect("a reply ends in its quote");
    assert!(
        fold.summary.contains("Ada Norwood wrote") && fold.summary.ends_with("1 quoted line"),
        "{}",
        fold.summary
    );
    assert_eq!(format!("{}{}", fold.written, fold.quote), draft.body);
    assert_eq!(postio_ffi::fold_quote("Six is fine.".to_owned()), None);
}

#[test]
fn the_composers_words_are_postio_uis() {
    use postio_ffi::DraftKindFfi;
    assert_eq!(postio_ffi::composer_title(DraftKindFfi::New), "New message");
    assert_eq!(
        postio_ffi::composer_title(DraftKindFfi::ReplyAll),
        "Reply to all"
    );
    let draft = postio_ffi::DraftFfi {
        id: 0,
        account: 1,
        kind: DraftKindFfi::New,
        from: String::new(),
        to: String::new(),
        cc: String::new(),
        bcc: String::new(),
        subject: String::new(),
        body: "Hi Ada,\n\nThe sheet is attached.".to_owned(),
        body_html: None,
        rich: false,
        in_reply_to: None,
        forwarded_from: None,
        path: String::new(),
        attachments: Vec::new(),
        remind_at: None,
        labels: Vec::new(),
    };
    assert_eq!(
        postio_ffi::draft_summary(draft.clone()),
        "Plain text \u{b7} 6 words"
    );
    // Turned Plain, the draft keeps its marks for the way back but sending drops them; the footer
    // must not say "Rich text".
    let mut switched = draft;
    switched.body_html = Some("<p>Hi <strong>Ada</strong>,</p>".to_owned());
    switched.rich = false;
    assert_eq!(
        postio_ffi::draft_summary(switched.clone()),
        "Plain text \u{b7} 6 words"
    );
    switched.rich = true;
    assert!(postio_ffi::draft_summary(switched).starts_with("Rich text"));
    let at = chrono::Utc::now().timestamp_millis();
    assert!(postio_ffi::draft_saved_words(at).starts_with("Draft saved locally "));
    assert_eq!(postio_ffi::remind_meaning(None), "Remind if no reply");
    assert!(postio_ffi::remind_meaning(Some(at)).starts_with("Remind if no reply \u{b7} "));
    assert_eq!(postio_ffi::remind_presets().len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn choosing_another_account_moves_the_draft_to_it() {
    let (session, database, _) = a_message_to_answer().await;
    let second = {
        let connection = database.connect().await.expect("a connection");
        let mut account = postio_model::Account::new(
            "Work",
            EmailAddress::new(Some("Test User"), "work@example.org"),
        );
        account.incoming.host = "imap.example.org".to_owned();
        account.outgoing.host = "smtp.example.org".to_owned();
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .expect("a second account");
        account.id
    };
    let mut draft = session.new_draft().await.expect("a draft");
    draft.subject = "From work".to_owned();
    let saved = session.save_draft(draft).await.expect("saved");
    assert_ne!(saved.account, i64::from(second), "written from the first");

    let mut moved = saved.clone();
    moved.account = second.into();
    let saved = session.save_draft(moved).await.expect("saved again");
    let connection = database.connect().await.expect("a connection");
    let stored = DraftRepository::new(&connection)
        .get(postio_model::ids::DraftId::new(saved.id))
        .await
        .expect("a read")
        .expect("the draft is in the store");
    assert_eq!(
        stored.account_id, second,
        "the draft moved to the account chosen"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_starts_with_its_threads_labels_and_keeps_what_is_left() {
    let (session, database, message) = a_message_to_answer().await;
    {
        let connection = database.connect().await.expect("a connection");
        let source = MessageRepository::new(&connection)
            .get(postio_model::MessageId::new(message))
            .await
            .expect("a read")
            .expect("the message");
        postio_storage::repository::ThreadingRepository::new(&connection, source.account_id)
            .thread(&source)
            .await
            .expect("in a conversation");
        let labels = postio_storage::repository::LabelRepository::new(&connection);
        for name in ["Harbor", "Atlas"] {
            let mut label = postio_model::Label::new(source.account_id, name);
            let id = labels.create(&mut label).await.expect("a label");
            labels.attach(source.id, id).await.expect("labelled");
        }
    }

    let draft = session.reply_draft(message, true).await.expect("a reply");
    let mut names: Vec<_> = draft
        .labels
        .iter()
        .map(|label| label.name.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["Atlas", "Harbor"],
        "the thread's labels come with the reply"
    );

    let mut edited = draft.clone();
    edited.labels.retain(|label| label.name == "Harbor");
    let saved = session.save_draft(edited).await.expect("saved");
    let reopened = session.draft(saved.id).await.expect("the draft again");
    assert_eq!(
        reopened
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["Harbor"],
        "a label taken off stays off"
    );

    let fresh = session.forward_draft(message).await.expect("a forward");
    assert!(
        fresh.labels.is_empty(),
        "a forward starts a conversation of its own"
    );
}
