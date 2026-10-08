//! Focus's engine runs behind the Mac app (specs/009-focus-macos R8).
//!
//! Filing, markers, digests and reminders are the host's, and they run only
//! once a frontend switches Focus on. The GTK app and the terminal always
//! did; the Mac never had, so its inbox had no markers and nothing was ever
//! filed. These assert what a person would see: the header strip's counts.

use std::time::{Duration, Instant};

use chrono::Utc;
use postio_ffi::{Session, SessionOptions};
use postio_model::Message;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// A session over a store whose inbox holds one message asking `question`.
async fn asked(question: &str) -> std::sync::Arc<Session> {
    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs = postio_storage::BlobStore::open(scratch.path(), &test_support::blob_keys())
        .expect("a blob store");
    {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        // A question sent to the person, as the detector needs it: from
        // someone else, to the account's own address, recently.
        let mut message = Message::new(account.id, inbox, Utc::now() - chrono::TimeDelta::hours(1));
        message.subject = Some("Atlas Q3 budget".to_owned());
        message.date = Some(Utc::now() - chrono::TimeDelta::hours(1));
        message.from = vec![postio_model::EmailAddress::new(
            Some("Ada"),
            "ada@example.com",
        )];
        message.to = vec![account.address.clone()];
        message.promoted = Some(postio_model::promoted::PromotedHeaders::default());
        let id = repository.create(&mut message).await.expect("a message");
        // Threaded, as sync would leave it: a list row is a conversation,
        // and its unread state and marker are the conversation's.
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        let mut thread = postio_model::Thread::new(account.id);
        thread.subject = message.subject.clone();
        threads.create(&mut thread).await.expect("a thread");
        threads.add_message(thread.id, id).await.expect("threaded");
        repository
            .set_body(
                id,
                &StoredBody {
                    text: Some(question.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::message::BodyState::Full,
            )
            .await
            .expect("the body is stored");
    }
    Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
        .expect("a session")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_in_the_inbox_is_counted_as_needing_action() {
    let session = asked(
        "Hi,\n\nCan you approve these by Friday so finance can close the quarter?\n\nThanks,\nAda",
    )
    .await;
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
    let counts = loop {
        let counts = session.focus_counts().expect("the counts");
        if counts.has_action > 0 || Instant::now() > deadline {
            break counts;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(counts.conversations, 1, "{counts:?}");
    assert_eq!(
        counts.has_action, 1,
        "the question is marked, so Has action counts it: {counts:?}"
    );
    session.shutdown();
}

/// Wait for `wanted` on the session's event stream, up to `secs`.
pub(crate) async fn heard(
    session: &Session,
    secs: u64,
    mut wanted: impl FnMut(&postio_ffi::UiEvent) -> bool,
) -> bool {
    tokio::time::timeout(Duration::from_secs(secs), async {
        while let Some(event) = session.next_event().await {
            if wanted(&event) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}

/// The Mac's Focus list, filled by the controller's feed (spec 009 T026):
/// the inbox opens, is counted, changes over, and its rows carry the words
/// a row draws -- the marker's chip and the sentence quoted verbatim.
#[tokio::test(flavor = "multi_thread")]
async fn the_inbox_fills_with_focus_rows_that_carry_their_marker() {
    let session = asked(
        "Hi,\n\nCan you approve these by Friday so finance can close the quarter?\n\nThanks,\nAda",
    )
    .await;
    // The marker is the body stage's; wait for it before the list is read.
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
    while session.focus_counts().expect("counts").has_action == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    session.open_focus(postio_ffi::FocusScopeFfi::Inbox);
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            postio_ffi::UiEvent::FocusListChanged { total: 1 }
        ))
        .await,
        "the list changes over to the inbox, one conversation long"
    );
    assert_eq!(session.focus_row_count(), 1);
    // The first ask is a miss that fetches; the page then lands.
    let first = session.focus_row_at(0);
    if first.is_none() {
        assert!(
            heard(&session, 10, |event| matches!(
                event,
                postio_ffi::UiEvent::FocusPageReady { page: 0 }
            ))
            .await,
            "the first page lands"
        );
    }
    let row = session
        .focus_row_at(0)
        .expect("the row, once its page landed");
    assert_eq!(row.subject, "Atlas Q3 budget");
    assert_eq!(row.sender, "Ada");
    assert!(row.unread, "unread is bold");
    // The heading of the day the message arrived (an hour ago), whichever
    // side of midnight this runs on: the words are the presenter's.
    let arrived = (Utc::now() - chrono::TimeDelta::hours(1))
        .with_timezone(&chrono::Local)
        .date_naive();
    assert_eq!(
        row.day_heading,
        postio_ui::focus_row::day_heading(arrived, chrono::Local::now().date_naive())
    );
    let marker = row.marker.expect("the question's marker line");
    assert_eq!(marker.chip, "Question");
    assert_eq!(
        marker.quote.as_deref(),
        Some("Can you approve these by Friday so finance can close the quarter?"),
        "quoted verbatim"
    );
    assert!(
        marker
            .actions
            .iter()
            .any(|action| action.command == "reply"),
        "a question is answered by Reply: {:?}",
        marker.actions
    );
    session.shutdown();
}

/// The header strip's words cross composed, by the functions GTK's strip
/// uses (spec 009 FR-004): Swift words nothing itself.
#[test]
fn the_strip_says_what_gtk_s_strip_says() {
    let session = Session::open(SessionOptions::in_memory().with_config_for_test(
        "[focus]\nfiltering = true\n\n[[focus.digests]]\nname = \"Newsletters\"\nqueries = [\"from:news@ledger.example\"]\ncadence = \"weekly\"\n",
    ))
    .expect("a session");
    let strip = session.focus_strip().expect("the strip");
    assert_eq!(strip.counts, "0");
    assert_eq!(strip.has_action, "Has action \u{b7} 0");
    assert_eq!(
        strip.filtered_today, None,
        "nothing filtered says nothing (C10)"
    );
    assert_eq!(strip.digest_rules.as_deref(), Some("1 digest rule"));
    session.shutdown();

    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    assert_eq!(
        session.focus_strip().expect("the strip").digest_rules,
        None,
        "no rules, no count (C10)"
    );
    session.shutdown();
}

/// A session whose inbox holds one conversation per subject, newest first.
async fn inbox_of(subjects: &[&str]) -> std::sync::Arc<Session> {
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        for (age, subject) in subjects.iter().enumerate() {
            let at = Utc::now() - chrono::TimeDelta::minutes(age as i64 + 1);
            let mut message = Message::new(account.id, inbox, at);
            message.subject = Some((*subject).to_owned());
            message.date = Some(at);
            message.from = vec![postio_model::EmailAddress::new(
                Some("Ada"),
                "ada@example.com",
            )];
            let id = repository.create(&mut message).await.expect("a message");
            let mut thread = postio_model::Thread::new(account.id);
            thread.subject = message.subject.clone();
            threads.create(&mut thread).await.expect("a thread");
            threads.add_message(thread.id, id).await.expect("threaded");
        }
    }
    Session::open(SessionOptions::in_memory_with(database)).expect("a session")
}

/// The Mac's list keys run through the controller (spec 009 T045): `x`
/// selects without moving the cursor, `j` moves it, and `r` marks what is
/// selected read -- and the host's own words come back as the toast. (Mark
/// read rather than archive: the fixture account has no archive folder, and
/// the host would only say so.)
#[tokio::test(flavor = "multi_thread")]
async fn list_keys_run_through_the_controller() {
    use postio_ffi::UiEvent;
    let session = inbox_of(&["First", "Second", "Third"]).await;
    session.open_focus(postio_ffi::FocusScopeFfi::Inbox);
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusCursor { position: 0, .. }
        ))
        .await,
        "the list opens with the cursor on its first row (C30)"
    );

    session.invoke("toggle_selection");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusSelection { selected, .. } if selected.len() == 1
        ))
        .await,
        "x selects the cursor's row"
    );
    session.invoke("next_message");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusCursor { position: 1, .. }
        ))
        .await,
        "j moves the cursor"
    );
    assert_eq!(session.undo_description(), None, "nothing done yet");
    session.invoke("toggle_read");
    let mut toast = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusToast {
                text,
                undoable: true,
                ..
            } if !text.is_empty() => {
                toast = Some(text.clone());
                true
            }
            _ => false,
        })
        .await,
        "the host's words are the toast, and Undo can take it back"
    );
    assert_eq!(
        session.undo_description(),
        toast,
        "Edit > Undo names what the toast said"
    );
    session.shutdown();
}

/// The pointer drives the same cursor and selection the keys do: a click
/// moves the cursor, a Command-click toggles, a Shift-click takes the range.
#[tokio::test(flavor = "multi_thread")]
async fn clicks_reach_the_controller() {
    use postio_ffi::UiEvent;
    let session = inbox_of(&["First", "Second", "Third", "Fourth"]).await;
    cursor_on_the_first_row(&session).await;

    session.focus_point(2);
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusCursor { position: 2, .. }
        ))
        .await,
        "a click puts the cursor on its row"
    );
    session.focus_pick(1, false);
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusSelection { selected, .. } if selected.len() == 1
        ))
        .await,
        "a Command-click takes its row in"
    );
    session.focus_pick(3, true);
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusSelection { selected, .. } if selected.len() == 3
        ))
        .await,
        "a Shift-click takes the rows from the anchor to it"
    );
    session.focus_at_top(true);
    session.shutdown();
}

/// Return opens the cursor's message, `j` in it steps the list behind it,
/// and Back closes it with the keyboard back on the list (T058, T062).
#[tokio::test(flavor = "multi_thread")]
async fn the_open_message_is_the_controllers() {
    use postio_ffi::UiEvent;
    let session = inbox_of(&["First", "Second", "Third"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("open_message");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenMessage {
                index: 0,
                total: 3,
                ..
            }
        ))
        .await,
        "Return opens the first row's message"
    );
    session.focus_surface_opened(postio_ffi::SurfaceKindFfi::Message);
    session.invoke("find_in_message");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusReader {
                verb: postio_ffi::ReaderVerbFfi::FindInMessage
            }
        ))
        .await,
        "find is the message's"
    );
    session.invoke("next_message");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenMessage { index: 1, .. }
        ))
        .await,
        "j steps the list and the message follows"
    );
    session.invoke("back");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusCloseSurface {
                kind: postio_ffi::SurfaceKindFfi::Message
            }
        ))
        .await,
        "Back closes the message"
    );
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusKeyboardHome
        ))
        .await,
        "and the keyboard goes home"
    );
    session.shutdown();
}

/// `!` narrows the list through the controller, and a verb the host aims
/// resolves against the narrowed list, not the inbox it came from.
#[tokio::test(flavor = "multi_thread")]
async fn has_action_is_the_list_a_verb_aims_at() {
    use postio_ffi::UiEvent;
    let session = inbox_of(&["First", "Second"]).await;
    cursor_on_the_first_row(&session).await;
    assert!(!session.focus_aims_at_has_action_for_test());
    session.invoke("toggle_has_action");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusHeading { text: Some(_) }
        ))
        .await,
        "the list narrows under one heading"
    );
    assert!(session.focus_aims_at_has_action_for_test());
    session.shutdown();
}

/// Open Focus's inbox and wait until the cursor is on its first row: where
/// a verb with nothing selected acts, as a person would find it.
pub(crate) async fn cursor_on_the_first_row(session: &Session) {
    session.open_focus(postio_ffi::FocusScopeFfi::Inbox);
    assert!(
        heard(session, 10, |event| matches!(
            event,
            postio_ffi::UiEvent::FocusCursor { position: 0, .. }
        ))
        .await,
        "the inbox opened with the cursor on its first row"
    );
}

/// A session whose inbox holds one conversation of two messages: an earlier
/// one from Grace, and the latest, from Ada, asking a question, copied to
/// Grace, labelled Harbor and carrying a PDF. Answers the session and the
/// two messages, earlier first.
async fn a_thread_with_a_question() -> (std::sync::Arc<Session>, i64, i64) {
    use postio_model::EmailAddress;
    use postio_storage::repository::LabelRepository;

    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs = postio_storage::BlobStore::open(scratch.path(), &test_support::blob_keys())
        .expect("a blob store");
    let (earlier, latest) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        let mut thread = postio_model::Thread::new(account.id);
        thread.subject = Some("Harbor API draft v3".to_owned());
        threads.create(&mut thread).await.expect("a thread");

        let at = Utc::now() - chrono::TimeDelta::hours(3);
        let mut first = Message::new(account.id, inbox, at);
        first.subject = Some("Harbor API draft v3".to_owned());
        first.date = Some(at);
        first.flags = [postio_model::Flag::Seen].into_iter().collect();
        first.from = vec![EmailAddress::new(Some("Grace Okafor"), "grace@example.com")];
        first.to = vec![account.address.clone()];
        let earlier = repository.create(&mut first).await.expect("a message");
        threads
            .add_message(thread.id, earlier)
            .await
            .expect("threaded");

        let at = Utc::now() - chrono::TimeDelta::hours(1);
        let mut second = Message::new(account.id, inbox, at);
        second.subject = Some("Re: Harbor API draft v3".to_owned());
        second.date = Some(at);
        second.from = vec![EmailAddress::new(Some("Ada Moreno"), "ada@example.com")];
        second.to = vec![account.address.clone()];
        second.cc = vec![EmailAddress::new(Some("Grace Okafor"), "grace@example.com")];
        second.promoted = Some(postio_model::promoted::PromotedHeaders::default());
        let mut part = postio_model::Attachment::new(
            postio_model::MessageId::UNASSIGNED,
            "application/pdf",
            48_000,
        );
        part.filename = Some("draft-v3.pdf".to_owned());
        part.part_id = Some("2".to_owned());
        second.attachments = vec![part];
        let latest = repository.create(&mut second).await.expect("a message");
        threads
            .add_message(thread.id, latest)
            .await
            .expect("threaded");
        repository
            .set_body(
                latest,
                &StoredBody {
                    text: Some(
                        "Hi,\n\nCan you approve these by Friday so finance can close the quarter?\n\nThanks,\nAda"
                            .to_owned(),
                    ),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::message::BodyState::Full,
            )
            .await
            .expect("the body is stored");

        let labels = LabelRepository::new(&connection);
        let mut label = postio_model::Label::new(account.id, "Harbor");
        let label = labels.create(&mut label).await.expect("a label");
        labels.attach(latest, label).await.expect("labelled");
        (earlier.get(), latest.get())
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session");
    (session, earlier, latest)
}

/// What surrounds the body in the Mac's message window comes across
/// composed, by the functions GTK's open message draws with (spec 009
/// T067, T068): the subject, the position line, the thread chip and the
/// messages `[` and `]` reach, the labels, the sender block, the marker's
/// card, the action row with what folds into More, and the attachments.
#[tokio::test(flavor = "multi_thread")]
async fn the_message_window_s_chrome_is_composed_in_rust() {
    use postio_core::CommandId;
    use postio_ui::focus_dialog;

    let (session, earlier, latest) = a_thread_with_a_question().await;
    // The marker is the body stage's; wait for it before the list is read.
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
    while session.focus_counts().expect("counts").has_action == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    cursor_on_the_first_row(&session).await;
    if session.focus_row_at(0).is_none() {
        assert!(
            heard(&session, 10, |event| matches!(
                event,
                postio_ffi::UiEvent::FocusPageReady { page: 0 }
            ))
            .await,
            "the first page lands"
        );
    }
    assert_eq!(session.focus_row_at(0).map(|row| row.id), Some(latest));

    let view = session.focus_message_view(latest, 0, 1);
    assert_eq!(view.message, latest);
    assert_eq!(view.subject, "Re: Harbor API draft v3");
    assert_eq!(
        view.position,
        focus_dialog::position_line(0, 1, 2, 1, true, None),
        "the row's place, and the conversation's size at its latest"
    );
    let chip = view.thread.expect("a conversation of two has a chip");
    assert_eq!(
        Some(chip.text.clone()),
        focus_dialog::thread_chip_at(2, 1, true)
    );
    let earlier_key = chip.earlier.expect("the latest offers the earlier one");
    assert_eq!(
        earlier_key.command,
        CommandId::PrevInConversation.to_string()
    );
    assert_eq!(earlier_key.label, focus_dialog::EARLIER_MESSAGE);
    assert!(chip.later.is_none(), "nothing is later than the latest");
    assert_eq!(view.earlier, Some(earlier), "what [ shows");
    assert_eq!(view.later, None, "what ] shows");

    assert_eq!(
        view.labels
            .iter()
            .map(|pill| pill.name.as_str())
            .collect::<Vec<_>>(),
        ["Harbor"]
    );
    assert_eq!(view.add_label.command, CommandId::AddLabel.to_string());
    assert_eq!(view.add_label.label, focus_dialog::ADD_LABEL);

    let fields: Vec<&str> = view
        .fields
        .iter()
        .map(|field| field.field.as_str())
        .collect();
    assert_eq!(
        fields,
        [
            focus_dialog::FIELD_FROM,
            focus_dialog::FIELD_TO,
            focus_dialog::FIELD_CC
        ]
    );
    let from = &view.fields[0].people[0];
    assert_eq!(from.name.as_deref(), Some("Ada Moreno"));
    assert_eq!(from.address, "ada@example.com");
    assert_eq!(
        view.fields[2].people[0].name.as_deref(),
        Some("Grace Okafor")
    );
    assert!(!view.date.is_empty(), "the sender block is dated");

    let marker = view.marker.expect("the question's card");
    assert_eq!(marker.chip, "Question");
    assert_eq!(
        marker.quote.as_deref(),
        Some("Can you approve these by Friday so finance can close the quarter?")
    );
    assert_eq!(view.dismiss.command, CommandId::DismissMarker.to_string());
    assert_eq!(view.dismiss.label, focus_dialog::DISMISS);

    let commands: Vec<&str> = view
        .actions
        .iter()
        .map(|verb| verb.command.as_str())
        .collect();
    let expected: Vec<String> = focus_dialog::OPEN_TOOLBAR
        .iter()
        .filter(|verb| verb.command != CommandId::MoreActions)
        .map(|verb| verb.command.to_string())
        .collect();
    assert_eq!(commands, expected, "screen 04's row, in its order");
    let folded: Vec<String> = view
        .actions
        .iter()
        .filter(|verb| verb.folds)
        .map(|verb| verb.command.clone())
        .collect();
    assert_eq!(
        folded,
        focus_dialog::FOLDED.map(|command| command.to_string()),
        "Label, Move and Delete fold into More when narrow"
    );
    let more = view.more.expect("received mail has More");
    assert_eq!(more.command, CommandId::MoreActions.to_string());

    assert_eq!(view.attachments.len(), 1, "{:?}", view.attachments);
    assert_eq!(view.attachments[0].name, "draft-v3.pdf");
    assert_eq!(
        view.attachments[0].size,
        postio_ui::format::human_size(48_000)
    );

    // `[`: the earlier message, in the same window. Its place in the
    // conversation is said, it has no card (the marker is the latest's),
    // and `]` comes back.
    let stepped = session.focus_message_view(earlier, 0, 1);
    assert_eq!(
        stepped.position,
        focus_dialog::position_line(0, 1, 2, 0, false, None)
    );
    assert!(stepped.marker.is_none(), "the card is the marked message's");
    assert_eq!(stepped.earlier, None);
    assert_eq!(stepped.later, Some(latest));
    let chip = stepped.thread.expect("still a conversation");
    assert!(chip.later.is_some(), "] is offered from an earlier message");
    session.shutdown();
}
