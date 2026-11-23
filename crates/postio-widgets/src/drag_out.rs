//! Dragging mail out: a lazy content provider that offers a file list and writes nothing until the
//! drop lands, since a drag is speculative and a selection can be a predicate over a whole mailbox.
//! It advertises the `gdk::FileList` type, not `message/rfc822`, so GDK offers every spelling,
//! including the portal file transfer that carries files out of Flatpak. The widgets may not open
//! the store: [`Materialise`] is the seam the app fills.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use postio_model::MessageId;

/// Asynchronous because it may have to fetch an unbackfilled message source. The error is prose for
/// the user.
pub type Produce =
    Rc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<Vec<gio::File>, String>> + 'static>>>;

/// Names messages, not a selection: a selection can be a predicate, which a file manager cannot be
/// handed.
pub type Materialise = Rc<
    dyn Fn(
        Vec<MessageId>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<gio::File>, String>> + 'static>>,
>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct LazyFiles {
        pub(super) produce: RefCell<Option<Produce>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LazyFiles {
        const NAME: &'static str = "PostioLazyFiles";
        type Type = super::LazyFiles;
        type ParentType = gdk::ContentProvider;
    }

    impl ObjectImpl for LazyFiles {}

    impl ContentProviderImpl for LazyFiles {
        fn formats(&self) -> gdk::ContentFormats {
            // Start from the type and let GDK name the mime types: a provider advertising only a
            // GType is never asked for `text/uri-list` inside a `GdkContentProviderUnion`, which
            // routes writes to the child whose formats name the mime type, so every drop outside
            // the app would fail.
            gdk::ContentFormats::builder()
                .add_type(gdk::FileList::static_type())
                .build()
                .union_serialize_mime_types()
        }

        fn write_mime_type_future(
            &self,
            mime_type: &str,
            stream: &gio::OutputStream,
            io_priority: glib::Priority,
        ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'static>> {
            // The only place anything is written to disk; an abandoned drag stops before it.
            let produce = self.produce.borrow().clone();
            let (stream, mime_type) = (stream.clone(), mime_type.to_string());

            Box::pin(async move {
                let Some(produce) = produce else {
                    // No handler registered: refuse, since a silent empty drop looks like it
                    // worked.
                    return Err(glib::Error::new(
                        gio::IOErrorEnum::NotSupported,
                        "This build cannot export messages as files",
                    ));
                };
                let files = produce().await.map_err(|reason| {
                    glib::Error::new(gio::IOErrorEnum::Failed, reason.as_str())
                })?;
                if files.is_empty() {
                    // `gdk_file_list_new_from_array` returns NULL for an empty array and gdk4-rs
                    // panics, so refuse.
                    return Err(glib::Error::new(
                        gio::IOErrorEnum::NotFound,
                        "There was nothing to export",
                    ));
                }
                let value = gdk::FileList::from_array(&files).to_value();
                gdk::content_serialize_future(&stream, &mime_type, &value, io_priority).await
            })
        }
    }
}

glib::wrapper! {
    pub struct LazyFiles(ObjectSubclass<imp::LazyFiles>)
        @extends gdk::ContentProvider;
}

impl LazyFiles {
    pub fn new(produce: Produce) -> Self {
        let provider: Self = glib::Object::new();
        provider.imp().produce.replace(Some(produce));
        provider
    }

    pub fn for_messages(messages: Vec<MessageId>, materialise: Materialise) -> Self {
        LazyFiles::new(Rc::new(move || materialise(messages.clone())))
    }
}
