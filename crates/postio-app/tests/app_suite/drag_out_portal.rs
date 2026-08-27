//! Does the portal spelling of a drag actually hand over files?
//!
//! `drag_out_wiring.rs` proves the running application offers
//! `application/vnd.portal.filetransfer` and that `text/uri-list` serves a
//! real message. Neither proves the portal *spelling* produces anything a
//! receiver could use: a provider can advertise a format, serialise it
//! without error, and still hand over a transfer key that resolves to
//! nothing. Inside a Flatpak that is the only spelling that carries files
//! out, so "it is offered" is the weakest possible claim about it.
//!
//! So this drives the whole round trip the way a receiving application does:
//! serialise to the portal mime type, take the transfer key off the stream,
//! and call `org.freedesktop.portal.FileTransfer.RetrieveFiles` with it —
//! then read the bytes back off whatever paths come out.
//!
//! # What this does and does not prove about the sandbox
//!
//! Run on the host it proves the mechanism: the serialiser really does talk
//! to the portal, the portal really does accept the files, and a receiver
//! really can read them. Run *inside* `flatpak run dev.postio.Postio` — the
//! same test, no changes — it proves the sandboxed path, which is what
//! issue #121 is about. It is one test either way; where you run it is the
//! variable.
//!
//! # The portal carries references, not bytes
//!
//! Worth knowing before reading the assertions: `AddFiles` takes file
//! descriptors, but `RetrieveFiles` hands back *paths*, and the portal never
//! copies the content. Delete an exported file between the drop and the
//! receiver's read and the receiver gets nothing, silently — which is
//! exactly why `paths::export_dir` pointing at a cache directory is worth a
//! test rather than a comment. `the_portal_does_not_copy_the_bytes` below
//! pins that down, so anyone who later "tidies up" the export directory
//! finds out here rather than from a bug report.
//!
//! One test function per binary: GTK initialises once per process.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread
// reading the environment. This sets it before the app under test starts,
// which is the one moment it is sound.

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

const PORTAL_MIME: &str = "application/vnd.portal.filetransfer";

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    glib::MainContext::default().block_on(future)
}

/// Ask the document portal what a receiver would get for this transfer key.
///
/// This is the receiving half of a drop, done by hand. `RetrieveFiles` is on
/// `org.freedesktop.portal.Documents` rather than `…portal.Desktop`, which is
/// the detail that makes this look wrong at first glance.
fn retrieve_files(key: &str) -> Result<Vec<String>, glib::Error> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
    let reply = bus.call_sync(
        Some("org.freedesktop.portal.Documents"),
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.FileTransfer",
        "RetrieveFiles",
        Some(
            &(
                key,
                std::collections::HashMap::<String, glib::Variant>::new(),
            )
                .to_variant(),
        ),
        Some(glib::VariantTy::new("(as)").unwrap()),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
    )?;
    let (files,): (Vec<String>,) = reply.get().expect("the portal answered with (as)");
    Ok(files)
}

/// Is there a document portal on this bus, and does it actually answer for
/// `FileTransfer`?
///
/// CI has no session bus and no portal, and a test that cannot run must say
/// so rather than fail. This is the one thing that legitimately skips.
///
/// Owning the `Documents` name is not enough on its own: the round trip
/// below goes through `org.freedesktop.portal.FileTransfer`, served by the
/// same document-portal daemon at the same object path but a separate
/// interface, and a backend that owns the name — or a sandbox without a
/// working FUSE mount underneath it — can still fail every call on that
/// interface. Left unchecked that surfaced as `RetrieveFiles`'s `-1`
/// (default, tens-of-seconds) timeout expiring deep inside the test instead
/// of a clean skip at the top. A bounded property read tells "not
/// implemented, or too slow to answer" apart from "genuinely there" without
/// borrowing the real call's much longer patience.
fn portal_available() -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    let has_owner = |name: &str| -> bool {
        bus.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&(name,).to_variant()),
            Some(glib::VariantTy::new("(b)").unwrap()),
            gio::DBusCallFlags::NONE,
            2_000,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|reply| reply.get::<(bool,)>())
        .map(|(owned,)| owned)
        .unwrap_or(false)
    };
    if !has_owner("org.freedesktop.portal.Documents") {
        return false;
    }

    bus.call_sync(
        Some("org.freedesktop.portal.Documents"),
        "/org/freedesktop/portal/documents",
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&("org.freedesktop.portal.FileTransfer", "version").to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2_000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

pub fn a_dragged_message_survives_the_portal() {
    let state_dir = std::env::temp_dir().join(format!("postio-dragportal-{}", std::process::id()));
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
    if !portal_available() {
        eprintln!(
            "skipping: no working org.freedesktop.portal.FileTransfer on this \
             session bus (either the Documents portal is entirely absent, or \
             it owns the name but does not answer for FileTransfer — a \
             sandbox with no working FUSE mount underneath it looks like \
             this). This test is the sandboxed drag path; it needs a real \
             desktop portal."
        );
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

    let _wired = feed_the_window(&window, &wiring).expect("the store has an account");
    while glib::MainContext::default().iteration(false) {}

    let list = window.list();
    list.selection().select_only(message_id);
    let offer = list.drag_offer();

    // ── an abandoned drag writes nothing ────────────────────────────────
    // The provider is lazy on purpose: a selection here can be a predicate
    // over a whole mailbox, and picking five hundred messages up and putting
    // them down again must not have written five hundred files. Asserting it
    // *here*, against the offer the running application actually hands GTK,
    // is the difference between testing the promise and testing a comment.
    assert!(
        !export_dir.exists() || std::fs::read_dir(&export_dir).unwrap().next().is_none(),
        "picking up a drag wrote files before any drop asked for them: {export_dir:?}"
    );

    // ── the drop lands, in the spelling that leaves a sandbox ────────────
    let stream = gio::MemoryOutputStream::new_resizable();
    block_on(offer.write_mime_type_future(PORTAL_MIME, &stream, glib::Priority::DEFAULT))
        .expect("the portal spelling of the drop is served");
    stream.close(gio::Cancellable::NONE).expect("it closes");

    let payload = stream.steal_as_bytes();
    let key = String::from_utf8_lossy(&payload)
        .trim_end_matches('\0')
        .trim()
        .to_string();
    assert!(
        !key.is_empty(),
        "the portal spelling serialised to nothing. A receiver inside a \
         sandbox would take this drop and get no files at all."
    );

    // ── and a receiver can read the message back off it ──────────────────
    let files = retrieve_files(&key).expect("the portal resolves the transfer key");
    assert_eq!(
        files.len(),
        1,
        "one message was dragged; the portal offered {} file(s): {files:?}",
        files.len()
    );
    let path = std::path::PathBuf::from(&files[0]);
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Lunch on Thursday.eml",
        "a receiver would save this under the wrong name"
    );
    assert_eq!(
        std::fs::read(&path).expect(
            "the portal named a file the receiver cannot open. This is the \
             silent failure #121 is about: Postio believes the drop succeeded."
        ),
        RAW,
        "the file another application opens is not the message the server sent"
    );

    the_portal_does_not_copy_the_bytes(&offer, &export_dir);

    bridge.shutdown();
    let _ = std::fs::remove_dir_all(&state_dir);
}

/// Removing an exported file breaks the receiver, transfer key or not.
///
/// `AddFiles` takes descriptors, which makes it tempting to assume the portal
/// has taken a copy and the export directory is free to be reclaimed. It has
/// not: `RetrieveFiles` hands back paths, and a path to a deleted file is
/// nothing. `paths::export_dir` points at `$XDG_CACHE_HOME`, a directory the
/// system is explicitly allowed to empty, so this is the assertion that turns
/// "the cache is fine, losing these costs nothing" from a claim into a
/// tested one — and that fails loudly if anyone adds a cleanup pass over the
/// export directory.
fn the_portal_does_not_copy_the_bytes(offer: &gdk::ContentProvider, export_dir: &std::path::Path) {
    let stream = gio::MemoryOutputStream::new_resizable();
    block_on(offer.write_mime_type_future(PORTAL_MIME, &stream, glib::Priority::DEFAULT))
        .expect("a second drop is served");
    stream.close(gio::Cancellable::NONE).expect("it closes");
    let payload = stream.steal_as_bytes();
    let key = String::from_utf8_lossy(&payload)
        .trim_end_matches('\0')
        .trim()
        .to_string();

    // Reclaim the cache, as the system is entitled to.
    for entry in std::fs::read_dir(export_dir).expect("the export directory") {
        let entry = entry.expect("an entry");
        let _ = std::fs::remove_file(entry.path());
    }

    let files = retrieve_files(&key).expect("the portal still resolves the key");
    let missing = files
        .iter()
        .filter(|path| !std::path::Path::new(path).exists())
        .count();
    assert_eq!(
        missing,
        files.len(),
        "the portal appears to have copied the exported bytes somewhere of \
         its own. That would be good news, and it would mean the reasoning \
         in paths::export_dir's doc comment is describing the wrong \
         mechanism — check what changed before relaxing anything."
    );
}
