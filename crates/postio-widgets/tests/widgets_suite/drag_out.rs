use std::cell::Cell;
use std::rc::Rc;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use postio_widgets::drag_out::{LazyFiles, Materialise};

fn provider(calls: &Rc<Cell<usize>>, files: Vec<gio::File>) -> LazyFiles {
    let calls = Rc::clone(calls);
    let materialise: Materialise = Rc::new(move |_messages| {
        calls.set(calls.get() + 1);
        let files = files.clone();
        Box::pin(async move { Ok(files) })
    });
    LazyFiles::for_messages(vec![postio_model::MessageId::new(1)], materialise)
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    glib::MainContext::default().block_on(future)
}

pub fn nothing_is_written_until_the_drop_asks() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("Lunch on Thursday.eml");
    std::fs::write(
        &path,
        b"From: Ada Lovelace <ada@example.com>\r\n\r\nHalf past twelve?\r\n",
    )
    .expect("a file");

    let calls = Rc::new(Cell::new(0));
    let provider = provider(&calls, vec![gio::File::for_path(&path)]);

    assert_eq!(
        calls.get(),
        0,
        "picking messages up must not write anything: an abandoned drag of a \
         large selection would have written a file per message for nothing"
    );

    let formats = provider.formats();
    assert!(
        formats.types().contains(&gdk::FileList::static_type()),
        "a receiver that cannot see a file list will never ask for one: {:?}",
        formats.types()
    );
    assert_eq!(calls.get(), 0, "reading the formats is not a drop");

    let stream = gio::MemoryOutputStream::new_resizable();
    block_on(provider.write_mime_type_future("text/uri-list", &stream, glib::Priority::DEFAULT))
        .expect("the drop is served");

    assert_eq!(calls.get(), 1, "the drop is what materialises the files");

    stream.close(gio::Cancellable::NONE).expect("it closes");
    let written = stream.steal_as_bytes();
    let written = String::from_utf8_lossy(&written);
    assert!(
        written.contains("Lunch%20on%20Thursday.eml"),
        "the receiver was handed no usable uri: {written:?}"
    );
}

/// Under Flatpak this is the spelling that carries files out of the sandbox.
pub fn the_sandboxed_spelling_is_offered_too() {
    let calls = Rc::new(Cell::new(0));
    let provider = provider(&calls, Vec::new());
    // GDK folds the registered serialisers in before a drag; `formats()` alone is a type-only list,
    // so asserting on it would pass for the wrong reason.
    let offered: Vec<String> = provider
        .formats()
        .union_serialize_mime_types()
        .mime_types()
        .iter()
        .map(|mime| mime.to_string())
        .collect();

    assert!(
        offered.iter().any(|mime| mime == "text/uri-list"),
        "offered: {offered:?}"
    );
    // The provider advertises the type, not hand-named mime types, which would drop this one and
    // break drag-out in the sandbox only.
    // Asserted only where GDK registers the spelling: when the FileTransfer portal is reachable on
    // a session bus (#121 proves it under Flatpak).
    // The skip is loud on purpose: quiet skips are how GTK tests passed without a display (#781).
    if offered
        .iter()
        .any(|mime| mime == "application/vnd.portal.filetransfer")
    {
        return;
    }
    assert!(
        !portal_is_reachable(),
        "the sandboxed path is not on offer even though the FileTransfer \
         portal is reachable, so this is Postio's provider and not the \
         environment: {offered:?}"
    );
    eprintln!(
        "skipping the sandbox spelling: no FileTransfer portal on this \
         session bus, so GDK cannot advertise it here. #121 proves this path \
         under Flatpak; it is not proven by this run."
    );
}

/// Whether GDK can advertise the sandbox spelling at all.
fn portal_is_reachable() -> bool {
    use gtk::gio;
    use gtk::prelude::*;

    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    let owned = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&("org.freedesktop.portal.FileTransfer",).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    );
    owned
        .ok()
        .and_then(|reply| reply.child_value(0).get::<bool>())
        .unwrap_or(false)
}

pub fn an_export_that_produced_nothing_refuses_the_drop() {
    // `gdk_file_list_new_from_array` returns NULL for an empty array and gdk4-rs panics, so this
    // guards an abort too.
    let empty = LazyFiles::for_messages(
        vec![postio_model::MessageId::new(1)],
        Rc::new(|_| Box::pin(async { Ok(Vec::new()) })),
    );
    let stream = gio::MemoryOutputStream::new_resizable();
    let outcome =
        block_on(empty.write_mime_type_future("text/uri-list", &stream, glib::Priority::DEFAULT));
    assert!(outcome.is_err(), "an empty export must refuse the drop");

    let unwired = glib::Object::builder::<LazyFiles>().build();
    let stream = gio::MemoryOutputStream::new_resizable();
    let outcome =
        block_on(unwired.write_mime_type_future("text/uri-list", &stream, glib::Priority::DEFAULT));
    assert!(outcome.is_err(), "an unwired export must refuse the drop");
}

pub fn a_failed_export_fails_the_drop() {
    let materialise: Materialise =
        Rc::new(|_| Box::pin(async { Err("That message is still downloading".to_string()) }));
    let provider = LazyFiles::for_messages(vec![postio_model::MessageId::new(1)], materialise);

    let stream = gio::MemoryOutputStream::new_resizable();
    let outcome = block_on(provider.write_mime_type_future(
        "text/uri-list",
        &stream,
        glib::Priority::DEFAULT,
    ));

    let error = outcome.expect_err("the drop must fail");
    assert!(
        error.to_string().contains("still downloading"),
        "the reason must survive to the receiver: {error}"
    );
}
