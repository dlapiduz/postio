use io_imap::codec::fragmentizer::Fragmentizer;
use io_imap::coroutine::{ImapCoroutine, ImapCoroutineState, ImapYield};
use io_imap::rfc3501::search::{ImapMessageSearch, ImapMessageSearchOptions};
use io_imap::types::core::Vec1;
use io_imap::types::search::SearchKey;

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
            ImapCoroutineState::Yielded(ImapYield::WantsTag) => {
                arg = Some(b"A1".to_vec());
            }
            ImapCoroutineState::Yielded(ImapYield::WantsWrite(bytes)) => {
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
                return ids.data.into_iter().map(|id| id.get()).collect();
            }
            ImapCoroutineState::Complete(Err(error)) => panic!("SEARCH failed: {error}"),
        }
    }
}

#[test]
fn one_untagged_line_is_read_whole() {
    assert_eq!(run_search(&["* SEARCH 1 2 3"]), vec![1, 2, 3]);
}

/// Pins an io-imap defect: when this fails, io-imap is fixed and the `existing_uids` workaround can
/// go.
#[test]
fn only_the_last_untagged_line_survives() {
    assert_eq!(
        run_search(&["* SEARCH 1 2 3", "* SEARCH 4 5 6"]),
        vec![4, 5, 6],
        "io-imap assigns rather than extends, so the first line is lost"
    );
}

/// A trailing line with no numbers discards the whole result: `listed=0` against a 60,934-message
/// mailbox.
#[test]
fn a_final_empty_line_discards_everything() {
    assert_eq!(
        run_search(&["* SEARCH 1 2 3", "* SEARCH"]),
        Vec::<u32>::new(),
        "an empty last line overwrites every id the server sent"
    );
}
