//! Can a person actually drag mail out of the running application?
//!
//! `postio-gtk`'s own test proves the content provider is lazy and offers the
//! right formats. `postio-app`'s unit tests prove an `.eml` comes out of the
//! blob store byte-identical. Both pass in a build where nothing ever calls
//! `connect_export`, and in that build dragging a message onto the desktop
//! does nothing at all.
//!
//! That gap is the shape of bug `postio-bl2` was filed about — a capability
//! fully implemented, fully tested, and never wired — so this test starts
//! where the application starts: a real store with real mail, a real `Window`,
//! and `feed_the_window`, the same call `run` makes. Then it asks the list for
//! the very offer its drag source hands to GTK, and reads the file back off
//! the disk.
//!
//! One test function: GTK initialises once per process, from one thread.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use postio_app::feed_the_window;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, test_support};

/// Raw mail, spelled the way every fixture in this repository spells it.
const RAW: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Lunch on Thursday\r\n\
\r\n\
Half past twelve?\r\n";

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    glib::MainContext::default().block_on(future)
}

#[test]
fn a_message_in_the_list_can_be_dragged_out_as_a_file() {
    let state_dir = std::env::temp_dir().join(format!("postio-dragout-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    let export_dir = state_dir.join("drag");
    // SAFETY: first statements of a single-threaded test.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", &state_dir);
        std::env::set_var("POSTIO_EXPORT_DIR", &export_dir);
    }

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── a store with one account, one folder and one real message ───────
    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    let message_id = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        // The list only shows a mailbox it can find; `seed_small` would do,
        // but one known message makes the assertion about *these* bytes.
        let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
        message.subject = Some("Lunch on Thursday".into());
        message.raw_blob_id = Some(blobs.put(RAW).expect("a blob"));
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("a message")
    };

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // ── the same call `run` makes ───────────────────────────────────────
    let _wired = feed_the_window(&window, &wiring).expect("the store has an account");
    while glib::MainContext::default().iteration(false) {}

    // ── select the message, exactly as clicking it would ────────────────
    let list = window.list();
    list.selection().select_only(message_id);

    // ── the offer the drag source hands GTK ─────────────────────────────
    let offer = list.drag_offer();
    let mimes: Vec<String> = offer
        .formats()
        .union_serialize_mime_types()
        .mime_types()
        .iter()
        .map(|mime| mime.to_string())
        .collect();
    assert!(
        mimes.iter().any(|mime| mime == "text/uri-list"),
        "the running application offers a drag that no file manager can take: {mimes:?}. \
         Every layer under this one is tested and passes; that is exactly the shape of \
         bug postio-bl2 is about — check whether anything calls connect_export."
    );

    // ── and Postio's own sidebar is still served ────────────────────────
    // The offer became a union when files were added to it. If the string
    // half were lost, dragging a message onto a folder — the thing that
    // already worked — would quietly stop working, and no test of the new
    // half would notice.
    let payload = offer
        .value(glib::types::Type::STRING)
        .expect("the string half is still offered");
    let payload: String = payload.get().expect("a string");
    assert_eq!(
        postio_gtk::list_view::dragged_messages(&payload),
        Some(postio_gtk::list_view::Dragged::Selection),
        "the sidebar drop reads this, and a selection must stay a reference to itself"
    );

    // ── the drop lands ──────────────────────────────────────────────────
    let stream = gio::MemoryOutputStream::new_resizable();
    block_on(offer.write_mime_type_future("text/uri-list", &stream, glib::Priority::DEFAULT))
        .expect("the drop is served");
    stream.close(gio::Cancellable::NONE).expect("it closes");

    let uris = stream.steal_as_bytes();
    let uris = String::from_utf8_lossy(&uris);
    let uri = uris
        .lines()
        .find(|line| line.starts_with("file://"))
        .unwrap_or_else(|| panic!("no file uri was handed over: {uris:?}"));

    // ── and the file is the message ─────────────────────────────────────
    let path = gio::File::for_uri(uri).path().expect("a local path");
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Lunch on Thursday.eml",
        "a receiver would save this under the wrong name"
    );
    assert_eq!(
        std::fs::read(&path).expect("the exported file"),
        RAW,
        "the file another application opens is not the message the server sent"
    );

    // ── and the reader's parts panel offers its attachments too ─────────
    a_part_in_the_reader_can_be_dragged_out(&window);

    bridge.shutdown();
    let _ = std::fs::remove_dir_all(&state_dir);
}

/// The parts panel is a second surface with the same failure mode: fully
/// built, fully unit-tested, and offering nothing because nothing called
/// `connect_export`.
///
/// This asserts the seam is wired and that a container is refused. What it
/// does not do is drive a real fetch — `export::tests` covers the bytes, and
/// getting a message *open* in the reader from here needs the activation
/// round trip that `reading.rs` owns.
fn a_part_in_the_reader_can_be_dragged_out(window: &Window) {
    use postio_gtk::parts::Node;

    let attachment = Node {
        part_id: "2".into(),
        depth: 1,
        mime: "text/csv".into(),
        filename: Some("figures.csv".into()),
        size: 7,
        downloaded: true,
        last: true,
        attachment: Some(postio_model::ids::AttachmentId::new(1)),
    };
    let container = Node {
        part_id: String::new(),
        depth: 0,
        mime: "multipart/mixed".into(),
        filename: None,
        size: 0,
        downloaded: true,
        last: true,
        attachment: None,
    };

    let offer = window.parts().drag_offer(&attachment).expect(
        "the reader offers no way to drag an attachment out. Every layer under this one \
         is tested and passes — check whether anything calls Parts::connect_export.",
    );
    let mimes: Vec<String> = offer
        .formats()
        .union_serialize_mime_types()
        .mime_types()
        .iter()
        .map(|mime| mime.to_string())
        .collect();
    assert!(
        mimes.iter().any(|mime| mime == "text/uri-list"),
        "no file manager could take this drag: {mimes:?}"
    );

    assert!(
        window.parts().drag_offer(&container).is_none(),
        "a container is a wrapper, not a file: dragging one would write an empty \
         file named after something that was never a file"
    );
}
