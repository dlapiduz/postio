//! A thread, read whole by one request (spec 013 T015, contracts/engine-threads.md).
//!
//! `Req::Thread` answers a window or an expansion with the members in date
//! order, each with its own line, the unsent reply, and the labels. It costs
//! the same number of statements whatever the thread's length (the budget is a
//! count, not a clock).

use std::sync::Arc;

use postio_client::protocol::ClientKind;
use postio_host::Host;
use postio_model::{Line, ThreadId};
use postio_storage::Store;
use postio_storage::sql::RowExt;
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

/// The thread whose subject is `subject`.
async fn thread_named(database: &Store, subject: &str) -> ThreadId {
    let connection = database.connect().await.expect("a connection");
    postio_storage::sql::first(
        &connection,
        "SELECT id FROM threads WHERE lower(subject) = lower(?1)",
        [subject.to_owned()],
        |row| Ok(ThreadId::new(row.col(0)?)),
    )
    .await
    .expect("a read")
    .unwrap_or_else(|| panic!("the seed has a thread called {subject:?}"))
}

/// The thread with `count` messages.
async fn thread_of(database: &Store, count: i64) -> ThreadId {
    let connection = database.connect().await.expect("a connection");
    postio_storage::sql::first(
        &connection,
        "SELECT id FROM threads WHERE message_count = ?1",
        [count],
        |row| Ok(ThreadId::new(row.col(0)?)),
    )
    .await
    .expect("a read")
    .unwrap_or_else(|| panic!("the seed has a thread of {count}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_reads_whole_with_its_answers_and_its_draft() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Threads).await;
    let harbor = thread_named(&database, "Harbor API draft v3").await;
    let blobs = tempfile::tempdir().expect("a directory");
    let host = Host::start(
        database.clone(),
        postio_storage::BlobStore::open(blobs.path().to_owned(), &test_support::blob_keys())
            .expect("a blob store"),
        |wiring| wiring.with_secrets(Arc::new(postio_account::secret::MemorySecretStore::new())),
    )
    .expect("a host");
    let client = host.connect(ClientKind::Test);

    let view = client
        .thread(harbor)
        .await
        .expect("a read")
        .expect("Harbor is there");

    assert_eq!(view.thread, harbor);
    assert_eq!(
        view.subject.as_deref().map(str::to_lowercase),
        Some("harbor api draft v3".to_owned())
    );
    assert_eq!(view.members.len(), 6, "every member, whatever folder");
    assert_eq!(
        view.members[3].answers,
        Some(0),
        "message 4 answers message 1"
    );
    assert!(
        view.members.windows(2).all(|pair| pair[0].at <= pair[1].at),
        "oldest first"
    );
    assert!(
        view.members
            .iter()
            .all(|member| !matches!(&member.line, Line::Own(line) if line.is_empty())),
        "an empty own line is `Quoted`"
    );
    let draft = view.draft.expect("Harbor has an unsent reply");
    assert!(draft.first_words.starts_with("Thanks both."));
    assert_eq!(view.muted, None, "mute arrives with US5");
    assert_eq!(
        client.counts().of("Thread"),
        1,
        "one round trip, counted by its family"
    );

    assert_eq!(
        client
            .thread(ThreadId::new(1_000_000))
            .await
            .expect("a read"),
        None,
        "a thread that is gone is none, not a failure"
    );
    drop(client);
    drop(host);
}

/// The same statements at six messages as at forty-one. Counted on this
/// thread, so the host's answer is called directly, as the host calls it.
#[tokio::test]
async fn reading_a_thread_costs_the_same_at_any_length() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Threads).await;
    let harbor = thread_named(&database, "Harbor API draft v3").await;
    let long = thread_of(&database, 41).await;
    // Warm the connection's own one-time work so both counts are the read.
    let _ = postio_host::threads::view(&database, harbor).await;

    let mut costs = Vec::new();
    for (thread, members) in [(harbor, 6), (long, 41)] {
        let mut read = None;
        let counts = counted_async(async || {
            read = Some(
                postio_host::threads::view(&database, thread)
                    .await
                    .expect("a read"),
            );
        })
        .await;
        let view = read.flatten().expect("the thread is there");
        assert_eq!(view.members.len(), members);
        costs.push(counts.statements);
    }

    assert_eq!(
        costs[0], costs[1],
        "six messages cost {} statements and forty-one cost {}",
        costs[0], costs[1]
    );
    // Five: the members, the draft, the subject, the labels, and whether
    // the thread is muted (US5). Equal at both lengths above is the budget
    // that matters; this cap catches a read that grows a statement.
    assert!(costs[0] <= 5, "the read took {} statements", costs[0]);
}

// -- What Swift receives when a thread opens (spec 013 T028) ----------------

use postio_ffi::{FocusScopeFfi, Session, SessionOptions, UiEvent};

use super::focus::heard;

/// The inbox over the threads seed, landed, with Harbor's row found.
async fn inbox_with_harbor() -> (Arc<Session>, u32) {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Threads).await;
    inbox_with_harbor_over(database).await
}

/// The same over a store the caller keeps a handle on.
async fn inbox_with_harbor_over(database: Store) -> (Arc<Session>, u32) {
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.open_focus(FocusScopeFfi::Inbox);
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusListChanged { .. }
        ))
        .await,
        "the inbox changes over"
    );
    let total = session.focus_row_count();
    for position in 0..total {
        while session.focus_row_at(position).is_none() {
            assert!(
                heard(&session, 10, |event| matches!(
                    event,
                    UiEvent::FocusPageReady { .. }
                ))
                .await,
                "the page of row {position} lands"
            );
        }
    }
    let harbor = (0..total)
        .find(|position| {
            session
                .focus_row_at(*position)
                .is_some_and(|row| row.subject.to_lowercase().contains("harbor api draft v3"))
        })
        .expect("Harbor is in the inbox");
    (session, harbor)
}

#[tokio::test(flavor = "multi_thread")]
async fn expanding_a_thread_hands_swift_its_children_and_counts_them_in_the_cursor() {
    let (session, harbor) = inbox_with_harbor().await;
    let rows = session.focus_row_count();
    session.focus_point(harbor);
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusCursor { position, .. } if *position == harbor
        ))
        .await,
        "the cursor is on Harbor"
    );

    session.invoke("expand_thread");
    let mut spliced = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusSplice { anchor, children } => {
                spliced = Some((*anchor, children.clone()));
                true
            }
            _ => false,
        })
        .await,
        "Swift is told to insert the children"
    );
    let (anchor, children) = spliced.expect("a splice");
    assert_eq!(anchor, harbor, "after Harbor's row");
    assert_eq!(children.len(), 7, "six messages and the unsent reply");
    assert!(
        children[..6]
            .iter()
            .all(|child| !child.draft && !child.line.is_empty()),
        "every message has its own line: {children:?}"
    );
    assert_eq!(
        children[3].tag.as_deref(),
        Some("\u{21b3} to Ana"),
        "message 4 answers Ana's"
    );
    let draft = &children[6];
    assert!(draft.draft, "the draft is last");
    assert!(draft.line.starts_with("Thanks both."), "{draft:?}");

    assert_eq!(
        session.focus_row_count(),
        rows + 7,
        "the table draws the children"
    );
    let child = session
        .focus_row_at(harbor + 1)
        .expect("a child sits right under the row");
    assert_eq!(
        child.child.as_ref().map(|c| c.line.clone()),
        Some(children[0].line.clone())
    );
    assert_eq!(
        session
            .focus_row_at(harbor)
            .and_then(|row| row.thread_row)
            .and_then(|t| t.chevron),
        Some(postio_ffi::ChevronFfi::Expanded)
    );

    // `j` walks into the children: the cursor's position counts them.
    session.invoke("next_message");
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusCursor { position, .. } if *position == harbor + 1
        ))
        .await,
        "the cursor steps onto the first child, display position {}",
        harbor + 1
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chevron_click_opens_the_thread_and_the_next_one_folds_it_and_the_cursor_stays_on_the_row()
 {
    let (session, harbor) = inbox_with_harbor().await;
    let rows = session.focus_row_count();

    session.focus_chevron_clicked(harbor);
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusSplice { anchor, .. } if *anchor == harbor
        ))
        .await,
        "the chevron opens the thread"
    );
    session.invoke("next_message");
    session.invoke("next_message");
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusCursor { position, .. } if *position == harbor + 2
        ))
        .await,
        "the cursor walks the children"
    );

    session.invoke("collapse_thread");
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusUnsplice { anchor } if *anchor == harbor
        ))
        .await,
        "Left folds it"
    );
    assert_eq!(session.focus_row_count(), rows, "the children are gone");
    assert_eq!(
        session
            .focus_row_at(harbor)
            .and_then(|row| row.thread_row)
            .and_then(|thread| thread.chevron),
        Some(postio_ffi::ChevronFfi::Collapsed)
    );
    session.shutdown();
}

// -- The same, in Filtered (spec 013 T035, FR-009) ---------------------------

/// The Threads seed with one of Harbor's messages filed away as a
/// notification, and Filtered open over it.
async fn filtered_with_harbor() -> (Arc<Session>, u32) {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Threads).await;
    let harbor = thread_named(&database, "Harbor API draft v3").await;
    let connection = database.connect().await.expect("a connection");
    let message = postio_storage::sql::first(
        &connection,
        "SELECT id FROM messages WHERE thread_id = ?1 ORDER BY id LIMIT 1",
        [harbor.get()],
        |row| Ok(postio_model::MessageId::new(row.col(0)?)),
    )
    .await
    .expect("a read")
    .expect("Harbor has a message");
    postio_storage::repository::FilterDecisionRepository::new(&connection)
        .record(&postio_storage::repository::FilterDecision {
            message,
            reason: postio_storage::repository::FilterReason::Promotion,
            source: None,
            layer: postio_storage::repository::FilterLayer::Header,
            decided_at: chrono::Utc::now(),
        })
        .await
        .expect("a decision");
    drop(connection);
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.invoke("go_to_filtered");
    let mut row = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusFiltered { view } if !view.rows.is_empty() => {
                row = view
                    .rows
                    .iter()
                    .position(|line| line.message == message.get());
                if let Some(at) = row {
                    assert_eq!(view.rows[at].messages, 6, "the row says it is a thread");
                }
                row.is_some()
            }
            _ => false,
        })
        .await,
        "Filtered lists Harbor's filed message"
    );
    (session, row.expect("a row") as u32)
}

#[tokio::test(flavor = "multi_thread")]
async fn expanding_a_filtered_thread_hands_swift_its_children_and_a_redraw_keeps_them() {
    let (session, harbor) = filtered_with_harbor().await;
    session.focus_filtered_point(harbor);
    session.invoke("expand_thread");
    let mut spliced = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusFilteredSplice { anchor, children } => {
                spliced = Some((*anchor, children.len()));
                true
            }
            _ => false,
        })
        .await,
        "Swift is told to insert the children under Filtered's row"
    );
    assert_eq!(
        spliced,
        Some((harbor, 7)),
        "six messages and the unsent reply"
    );

    // j walks into the children: the keyboard is a display position.
    session.invoke("next_message");
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusFilteredFocus { index: Some(index) } if *index == harbor + 1
        ))
        .await,
        "the first child has the keyboard"
    );

    session.invoke("collapse_thread");
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusFilteredUnsplice { anchor } if *anchor == harbor
        ))
        .await,
        "Left folds it"
    );
    session.shutdown();
}

// -- The footer under the list (spec 013 T040, FR-014) -----------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_footer_names_the_scope_the_keys_and_the_note_as_the_cursor_moves() {
    let (session, harbor) = inbox_with_harbor().await;
    session.focus_point(harbor);
    let mut collapsed = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusFooter { view } => {
                collapsed = Some(view.clone());
                true
            }
            _ => false,
        })
        .await,
        "Swift is handed a footer when the cursor lands on the row"
    );
    let collapsed = collapsed.expect("a footer");
    assert!(
        collapsed
            .title
            .to_lowercase()
            .contains("harbor api draft v3")
            && collapsed.title.ends_with("\u{b7} 6 messages"),
        "the row's footer names the thread and its size: {:?}",
        collapsed.title
    );

    session.invoke("expand_thread");
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusSplice { .. }
        ))
        .await,
        "the thread opens before the cursor walks it"
    );
    for _ in 0..4 {
        session.invoke("next_message");
    }
    let mut child = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusFooter { view } if view.title.starts_with("Message 4 of 6") => {
                child = Some(view.clone());
                true
            }
            _ => false,
        })
        .await,
        "a footer names message 4"
    );
    let child = child.expect("a footer");
    assert_eq!(child.title, "Message 4 of 6 \u{b7} Ben Adeyemi");
    assert!(
        child.keys.iter().any(|key| key.label == "reply to this"),
        "the keys say what Reply acts on: {:?}",
        child.keys
    );
    assert!(child.note.is_some(), "a child's footer carries the note");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn reply_on_a_child_answers_that_message_not_the_latest() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Threads).await;
    let harbor_thread = thread_named(&database, "Harbor API draft v3").await;
    let members = postio_host::threads::view(&database, harbor_thread)
        .await
        .expect("a read")
        .expect("Harbor is there")
        .members;
    let fourth = members[3].id.get();
    assert_ne!(fourth, members[5].id.get(), "message 4 is not the latest");
    let (session, harbor) = inbox_with_harbor_over(database).await;
    session.focus_point(harbor);
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusCursor { position, .. } if *position == harbor
        ))
        .await,
        "the cursor is on Harbor"
    );
    session.invoke("expand_thread");
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusSplice { .. }
        ))
        .await,
        "the thread opens"
    );
    for _ in 0..4 {
        session.invoke("next_message");
    }
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusCursor { position, .. } if *position == harbor + 4
        ))
        .await,
        "the cursor is on message 4"
    );
    session.invoke("reply");
    let mut opened = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusComposer { kind, message } => {
                opened = Some((*kind, *message));
                true
            }
            _ => false,
        })
        .await,
        "Reply opens the composer"
    );
    assert_eq!(
        opened,
        Some((postio_ffi::ComposerKindFfi::Reply, Some(fourth))),
        "the reply answers message 4"
    );
    session.shutdown();
}

// -- The message window's thread bar and walk (spec 013 T047, US3) -----------

/// Everything the window's thread intents said, as they arrive.
#[derive(Default, Debug)]
struct Walked {
    bar: Option<postio_ffi::ThreadBarFfi>,
    kicker: Option<String>,
    banner: Option<Option<postio_ffi::ThreadBannerFfi>>,
    reply_to: Option<Option<String>>,
    /// The Reply to row that closes the bar's four intents has arrived.
    whole: bool,
    opened: Vec<i64>,
}

/// Drain the event stream for what the walk said, until the bar shows the
/// ringed chip `current` (a 1-based message number) and all four intents
/// of that step have arrived.
async fn walked_to(session: &Session, walked: &mut Walked, current: u32) -> bool {
    heard(session, 10, |event| {
        match event {
            UiEvent::FocusThreadBar { view } => {
                walked.bar = Some(view.clone());
                walked.whole = false;
            }
            UiEvent::FocusKicker { text } => walked.kicker = Some(text.clone()),
            UiEvent::FocusOlderBanner { banner } => walked.banner = Some(banner.clone()),
            UiEvent::FocusReplyTo { text } => {
                walked.reply_to = Some(text.clone());
                walked.whole = true;
            }
            UiEvent::FocusOpenMessage { message, .. } => walked.opened.push(*message),
            _ => {}
        }
        walked.whole
            && walked.bar.as_ref().is_some_and(|bar| {
            bar.chips.iter().any(|chip| {
                matches!(chip, postio_ffi::ThreadChipFfi::Message { index, current: true, .. } if *index + 1 == current)
            })
        })
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_window_walks_a_thread_and_the_controller_says_what_to_draw() {
    let (session, harbor) = inbox_with_harbor().await;
    session.focus_point(harbor);
    session.invoke("open_message");
    let mut walked = Walked::default();
    assert!(
        walked_to(&session, &mut walked, 5).await,
        "the window opens on Harbor's open question (message 5), ringed: {walked:?}"
    );
    session.focus_surface_opened(postio_ffi::SurfaceKindFfi::Message);
    session.focus_bar_room(8);

    session.invoke("prev_in_conversation");
    assert!(
        walked_to(&session, &mut walked, 4).await,
        "`[` rings message 4: {walked:?}"
    );
    let bar = walked.bar.clone().expect("a bar");
    assert_eq!(bar.chips.len(), 6, "every member has a chip: {bar:?}");
    assert_eq!(bar.title, "Thread");
    let kicker = walked.kicker.clone().expect("a kicker");
    assert!(kicker.starts_with("Message 4 of 6 \u{b7} "), "{kicker:?}");
    assert!(
        walked.banner.clone().flatten().is_some(),
        "an older message says so: {:?}",
        walked.banner
    );
    let reply_to = walked.reply_to.clone().flatten().expect("a Reply to row");
    assert!(
        reply_to.starts_with("Ana Ruiz, ") && reply_to.ends_with("(message 1)"),
        "{reply_to:?}"
    );
    assert_eq!(
        walked.opened.len(),
        2,
        "the open and the step each show a message in place: {:?}",
        walked.opened
    );

    session.invoke("latest_in_conversation");
    assert!(
        walked_to(&session, &mut walked, 6).await,
        "`}}` goes to the latest"
    );
    assert_eq!(walked.banner, Some(None), "`}}` clears the banner");
    session.shutdown();
}

/// Opening a message window and closing it, again and again, as the Mac's
/// table does: each open draws the bar once, and nothing a draw or a reply
/// causes opens, reads or draws again (no feedback between the controller's
/// `ReadThread` and the frontend's echo of what it was shown).
#[tokio::test(flavor = "multi_thread")]
async fn forty_opens_of_a_thread_draw_its_bar_forty_times_and_no_more() {
    let (session, harbor) = inbox_with_harbor().await;
    let mut bars = 0_u32;
    let mut count = |event: &UiEvent| {
        if matches!(event, UiEvent::FocusThreadBar { .. }) {
            bars += 1;
        }
    };
    for open in 1..=40 {
        session.focus_point(harbor);
        session.invoke("open_message");
        let mut walked = Walked::default();
        let mut heard_bar = false;
        assert!(
            heard(&session, 10, |event| {
                count(event);
                if let UiEvent::FocusThreadBar { view } = event {
                    heard_bar = true;
                    walked.bar = Some(view.clone());
                }
                if matches!(event, UiEvent::FocusReplyTo { .. }) {
                    walked.whole = true;
                }
                heard_bar && walked.whole
            })
            .await,
            "open {open} drew no bar"
        );
        session.focus_surface_opened(postio_ffi::SurfaceKindFfi::Message);
        session.invoke("back");
        session.focus_surface_closed(postio_ffi::SurfaceKindFfi::Message);
    }
    // Whatever the last open set going has had a quarter of a second to say.
    while let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_millis(250), session.next_event()).await
    {
        count(&event);
    }
    assert_eq!(bars, 40, "one bar for each open, none from the replies");
    session.shutdown();
}
