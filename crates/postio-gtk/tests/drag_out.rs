//! Dragging mail out of Postio produces files, and produces them *late*.
//!
//! The laziness is the whole design and it is invisible from the outside: a
//! provider that writes five hundred `.eml` files at drag start and one that
//! writes them at the drop look identical to anyone dragging, right up until
//! somebody picks up a large selection and puts it straight back down. So it
//! is asserted here rather than reasoned about.
//!
//! Nothing here touches the network or a database — the export callback is the
//! seam `postio-app` fills in, and a test can fill it with anything.
//!
//! One test function, deliberately: GTK initialises once per process and only
//! from one thread, and Rust's harness gives every `#[test]` a thread of its
//! own. Split into four, three of them race the init and fail — which they
//! did, intermittently, before this was one function. `gtk_style.rs` carries
//! the same note for the same reason.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gdk;
use gtk::gio;
use gtk::prelude::*;
use postio_gtk::drag_out::{LazyFiles, Materialise};

/// A provider whose export callback counts how often it ran.
fn provider(calls: &Rc<Cell<usize>>, files: Vec<gio::File>) -> LazyFiles {
    let calls = Rc::clone(calls);
    let materialise: Materialise = Rc::new(move |_messages| {
        calls.set(calls.get() + 1);
        let files = files.clone();
        Box::pin(async move { Ok(files) })
    });
    LazyFiles::for_messages(vec![postio_model::MessageId::new(1)], materialise)
}

/// Drive the main context until `future` finishes, the way GTK would.
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    glib::MainContext::default().block_on(future)
}
#[test]

fn dragging_messages_out_hands_over_files() {
    if gtk::init().is_err() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }

    nothing_is_written_until_the_drop_asks();
    the_panes_a_drag_crosses_can_scroll_under_it();
    the_sandboxed_spelling_is_offered_too();
    an_export_that_produced_nothing_refuses_the_drop();
    a_failed_export_fails_the_drop();
}

/// The one that justifies the whole module.
fn nothing_is_written_until_the_drop_asks() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("Lunch on Thursday.eml");
    std::fs::write(
        &path,
        b"From: Ada Lovelace <ada@example.com>\r\n\r\nHalf past twelve?\r\n",
    )
    .expect("a file");

    let calls = Rc::new(Cell::new(0));
    let provider = provider(&calls, vec![gio::File::for_path(&path)]);

    // ── the drag has started ────────────────────────────────────────────
    assert_eq!(
        calls.get(),
        0,
        "picking messages up must not write anything: an abandoned drag of a \
         large selection would have written a file per message for nothing"
    );

    // Asking what it *can* give is not asking for it. A receiver reads the
    // formats on every drag that passes over it.
    let formats = provider.formats();
    assert!(
        formats.types().contains(&gdk::FileList::static_type()),
        "a receiver that cannot see a file list will never ask for one: {:?}",
        formats.types()
    );
    assert_eq!(calls.get(), 0, "reading the formats is not a drop");

    // ── the drop lands ──────────────────────────────────────────────────
    let stream = gio::MemoryOutputStream::new_resizable();
    block_on(provider.write_mime_type_future("text/uri-list", &stream, glib::Priority::DEFAULT))
        .expect("the drop is served");

    assert_eq!(calls.get(), 1, "the drop is what materialises the files");

    // `steal_as_bytes` wants the stream finished; GDK leaves it to the caller
    // to close, exactly as a real drop does.
    stream.close(gio::Cancellable::NONE).expect("it closes");
    let written = stream.steal_as_bytes();
    let written = String::from_utf8_lossy(&written);
    assert!(
        written.contains("Lunch%20on%20Thursday.eml"),
        "the receiver was handed no usable uri: {written:?}"
    );
}

/// A folder below the fold has to be reachable without putting the drag down.
///
/// What this proves is that the controller is *attached* — the failure it
/// exists to catch is nobody calling `autoscroll::attach`, which leaves a
/// perfectly correct ramp wired to nothing. How far each tick moves is
/// `autoscroll`'s own unit tests; emitting a real drag motion needs a real
/// drag, which no headless harness can start.
fn the_panes_a_drag_crosses_can_scroll_under_it() {
    fn scrolls_under_a_drag(widget: &gtk::Widget) -> bool {
        let mut queue = vec![widget.clone()];
        while let Some(current) = queue.pop() {
            if current.downcast_ref::<gtk::ScrolledWindow>().is_some() {
                let controllers = current.observe_controllers();
                for index in 0..controllers.n_items() {
                    if controllers
                        .item(index)
                        .and_downcast::<gtk::DropControllerMotion>()
                        .is_some()
                    {
                        return true;
                    }
                }
            }
            let mut child = current.first_child();
            while let Some(node) = child {
                queue.push(node.clone());
                child = node.next_sibling();
            }
        }
        false
    }

    let sidebar = postio_gtk::sidebar::Sidebar::default();
    assert!(
        scrolls_under_a_drag(sidebar.upcast_ref()),
        "a folder below the fold cannot be dropped on: the sidebar does not scroll under a drag"
    );

    let list = postio_gtk::list_view::MessageListView::default();
    assert!(
        scrolls_under_a_drag(list.upcast_ref()),
        "the message list does not scroll under a drag"
    );
}

/// Under Flatpak this is the spelling that carries files out of the sandbox.
fn the_sandboxed_spelling_is_offered_too() {
    let calls = Rc::new(Cell::new(0));
    let provider = provider(&calls, Vec::new());
    // The step GDK itself takes before putting a drag on the wire: a
    // type-only format list carries no mime types until the registered
    // serialisers are folded in. Asserting on `formats()` alone would assert
    // on an empty list and pass for the wrong reason.
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
    // The reason the provider advertises the *type* rather than naming mime
    // types by hand: naming them would silently drop this one, and drag-out
    // would work on the host and fail in the sandbox — which is the one
    // combination nobody would notice until a user reported it.
    // Asserted only where GDK could offer it. The spelling is registered by
    // GDK when the FileTransfer portal is reachable on a session bus, so on a
    // desktop it is there and on a bare CI runner -- no session bus at all --
    // it never can be, and asserting it there tests the runner rather than
    // Postio. `dbus-run-session` with xdg-desktop-portal installed does not
    // close the gap: the daemon starts and then fails to reach the secrets
    // service, so the name is still unowned.
    //
    // The skip is loud on purpose. A quiet one is how every GTK test in this
    // workspace came to "pass" without a display (#781), and the assertion
    // below is worth more than most: its own point is that hand-naming the
    // mime types would work on the host and fail in the sandbox, which is the
    // one combination nobody notices until a user reports it. #121 is where
    // that path is proven for real, under Flatpak.
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

/// Whether `org.freedesktop.portal.FileTransfer` has an owner on the session
/// bus -- which is what decides whether GDK can advertise the sandbox
/// spelling at all.
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

/// A drop that hands over nothing must not report success.
fn an_export_that_produced_nothing_refuses_the_drop() {
    // It also cannot be handed to GDK at all: `gdk_file_list_new_from_array`
    // returns NULL for an empty array and gdk4-rs turns that into a panic, so
    // this guards an abort and not only a lie.
    let empty = LazyFiles::for_messages(
        vec![postio_model::MessageId::new(1)],
        Rc::new(|_| Box::pin(async { Ok(Vec::new()) })),
    );
    let stream = gio::MemoryOutputStream::new_resizable();
    let outcome =
        block_on(empty.write_mime_type_future("text/uri-list", &stream, glib::Priority::DEFAULT));
    assert!(outcome.is_err(), "an empty export must refuse the drop");

    // And a build that never registered the seam at all.
    let unwired = glib::Object::builder::<LazyFiles>().build();
    let stream = gio::MemoryOutputStream::new_resizable();
    let outcome =
        block_on(unwired.write_mime_type_future("text/uri-list", &stream, glib::Priority::DEFAULT));
    assert!(outcome.is_err(), "an unwired export must refuse the drop");
}

/// The message was never downloaded and could not be fetched.
fn a_failed_export_fails_the_drop() {
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
