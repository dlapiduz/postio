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
    assert!(costs[0] <= 4, "the read took {} statements", costs[0]);
}

// -- What Swift receives when a thread opens (spec 013 T028) ----------------

use postio_ffi::{FocusScopeFfi, Session, SessionOptions, UiEvent};

use super::focus::heard;

/// The inbox over the threads seed, landed, with Harbor's row found.
async fn inbox_with_harbor() -> (Arc<Session>, u32) {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Threads).await;
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
