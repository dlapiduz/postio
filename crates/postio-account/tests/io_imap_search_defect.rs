//! What `io-imap` 0.6.0 does with a SEARCH result split across lines.
//!
//! **This pins a defect in a dependency, not behaviour Postio wants.** It
//! asserts what the current pinned `io-imap` actually does, so that when a
//! future version fixes it these tests fail — and failing is the signal to
//! delete both them and the workaround in `imap::fetch::existing_uids`.
//!
//! # The defect
//!
//! `rfc3501/search.rs` collects the untagged responses of a SEARCH like this:
//!
//! ```ignore
//! let mut ids = Vec::new();
//! for data in out.data {
//!     if let Data::Search(search_ids, _) = data {
//!         ids = search_ids;          // assignment, not extend
//!     }
//! }
//! ```
//!
//! A server is free to answer one SEARCH with several untagged `* SEARCH`
//! lines, and servers do exactly that for results too large to sit on one
//! line. Every line but the last is silently discarded, and a final line
//! carrying no numbers discards the lot.
//!
//! `rfc5256/sort.rs` (`ids = Some(sort_ids)`) and `rfc5256/thread.rs`
//! (`threads = Some(t)`) have the same shape and presumably the same defect.
//! Postio does not call either yet.
//!
//! # What it cost
//!
//! Measured against iCloud on 2026-09-17, syncing a real account:
//!
//! ```text
//! the server listed fewer UIDs than it says the mailbox holds
//!   mailbox=2   listed=0  exists=60934    Archive
//!   mailbox=13  listed=0  exists=20324    Sent Messages
//!   mailbox=7   listed=0  exists=830      Deleted Messages
//! ```
//!
//! An empty listing is indistinguishable from an empty mailbox, so the
//! enumeration fetched nothing, completed, and stamped the mailbox as fully
//! synchronised. Small mailboxes were unaffected: INBOX's 173 messages fit in
//! one untagged line and synced correctly.
//!
//! # The other way to the same symptom
//!
//! `io-imap` also drops any untagged response it *cannot decode*, completing
//! the command `Ok` (see `imap::skip_counter`). A `* SEARCH` line that fails
//! to parse therefore removes its UIDs just as quietly. Both paths end in an
//! empty listing, and `existing_uids` now refuses on either.

use io_imap::codec::fragmentizer::Fragmentizer;
use io_imap::coroutine::{ImapCoroutine, ImapCoroutineState, ImapYield};
use io_imap::rfc3501::search::{ImapMessageSearch, ImapMessageSearchOptions};
use io_imap::types::core::Vec1;
use io_imap::types::search::SearchKey;

/// Drives a real `ImapMessageSearch` against a canned server response.
fn run_search(untagged: &[&str]) -> Vec<u32> {
    let mut fragmentizer = Fragmentizer::new(50 * 1024 * 1024);
    let mut coroutine = ImapMessageSearch::new(
        Vec1::from(SearchKey::All),
        ImapMessageSearchOptions { uid: true },
    );
    let mut feed: Option<Vec<u8>> = None;
    let mut arg: Option<Vec<u8>> = None;

    loop {
        let slice = arg.take();
        match coroutine.resume(&mut fragmentizer, slice.as_deref()) {
            ImapCoroutineState::Yielded(ImapYield::WantsWrite(bytes)) => {
                // The command carries the tag its response has to echo.
                let sent = String::from_utf8_lossy(&bytes).to_string();
                let tag = sent.split_whitespace().next().unwrap_or("A1").to_owned();
                let mut response = String::new();
                for line in untagged {
                    response.push_str(line);
                    response.push_str("\r\n");
                }
                response.push_str(&format!("{tag} OK SEARCH completed\r\n"));
                feed = Some(response.into_bytes());
            }
            ImapCoroutineState::Yielded(ImapYield::WantsRead) => {
                arg = Some(feed.take().expect("the response is fed once"));
            }
            ImapCoroutineState::Complete(Ok(ids)) => {
                return ids.into_iter().map(|id| id.get()).collect();
            }
            ImapCoroutineState::Complete(Err(error)) => panic!("SEARCH failed: {error}"),
        }
    }
}

/// The case that works, and the reason small mailboxes were fine.
#[test]
fn one_untagged_line_is_read_whole() {
    assert_eq!(run_search(&["* SEARCH 1 2 3"]), vec![1, 2, 3]);
}

/// Every line but the last is discarded.
///
/// When this starts failing, `io-imap` has been fixed: the right answer is
/// `[1, 2, 3, 4, 5, 6]`.
#[test]
fn only_the_last_untagged_line_survives() {
    assert_eq!(
        run_search(&["* SEARCH 1 2 3", "* SEARCH 4 5 6"]),
        vec![4, 5, 6],
        "io-imap assigns rather than extends, so the first line is lost"
    );
}

/// A trailing line with no numbers discards the whole result.
///
/// This is the one that reached a user: `listed=0` against a mailbox holding
/// 60,934 messages. When this starts failing, the workaround in
/// `existing_uids` can go.
#[test]
fn a_final_empty_line_discards_everything() {
    assert_eq!(
        run_search(&["* SEARCH 1 2 3", "* SEARCH"]),
        Vec::<u32>::new(),
        "an empty last line overwrites every id the server sent"
    );
}
