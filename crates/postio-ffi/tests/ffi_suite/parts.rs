//! A message's parts, and getting an attachment's bytes out of one.
//!
//! The surface #1572 is about. On Linux this is `postio-gtk`'s parts panel
//! and `postio-app`'s byte-fetching half; on macOS there was nothing at all,
//! which meant an attachment that arrived on a Mac could not be saved by any
//! route — no panel, no menu item, and no call under either.
//!
//! These assert against `postio_ui::reader::parts`' own functions wherever a
//! *word* is involved — the sanitised filename, the detail sentence, the
//! failure count's phrasing — for the reason `reader.rs` gives above its own
//! CSP assertions: a copy of the rule written out in a test drifts in exactly
//! the same way a copy in the code does, and then the test agrees with the
//! drift. What is written out longhand here is only the things the boundary
//! itself decides: that listing fetches nothing, that a hostile filename
//! cannot reach the filesystem, and that two parts claiming one name do not
//! overwrite each other.

use chrono::Utc;
use postio_ffi::{Session, SessionOptions};
use postio_model::attachment::Disposition;
use postio_model::{Attachment, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_ui::reader::parts as shared;

/// How a part is described to the seeder: its MIME path, type, size, the name
/// the sender gave it, and the bytes — when the bytes are on this machine.
struct Seed<'a> {
    part_id: &'a str,
    mime: &'a str,
    filename: Option<&'a str>,
    content_id: Option<&'a str>,
    inline: bool,
    bytes: Option<&'a [u8]>,
}

impl<'a> Seed<'a> {
    fn new(part_id: &'a str, mime: &'a str) -> Self {
        Seed {
            part_id,
            mime,
            filename: None,
            content_id: None,
            inline: false,
            bytes: None,
        }
    }

    fn named(mut self, filename: &'a str) -> Self {
        self.filename = Some(filename);
        self
    }

    fn downloaded(mut self, bytes: &'a [u8]) -> Self {
        self.bytes = Some(bytes);
        self
    }

    fn inline_as(mut self, content_id: &'a str) -> Self {
        self.content_id = Some(content_id);
        self.inline = true;
        self
    }
}

/// A session over a store holding one message made of `seeds`.
///
/// `content_type` is the message's own — what the tree hangs off. Every part
/// that names bytes has them written into the blob store first, so
/// "downloaded" in these tests means the same thing it means in the
/// application: the bytes are here, and reading them reaches no network.
async fn with_parts(content_type: &str, seeds: &[Seed<'_>]) -> (std::sync::Arc<Session>, i64) {
    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let id = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.content_type = Some(content_type.to_owned());
        message.attachments = seeds
            .iter()
            .map(|seed| {
                let size = seed.bytes.map(|bytes| bytes.len() as u64).unwrap_or(4_096);
                let mut part =
                    Attachment::new(postio_model::MessageId::UNASSIGNED, seed.mime, size);
                part.part_id = Some(seed.part_id.to_owned());
                part.filename = seed.filename.map(str::to_owned);
                part.content_id = seed.content_id.map(str::to_owned);
                if seed.inline {
                    part.disposition = Disposition::Inline;
                }
                part.blob_id = seed
                    .bytes
                    .map(|bytes| blobs.put(bytes).expect("a part blob"));
                part
            })
            .collect();
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message")
    };

    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session");
    (session, id.into())
}

// -- listing -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_message_crosses_as_the_tree_it_is_made_of() {
    let (session, message) = with_parts(
        "multipart/mixed",
        &[
            Seed::new("1", "text/plain").downloaded(b"hello"),
            Seed::new("2", "application/pdf")
                .named("statement.pdf")
                .downloaded(b"%PDF-1.4"),
        ],
    )
    .await;

    let view = session.message_parts(message).await;

    assert_eq!(view.root, "multipart/mixed");
    assert_eq!(
        view.parts.len(),
        3,
        "the message itself and its two parts: a tree that showed only the \
         parts would have nothing to hang them off"
    );
    assert_eq!(view.parts[0].part_id, "", "the root is the message");
    assert_eq!(view.parts[0].depth, 0);
    assert!(view.parts[0].is_container);

    let pdf = &view.parts[2];
    assert_eq!(pdf.part_id, "2", "the MIME path, which survives a refetch");
    assert_eq!(pdf.mime_type, "application/pdf");
    assert_eq!(pdf.filename.as_deref(), Some("statement.pdf"));
    assert_eq!(pdf.size, 8);
    assert!(pdf.downloaded);
    assert!(!pdf.is_container);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inline_part_says_so_and_an_attachment_does_not() {
    // The one distinction a frontend cannot work out for itself and must not
    // guess at: a `cid:` the body already drew is not something to offer as
    // "the attachment on this message", and a list that called it one would
    // put the sender's signature logo beside their invoice.
    let (session, message) = with_parts(
        "multipart/related",
        &[
            Seed::new("1", "text/html").downloaded(b"<p>hi</p>"),
            Seed::new("2", "image/png")
                .named("logo.png")
                .inline_as("logo@example.com")
                .downloaded(b"\x89PNG"),
            Seed::new("3", "application/pdf")
                .named("invoice.pdf")
                .downloaded(b"%PDF"),
        ],
    )
    .await;

    let view = session.message_parts(message).await;
    let logo = &view.parts[2];
    let invoice = &view.parts[3];

    assert!(logo.inline, "a `cid:` the body references is inline");
    assert_eq!(logo.content_id.as_deref(), Some("logo@example.com"));
    assert!(!invoice.inline, "an attachment is not");
    assert_eq!(invoice.content_id, None);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_the_parts_fetches_nothing() {
    // `BODYSTRUCTURE` describes a message without transferring a byte of it,
    // and the panel is a reading of that description. So a part nobody has
    // downloaded is listed, sized and named, and says plainly that its bytes
    // are not here — rather than being absent, or being fetched to find out.
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("1", "application/zip").named("archive.zip")],
    )
    .await;

    // What actually makes this true is structural and no assertion can see
    // it: `postio_session::reading::message_parts` is handed a `&Store` and
    // nothing else -- no blob store, no engine -- so there is no expression
    // it could write that reaches the network. What is asserted below is the
    // consequence, and this is the guard that keeps even that honest: a
    // session with no engine cannot fetch, so if one is ever wired into this
    // fixture the test has to be rewritten rather than quietly keep passing
    // for the wrong reason.
    assert!(
        !session.has_engine(),
        "this fixture proves nothing about fetching once an engine is in it"
    );

    let view = session.message_parts(message).await;
    let archive = &view.parts[1];

    assert!(!archive.downloaded, "described by the server, not fetched");
    assert_eq!(archive.size, 4_096, "the size the server declared");
    assert_eq!(
        postio_ffi::part_note(archive.clone(), 0, 0),
        shared::NOT_FETCHED,
        "the sentence that explains an empty preview, shared with GTK so the \
         two readers do not say different things about the same state"
    );
    session.shutdown();
}

// -- the name a part is saved under ------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_filename_off_the_wire_cannot_steer_where_a_save_lands() {
    // The filename is attacker-controlled. `postio_model::mime::parse`
    // reports what the sender wrote, faithfully and on purpose; this is the
    // layer that makes it fit to name a file, and it has to be this layer
    // rather than Swift's — a second sanitiser is a second chance to miss a
    // separator.
    let (session, message) = with_parts(
        "multipart/mixed",
        &[
            Seed::new("1", "text/plain")
                .named("../../.bashrc")
                .downloaded(b"owned"),
            Seed::new("2", "text/plain")
                .named("a\0b.txt")
                .downloaded(b"nul"),
        ],
    )
    .await;

    let view = session.message_parts(message).await;
    for part in &view.parts[1..] {
        let name = &part.save_name;
        assert!(!name.contains('/'), "{name:?} carries a path separator");
        assert!(!name.contains('\\'), "{name:?} carries a path separator");
        assert!(!name.starts_with('.'), "{name:?} is a dotfile");
        assert!(
            !name.chars().any(char::is_control),
            "{name:?} carries a control character"
        );
        assert!(!name.is_empty(), "a part must always have a name to offer");
    }
    session.shutdown();
}

// -- bytes -------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_downloaded_parts_bytes_cross_exactly() {
    let (session, message) = with_parts(
        "multipart/mixed",
        &[
            Seed::new("1", "text/plain").downloaded(b"words"),
            Seed::new("2", "image/png")
                .named("cold.png")
                .downloaded(b"\x89PNG\r\n\x1a\n"),
        ],
    )
    .await;

    let bytes = session
        .part_bytes(message, "2".to_string())
        .await
        .expect("the part's own bytes");
    assert_eq!(bytes, b"\x89PNG\r\n\x1a\n");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_part_that_is_not_on_this_machine_and_cannot_be_fetched_says_so() {
    // No engine — nothing is syncing this store — so there is no route to the
    // bytes at all. A sentence rather than an empty file: a zero-byte
    // attachment on disk looks like a saved file and is not one.
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("1", "application/zip").named("archive.zip")],
    )
    .await;

    let refused = session
        .part_bytes(message, "1".to_string())
        .await
        .expect_err("bytes that are not here cannot be handed over")
        .to_string();
    assert!(
        refused.contains("syncing"),
        "the refusal has to name the reason, and this reason in particular: a \
         store that is not open and an account that is not syncing send the \
         user to two different places, and the one sentence they both get is \
         the one that sends them to the wrong one. Got: {refused:?}"
    );
    session.shutdown();
}

// -- saving ------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn saving_a_part_writes_its_bytes_where_the_user_chose() {
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("1", "application/pdf")
            .named("statement.pdf")
            .downloaded(b"%PDF-1.7 statement")],
    )
    .await;

    let chosen = tempfile::tempdir().expect("somewhere to save");
    let path = chosen.path().join("wherever-they-said.pdf");
    session
        .save_part(message, "1".to_string(), path.display().to_string())
        .await
        .expect("the part is saved");

    assert_eq!(
        std::fs::read(&path).expect("the saved file"),
        b"%PDF-1.7 statement",
        "the file the user named holds the part's bytes and nothing else"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn exporting_a_part_names_it_rather_than_letting_the_sender_name_it() {
    // What "Open with…" and a drag-out are built on: Postio chooses the
    // filename, the caller chooses only the directory. That is what stops a
    // frontend passing the raw `filename=` through on the one path where the
    // file is then handed to another application.
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("1", "image/png")
            .named("../../../evil.png")
            .downloaded(b"\x89PNG")],
    )
    .await;

    let into = tempfile::tempdir().expect("a directory to export into");
    let written = session
        .export_part(message, "1".to_string(), into.path().display().to_string())
        .await
        .expect("the part is exported");
    let written = std::path::PathBuf::from(written);

    assert_eq!(
        written.parent(),
        Some(into.path()),
        "the export escaped the directory it was given: {written:?}"
    );
    assert_eq!(
        std::fs::read(&written).expect("the exported file"),
        b"\x89PNG"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_every_part_gives_each_one_a_name_of_its_own() {
    // Two senders' worth of the same habit: a message with two parts both
    // called `invoice.pdf`. Named by the sender alone, the second would land
    // on top of the first and "save all" would silently produce one file.
    let (session, message) = with_parts(
        "multipart/mixed",
        &[
            Seed::new("1", "text/plain").downloaded(b"see attached"),
            Seed::new("2", "application/pdf")
                .named("invoice.pdf")
                .downloaded(b"first"),
            Seed::new("3", "application/pdf")
                .named("invoice.pdf")
                .downloaded(b"second"),
        ],
    )
    .await;

    let into = tempfile::tempdir().expect("a directory to save into");
    let outcome = session
        .save_all_parts(message, into.path().display().to_string())
        .await
        .expect("every part is saved");

    assert_eq!(outcome.saved, 3, "the text part counts too — it has bytes");
    assert_eq!(outcome.failed, 0);
    assert_eq!(outcome.failure, None, "nothing failed, so nothing to say");

    let mut written: Vec<String> = std::fs::read_dir(into.path())
        .expect("the directory")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    written.sort();
    assert_eq!(
        written.len(),
        3,
        "two parts claiming one name overwrote each other: {written:?}"
    );

    let bodies: std::collections::BTreeSet<Vec<u8>> = written
        .iter()
        .map(|name| std::fs::read(into.path().join(name)).expect("a saved part"))
        .collect();
    assert!(
        bodies.contains(b"first".as_slice()) && bodies.contains(b"second".as_slice()),
        "both invoices survived the save: {bodies:?}"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_that_could_not_get_every_part_says_how_many() {
    // One part is here and one never arrived, with no engine to fetch it.
    // The count crosses with the sentence already written for it, because a
    // frontend that phrased this itself would phrase it differently from the
    // other frontend the first time the wording changed.
    let (session, message) = with_parts(
        "multipart/mixed",
        &[
            Seed::new("1", "text/plain").downloaded(b"here"),
            Seed::new("2", "application/zip").named("archive.zip"),
        ],
    )
    .await;

    let into = tempfile::tempdir().expect("a directory to save into");
    let outcome = session
        .save_all_parts(message, into.path().display().to_string())
        .await
        .expect("the batch runs to the end");

    assert_eq!(outcome.saved, 1);
    assert_eq!(outcome.failed, 1, "the batch did not abandon the rest");
    assert_eq!(
        outcome.failure.as_deref(),
        shared::save_all_failure(1).as_deref(),
        "one sentence for the batch, shared with the other frontend"
    );
    session.shutdown();
}

// -- walking the tree --------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_cursor_starts_on_the_first_part_and_stops_at_the_ends() {
    // The rule, not the state: which row a panel opens on and what `j` at the
    // bottom does are decisions both frontends have to make identically, and
    // the state itself stays with whichever panel is drawing.
    let (session, message) = with_parts(
        "multipart/mixed",
        &[
            Seed::new("1", "text/plain").downloaded(b"a"),
            Seed::new("2", "text/html").downloaded(b"b"),
        ],
    )
    .await;

    let view = session.message_parts(message).await;
    assert_eq!(
        view.cursor, 1,
        "the first part, not the message: opening on the container costs a \
         keystroke every single time"
    );

    let count = view.parts.len() as u32;
    assert_eq!(postio_ffi::part_cursor_after(1, true, count), 2);
    assert_eq!(
        postio_ffi::part_cursor_after(2, true, count),
        2,
        "the bottom holds rather than wrapping round to the top"
    );
    assert_eq!(postio_ffi::part_cursor_after(1, false, count), 0);
    assert_eq!(
        postio_ffi::part_cursor_after(0, false, count),
        0,
        "and the top holds"
    );
    session.shutdown();
}

// -- rendering a held-back part once -----------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn only_the_markup_part_is_offered_a_single_render() {
    // `render_part_once` is about the one part that can load things. An
    // `image/png` attachment references nothing and cannot phone home, so
    // offering to render *it* once would be theatre.
    assert_eq!(
        postio_ffi::part_held_back_note("image/png".to_string(), 6, 1),
        None,
        "an image part is not what is being held back"
    );
    let offered = postio_ffi::part_held_back_note("text/html".to_string(), 6, 1)
        .expect("a held-back HTML part is offered a single render");
    assert_eq!(
        offered,
        shared::held_back_note("text/html", 6, 1).expect("the shared sentence"),
        "the sentence is GTK's, to the word"
    );
    // Written out rather than left to the comparison above, which is happy
    // when both sides are `None` and happy when both count the same thing
    // twice. What the crossing has to get right is that these two numbers
    // arrive as themselves: six images and one tracker, not one image and six
    // trackers, and not six of one counted as both.
    assert!(
        offered.contains("6 remote images") && offered.contains("1 likely tracker"),
        "the counts did not cross as themselves: {offered:?}"
    );
    assert_eq!(
        postio_ffi::part_held_back_note("text/html".to_string(), 0, 0),
        None,
        "nothing held back, nothing to offer"
    );
}
