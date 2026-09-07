//! A maildir on this machine, through the seam every other backend uses.
//!
//! The acceptance criteria of #1278 are all here: a `~/mail` tree opens with
//! no server, a second pass over it does not import the mail again, and a
//! directory that is not a maildir says so instead of appearing as an empty
//! account. The rest is the seam's own contract — that this backend answers
//! the same questions the IMAP one does, in the same shapes.

use std::path::{Path, PathBuf};

use postio_account::backend::{
    AppendMessage, BackendError, BodyPart, Capability, FlagChange, MailBackend, MailboxFilter,
    SelectMode, UidSet, VecSink,
};
use postio_account::cancel::CancelToken;
use postio_account::maildir::MaildirBackend;
use postio_model::{Flag, FlagSet, RemoteId, Uid};
use tempfile::TempDir;

/// A tree with an inbox and an archive, and three messages in the inbox.
fn tree() -> TempDir {
    let tree = tempfile::tempdir().expect("a temporary directory");
    make_maildir(tree.path());
    make_maildir(&tree.path().join("Archive"));

    deliver(
        tree.path(),
        "1770000001.M1.host",
        "S",
        "ada@example.com",
        "Ada Lovelace",
        "The analytical engine",
        "<note-1@example.com>",
    );
    deliver(
        tree.path(),
        "1770000002.M2.host",
        "",
        "grace@example.net",
        "Grace Hopper",
        "A bug, literally",
        "<note-2@example.net>",
    );
    deliver(
        tree.path(),
        "1770000003.M3.host",
        "F",
        "alan@example.org",
        "Alan Turing",
        "On computable numbers",
        "<note-3@example.org>",
    );
    tree
}

fn make_maildir(directory: &Path) {
    for subdir in ["cur", "new", "tmp"] {
        std::fs::create_dir_all(directory.join(subdir)).expect("a maildir subdirectory");
    }
}

fn deliver(
    folder: &Path,
    id: &str,
    info: &str,
    address: &str,
    name: &str,
    subject: &str,
    message_id: &str,
) -> PathBuf {
    let file = folder.join("cur").join(if info.is_empty() {
        id.to_owned()
    } else {
        format!("{id}:2,{info}")
    });
    std::fs::write(
        &file,
        format!(
            "From: {name} <{address}>\r\n\
             To: user@example.invalid\r\n\
             Subject: {subject}\r\n\
             Message-ID: {message_id}\r\n\
             Date: Tue, 3 Feb 2026 09:00:00 +0000\r\n\
             \r\n\
             {subject}, in the body.\r\n"
        ),
    )
    .expect("a delivered message");
    file
}

async fn opened(root: &Path) -> MaildirBackend {
    let backend = MaildirBackend::open(root).expect("the tree is a maildir");
    backend.connect().await.expect("connect");
    backend
}

/// Every message in `mailbox`, newest number last.
async fn all_of(
    backend: &MaildirBackend,
    mailbox: &str,
) -> Vec<postio_account::backend::FetchedMessage> {
    backend
        .fetch_headers(mailbox, &UidSet::all(), None, &CancelToken::new())
        .await
        .expect("fetch")
}

#[tokio::test]
async fn a_maildir_opens_and_reads_with_no_server_anywhere() {
    let tree = tree();
    let backend = opened(tree.path()).await;

    let capabilities = backend.connect().await.expect("connect");
    assert!(
        !capabilities.supports_incremental_sync(),
        "a maildir has no modification sequence and must not claim one"
    );
    assert!(
        capabilities.contains(Capability::UidPlus),
        "a stored message is at a path this backend chose, so it knows where it went"
    );

    let folders = backend
        .list_mailboxes(&MailboxFilter::all())
        .await
        .expect("list");
    let paths: Vec<&str> = folders.iter().map(|folder| folder.path.as_str()).collect();
    assert_eq!(paths, ["INBOX", "Archive"], "the inbox comes first");

    let status = backend
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .expect("select");
    assert_eq!(status.exists, 3);
    assert_eq!(status.unseen, Some(2), "only one of the three is read");
    assert_eq!(
        status.highest_mod_seq, None,
        "there is no modification sequence on disk to report"
    );

    let messages = all_of(&backend, "INBOX").await;
    let subjects: Vec<&str> = messages
        .iter()
        .filter_map(|message| message.envelope.as_ref()?.subject.as_deref())
        .collect();
    assert_eq!(
        subjects,
        [
            "The analytical engine",
            "A bug, literally",
            "On computable numbers"
        ],
        "the headers are parsed off disk, not guessed"
    );
    assert_eq!(
        messages[0]
            .envelope
            .as_ref()
            .and_then(|envelope| envelope.from.first())
            .map(|from| from.address.as_str()),
        Some("ada@example.com")
    );
    assert!(messages[0].flags.contains(&Flag::Seen));
    assert!(messages[2].flags.contains(&Flag::Flagged));

    let mut sink = VecSink::new();
    backend
        .fetch_body(
            "INBOX",
            &messages[1].remote_id,
            &mut sink,
            &CancelToken::new(),
        )
        .await
        .expect("body");
    assert!(sink.is_finished(), "a completed fetch finishes its sink");
    let body = String::from_utf8(sink.into_inner()).expect("utf-8");
    assert!(
        body.contains("A bug, literally, in the body."),
        "the bytes are the ones on disk: {body}"
    );
}

#[tokio::test]
async fn opening_the_same_tree_again_finds_the_same_mail_not_more_of_it() {
    let tree = tree();

    let first = opened(tree.path()).await;
    let before = all_of(&first, "INBOX").await;
    let before_status = first
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .expect("select");

    // A whole new backend over the same directory — a second run of the app.
    let second = opened(tree.path()).await;
    let after = all_of(&second, "INBOX").await;
    let after_status = second
        .select("INBOX", SelectMode::ReadWrite)
        .await
        .expect("select");

    assert_eq!(after.len(), 3, "a second pass must not find six messages");
    assert_eq!(
        before
            .iter()
            .map(|message| message.remote_id.clone())
            .collect::<Vec<_>>(),
        after
            .iter()
            .map(|message| message.remote_id.clone())
            .collect::<Vec<_>>(),
        "the same mail keeps the same identity, which is what stops the \
         engine importing it again"
    );
    assert_eq!(
        before_status.uid_next, after_status.uid_next,
        "no numbers were handed out on a pass that changed nothing"
    );
    assert_eq!(before_status.generation, after_status.generation);
}

#[tokio::test]
async fn a_directory_that_is_not_a_maildir_says_why_rather_than_looking_empty() {
    let tree = tempfile::tempdir().expect("a temporary directory");
    std::fs::write(tree.path().join("letter.txt"), "not mail").expect("a file");

    let refusal = MaildirBackend::open(tree.path()).expect_err("not a maildir");

    assert!(
        refusal.contains(&tree.path().display().to_string()) && refusal.contains("cur/new/tmp"),
        "the refusal must name the directory and what was missing: {refusal}"
    );
}

#[tokio::test]
async fn a_tree_that_goes_away_underneath_fails_to_connect() {
    let tree = tree();
    let backend = MaildirBackend::open(tree.path()).expect("a maildir");
    drop(tree);

    let error = backend.connect().await.expect_err("the tree is gone");

    assert!(
        matches!(error, BackendError::Io { .. }),
        "an unplugged disk is an I/O failure, not an account with no mail: {error}"
    );
}

#[tokio::test]
async fn nothing_answers_before_a_connect() {
    let tree = tree();
    let backend = MaildirBackend::open(tree.path()).expect("a maildir");

    let error = backend
        .list_mailboxes(&MailboxFilter::all())
        .await
        .expect_err("no session yet");

    assert!(
        matches!(error, BackendError::NotConnected { .. }),
        "the seam's contract holds for every backend: {error}"
    );
}

#[tokio::test]
async fn marking_a_message_read_renames_its_file_and_the_next_listing_sees_it() {
    let tree = tree();
    let backend = opened(tree.path()).await;
    let unread = all_of(&backend, "INBOX").await[1].remote_id.clone();

    let updates = backend
        .store_flags(
            "INBOX",
            std::slice::from_ref(&unread),
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("store");

    assert_eq!(updates.len(), 1);
    assert!(
        updates[0].flags.contains(&Flag::Seen),
        "what is reported is read back off the filename, not what was asked for"
    );

    // The filename is the record, so another client sharing this tree sees it.
    let names = filenames(&tree.path().join("cur"));
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("1770000002") && name.ends_with(":2,S")),
        "the flag lives in the name now: {names:?}"
    );

    let again = opened(tree.path()).await;
    let message = all_of(&again, "INBOX")
        .await
        .into_iter()
        .find(|message| message.remote_id == unread)
        .expect("the same message, still");
    assert!(message.flags.contains(&Flag::Seen));
    assert_eq!(
        message.uid,
        Uid::new(2),
        "reading a message must not renumber it"
    );
}

#[tokio::test]
async fn archiving_moves_the_file_and_says_where_it_landed() {
    let tree = tree();
    let backend = opened(tree.path()).await;
    let message = all_of(&backend, "INBOX").await[0].remote_id.clone();

    let landed = backend
        .move_messages("INBOX", std::slice::from_ref(&message), "Archive")
        .await
        .expect("move");

    assert_eq!(landed.len(), 1, "UIDPLUS is claimed, so this is not empty");
    assert_eq!(landed[0].source, Uid::new(1));
    assert_eq!(filenames(&tree.path().join("cur")).len(), 2);
    assert_eq!(
        filenames(&tree.path().join("Archive").join("cur")).len()
            + filenames(&tree.path().join("Archive").join("new")).len(),
        1,
        "the file is in the archive now, not copied into it"
    );

    let archived = all_of(&backend, "Archive").await;
    assert_eq!(archived.len(), 1);
    assert_eq!(
        archived[0].remote_id, landed[0].destination_remote_id,
        "the reported destination is the identity the next listing hands out"
    );
}

#[tokio::test]
async fn an_appended_message_looks_like_one_that_was_delivered() {
    let tree = tree();
    let backend = opened(tree.path()).await;

    let landed = backend
        .append(
            "Archive",
            &AppendMessage::new(
                b"From: Ada Lovelace <ada@example.com>\r\n\
                  Subject: Filed by hand\r\n\
                  \r\n\
                  body\r\n"
                    .to_vec(),
            ),
        )
        .await
        .expect("append")
        .expect("a maildir always knows where it put a file");

    assert_eq!(
        filenames(&tree.path().join("Archive").join("new")).len(),
        1,
        "unread mail waits in new/, which is where another client looks for it"
    );

    let archived = all_of(&backend, "Archive").await;
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].remote_id, landed.destination_remote_id);
}

#[tokio::test]
async fn an_expunge_removes_what_was_named_and_refuses_to_widen() {
    let tree = tree();
    let backend = opened(tree.path()).await;
    let messages = all_of(&backend, "INBOX").await;
    let doomed = messages[0].remote_id.clone();
    let bystander = messages[1].remote_id.clone();

    // Another client marked one deleted; Postio was handed the other.
    backend
        .store_flags(
            "INBOX",
            &[doomed.clone(), bystander.clone()],
            &FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("store");

    let refusal = backend
        .expunge("INBOX", None)
        .await
        .expect_err("untargeted");
    assert!(
        matches!(refusal, BackendError::Rejected { .. }),
        "an untargeted expunge would take mail another client marked: {refusal}"
    );

    let gone = backend
        .expunge("INBOX", Some(std::slice::from_ref(&doomed)))
        .await
        .expect("expunge");

    assert_eq!(gone, vec![doomed], "it removed them by name, so it knows");
    assert_eq!(filenames(&tree.path().join("cur")).len(), 2);
    let left = all_of(&backend, "INBOX").await;
    assert!(
        left.iter().any(|message| message.remote_id == bystander),
        "the message Postio was not handed is still there"
    );
}

#[tokio::test]
async fn an_identity_from_a_renumbered_generation_is_refused_not_guessed_at() {
    let tree = tree();
    let backend = opened(tree.path()).await;
    let stale = RemoteId::new("1:1");

    let error = backend
        .store_flags(
            "INBOX",
            std::slice::from_ref(&stale),
            &FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect_err("a stale identity");

    assert!(
        matches!(error, BackendError::UidValidityChanged { .. }),
        "acting on whatever now holds that number would flag mail nobody \
         asked about: {error}"
    );
}

#[tokio::test]
async fn a_first_sync_is_told_which_numbers_exist() {
    let tree = tree();
    let backend = opened(tree.path()).await;

    let uids = backend
        .existing_uids("INBOX", &CancelToken::new())
        .await
        .expect("enumerate")
        .expect("a listing is the cheapest thing a maildir does");

    assert_eq!(uids, [Uid::new(1), Uid::new(2), Uid::new(3)]);
}

#[tokio::test]
async fn a_message_can_be_found_by_the_id_its_sender_gave_it() {
    let tree = tree();
    let backend = opened(tree.path()).await;

    let found = backend
        .find_by_message_id("INBOX", "<note-2@example.net>")
        .await
        .expect("search")
        .expect("it is in there");

    let expected = all_of(&backend, "INBOX").await[1].remote_id.clone();
    assert_eq!(found, expected);

    assert_eq!(
        backend
            .find_by_message_id("INBOX", "<never-sent@example.invalid>")
            .await
            .expect("search"),
        None,
        "not finding it is an answer, not a failure"
    );
}

#[tokio::test]
async fn the_header_block_and_the_body_can_be_asked_for_separately() {
    let tree = tree();
    let backend = opened(tree.path()).await;
    let message = all_of(&backend, "INBOX").await[0].remote_id.clone();
    let cancel = CancelToken::new();

    let mut headers = VecSink::new();
    backend
        .fetch_part("INBOX", &message, &BodyPart::Headers, &mut headers, &cancel)
        .await
        .expect("headers");
    let mut text = VecSink::new();
    backend
        .fetch_part("INBOX", &message, &BodyPart::Text, &mut text, &cancel)
        .await
        .expect("text");

    let headers = String::from_utf8(headers.into_inner()).expect("utf-8");
    let text = String::from_utf8(text.into_inner()).expect("utf-8");
    assert!(headers.contains("Subject: The analytical engine"));
    assert!(!headers.contains("in the body"));
    assert!(text.starts_with("The analytical engine, in the body."));
}

fn filenames(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    names.sort();
    names
}
