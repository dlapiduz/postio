use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use postio_widgets::list_model::WindowedModel;

use crate::support::{self, Fixture};

fn raw(subject: &str) -> Vec<u8> {
    format!(
        "From: Ada Moreno <ada@example.com>\r\nTo: Grace Hopper <grace@example.net>\r\n\
         Subject: {subject}\r\n\r\nHalf past twelve?\r\n"
    )
    .into_bytes()
}

fn export_dir() -> std::path::PathBuf {
    postio_session::paths::export_dir()
}

async fn five_with_sources() -> (
    Fixture,
    postio_gtk::window::FocusWindow,
    Vec<postio_model::MessageId>,
) {
    let fixture = Fixture::empty().await;
    let messages = fixture.file_five().await;
    for (message, subject) in messages
        .iter()
        .zip(["First", "Second", "Third", "Fourth", "Fifth"])
    {
        fixture.write_raw(*message, &raw(subject)).await;
    }
    let (window, _client) = fixture.open_five().await;
    (fixture, window, messages)
}

fn position(window: &postio_gtk::window::FocusWindow, message: postio_model::MessageId) -> u32 {
    window
        .pane()
        .expect("the inbox")
        .feed()
        .list()
        .position_of(message)
        .expect("the message is listed")
}

fn dropped(offer: &gdk::ContentProvider) -> Vec<std::path::PathBuf> {
    let stream = gio::MemoryOutputStream::new_resizable();
    glib::MainContext::default()
        .block_on(offer.write_mime_type_future("text/uri-list", &stream, glib::Priority::DEFAULT))
        .expect("the drop is served");
    stream.close(gio::Cancellable::NONE).expect("it closes");
    let uris = stream.steal_as_bytes();
    String::from_utf8_lossy(&uris)
        .lines()
        .filter(|line| line.starts_with("file://"))
        .map(|uri| gio::File::for_uri(uri).path().expect("a local path"))
        .collect()
}

fn names(paths: &[std::path::PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

pub fn a_dragged_row_hands_a_file_manager_its_message_as_an_eml() {
    crate::gtk_case(async {
        let (_fixture, window, messages) = five_with_sources().await;

        let offer = window
            .drag_offer(position(&window, messages[1]))
            .expect("a row in the list can be dragged out");
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

        let files = dropped(&offer);
        assert_eq!(names(&files), ["Second.eml"]);
        assert_eq!(
            std::fs::read(&files[0]).expect("the exported file"),
            raw("Second"),
            "the file another application opens is not the message the server sent"
        );
    });
}

pub fn a_dragged_selection_is_every_selected_message_and_nothing_is_written_early() {
    crate::gtk_case(async {
        let (_fixture, window, messages) = five_with_sources().await;
        support::keys(&window, &["x", "j", "j", "x"]);

        let offer = window
            .drag_offer(position(&window, messages[2]))
            .expect("a selected row can be dragged out");
        assert!(
            !export_dir().exists() || std::fs::read_dir(export_dir()).unwrap().next().is_none(),
            "picking up a drag wrote files before any drop asked for them"
        );
        let files = dropped(&offer);
        let mut named = names(&files);
        named.sort();
        assert_eq!(named, ["First.eml", "Third.eml"]);

        let alone = window
            .drag_offer(position(&window, messages[4]))
            .expect("an unselected row can be dragged out");
        assert_eq!(names(&dropped(&alone)), ["Fifth.eml"]);
        assert_eq!(
            window.selection().ids().map(|ids| ids.len()),
            Some(2),
            "dragging a row outside the selection does not change it"
        );
    });
}

pub fn a_select_all_drag_offers_no_files() {
    crate::gtk_case(async {
        let (_fixture, window, messages) = five_with_sources().await;
        support::keys(&window, &["X"]);
        assert!(window.selection().is_everything());
        // A predicate has no file form: resolving "everything" would write an `.eml` for every
        // message in the folder.
        assert!(
            window.drag_offer(position(&window, messages[0])).is_none(),
            "a select-all drag offered files for a whole mailbox"
        );
    });
}

const PORTAL_MIME: &str = "application/vnd.portal.filetransfer";

/// Inside a sandbox the portal spelling is the only one that carries a file out; GTK registers it
/// only where the file-transfer portal answers, so a CI runner has nothing to offer it with.
pub fn a_dragged_row_offers_the_portal_spelling_too() {
    crate::gtk_case(async {
        if !portal_available() {
            eprintln!("skipping: no working org.freedesktop.portal.FileTransfer on this bus");
            return;
        }
        let (_fixture, window, messages) = five_with_sources().await;
        let offer = window
            .drag_offer(position(&window, messages[1]))
            .expect("a row in the list can be dragged out");
        let mimes: Vec<String> = offer
            .formats()
            .union_serialize_mime_types()
            .mime_types()
            .iter()
            .map(|mime| mime.to_string())
            .collect();
        assert!(
            mimes.iter().any(|mime| mime == PORTAL_MIME),
            "a sandboxed receiver could take nothing from this drag: {mimes:?}"
        );
    });
}

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
        5_000,
        gio::Cancellable::NONE,
    )?;
    let (files,): (Vec<String>,) = reply.get().expect("the portal answered with (as)");
    Ok(files)
}

/// Owning the name is not enough: a sandbox with no FUSE mount owns it and fails every call.
fn portal_available() -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
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

fn portal_key(offer: &gdk::ContentProvider) -> String {
    let stream = gio::MemoryOutputStream::new_resizable();
    glib::MainContext::default()
        .block_on(offer.write_mime_type_future(PORTAL_MIME, &stream, glib::Priority::DEFAULT))
        .expect("the portal spelling of the drop is served");
    stream.close(gio::Cancellable::NONE).expect("it closes");
    String::from_utf8_lossy(&stream.steal_as_bytes())
        .trim_end_matches('\0')
        .trim()
        .to_owned()
}

/// The portal carries references, not bytes, so reclaiming the export directory afterwards breaks
/// the receiver: the export directory is a cache the app never sweeps.
/// Needs a working document portal, so it is ignored by name (see `IGNORED`).
pub fn a_dragged_message_survives_the_portal() {
    crate::gtk_case(async {
        if !portal_available() {
            eprintln!("skipping: no working org.freedesktop.portal.FileTransfer on this bus");
            return;
        }
        let (_fixture, window, messages) = five_with_sources().await;
        let offer = window
            .drag_offer(position(&window, messages[1]))
            .expect("a row in the list can be dragged out");

        let key = portal_key(&offer);
        assert!(
            !key.is_empty(),
            "the portal spelling serialised to nothing: a sandboxed receiver would get no files"
        );
        let files = retrieve_files(&key).expect("the portal resolves the transfer key");
        assert_eq!(files.len(), 1, "one message was dragged: {files:?}");
        let path = std::path::PathBuf::from(&files[0]);
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "Second.eml");
        assert_eq!(
            std::fs::read(&path).expect("the portal named a file the receiver cannot open"),
            raw("Second"),
            "the file another application opens is not the message the server sent"
        );

        let key = portal_key(&offer);
        for entry in std::fs::read_dir(export_dir()).expect("the export directory") {
            let _ = std::fs::remove_file(entry.expect("an entry").path());
        }
        let files = retrieve_files(&key).expect("the portal still resolves the key");
        assert!(
            files
                .iter()
                .all(|path| !std::path::Path::new(path).exists()),
            "the portal copied the exported bytes; paths::export_dir's reasoning needs revisiting"
        );
    });
}
