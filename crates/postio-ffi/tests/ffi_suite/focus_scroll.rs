//! Scrolling Focus's list over ten thousand conversations, counted rather
//! than timed (specs/009-focus-macos T035, constitution V).
//!
//! A shared runner cannot defend 16 ms, so what is held is the cause of the
//! budget: the window keeps a bounded number of rows however far the list is
//! scrolled, a frame that misses reads the pages under it and no more, and a
//! walk from the first row to the last reads each page once. Each number is
//! the same on any machine. The table's half -- that it makes a screenful of
//! views and asks for the rows it draws -- is `FocusListScrollTests.swift`.
//!
//! POSTIO-MEASUREMENT: it seeds ten thousand conversations, which is ten
//! seconds of the file's eleven, so it runs on the nightly rather than on
//! every pull request. What it asserts is counts, never times.

use std::time::{Duration, Instant};

use postio_ffi::{FocusScopeFfi, Session, SessionOptions, UiEvent};
use postio_storage::seed::seed_conversations;
use postio_storage::test_support;
use postio_ui::list::{CACHE_PAGES, PAGE_SIZE};

const CONVERSATIONS: usize = 10_000;
/// Rows a window of the Mac shows at once, a little over the design's 19.
const VISIBLE: u32 = 20;

/// Wait for `wanted` on the session's event stream.
async fn heard(session: &Session, wanted: impl Fn(&UiEvent) -> bool) -> bool {
    tokio::time::timeout(
        postio_test_support::scaled(Duration::from_secs(30)),
        async {
            while let Some(event) = session.next_event().await {
                if wanted(&event) {
                    return true;
                }
            }
            false
        },
    )
    .await
    .unwrap_or(false)
}

/// Ask for every row of a screen, the way the table does, and wait for the
/// pages the misses started to land.
/// Until the session has said nothing for a quarter of a second.
async fn quiet(session: &Session) {
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(30));
    while Instant::now() < deadline {
        let said = tokio::time::timeout(Duration::from_millis(250), session.next_event()).await;
        if said.is_err() {
            return;
        }
    }
}

async fn draw(session: &Session, first: u32, count: u32) {
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(30));
    loop {
        let missing = (first..first + count)
            .filter(|position| session.focus_row_at(*position).is_none())
            .count();
        if missing == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{missing} rows of the screen at {first} never arrived"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn walking_ten_thousand_conversations_reads_each_page_once_and_holds_a_few() {
    let database = test_support::memory().await;
    seed_conversations(&database, 7, &vec![1; CONVERSATIONS]).await;
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");

    session.open_focus(FocusScopeFfi::Inbox);
    assert!(
        heard(&session, |event| matches!(
            event,
            UiEvent::FocusListChanged { total } if *total as usize == CONVERSATIONS
        ))
        .await,
        "the inbox is counted as ten thousand conversations"
    );
    assert_eq!(session.focus_row_count() as usize, CONVERSATIONS);
    // Let startup finish saying things. A store event re-reads every page
    // the list holds, in place (so a delete does not blank the list), and
    // one landing mid-walk is a refresh, not the scrolling this measures.
    quiet(&session).await;

    let total = CONVERSATIONS as u32;
    let pages = total.div_ceil(PAGE_SIZE) as usize;
    let mut position = 0;
    let mut most_in_one_frame = 0;
    let mut most_resident = 0;
    while position < total {
        let count = VISIBLE.min(total - position);
        let before = session.focus_page_reads_for_test();
        draw(&session, position, count).await;
        most_in_one_frame = most_in_one_frame.max(session.focus_page_reads_for_test() - before);
        most_resident = most_resident.max(session.focus_resident_rows_for_test());
        position += count;
    }

    // However far it scrolled, the window held its cache and no more.
    let bound = CACHE_PAGES * PAGE_SIZE as usize;
    assert!(
        most_resident <= bound,
        "the window held {most_resident} rows; {CACHE_PAGES} pages is {bound}"
    );
    // A frame reads the page it is on and, at a boundary, the next one.
    assert!(
        most_in_one_frame <= 2,
        "one screen of {VISIBLE} rows read {most_in_one_frame} pages"
    );
    // And the walk read each page of the list about once: not once per row
    // asked for, which would be ten thousand reads.
    let reads = session.focus_page_reads_for_test();
    assert!(
        reads <= pages + 2,
        "walking {pages} pages read {reads} times"
    );
    session.shutdown();
}
