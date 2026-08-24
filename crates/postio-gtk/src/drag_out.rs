//! Dragging mail out of Postio.
//!
//! Dropping messages into a file manager, another mail client or an editor
//! means handing over files. This module is the *offer*: a content provider
//! that says "I can give you files" and does not produce a single one until
//! the drop actually lands somewhere.
//!
//! # Why it has to be lazy
//!
//! A drag is speculative. Most drags are abandoned, and a selection here can
//! be a predicate over a whole mailbox — `spec.md` §18's rule that nothing
//! ever materialises a mailbox applies to files on disk exactly as it applies
//! to rows in memory. Writing an `.eml` per selected message at drag *start*
//! would mean a person picking up five hundred messages and putting them
//! straight back down had just written five hundred files for nothing.
//!
//! So [`MessageFiles`] holds a description of what was dragged and a callback,
//! and GDK only calls that callback when a receiving application asks for the
//! bytes — which is to say, on the drop.
//!
//! # Why it advertises a file list rather than `message/rfc822`
//!
//! A receiver that is handed `message/rfc822` bytes has one message and
//! nowhere to put it; a file manager wants files. GDK already knows how to
//! serialise a [`gdk::FileList`] into both `text/uri-list` and
//! `application/vnd.portal.filetransfer`, and the second of those is what
//! carries files out of a Flatpak sandbox through the document portal. So this
//! advertises the *type* and lets GDK offer every spelling it knows, rather
//! than naming mime types by hand and losing the sandboxed path.
//!
//! # This crate does not know what a message is made of
//!
//! `postio-gtk` may not depend on `rusqlite`, so the bytes cannot come from
//! here. [`Materialise`] is the seam: `postio-app` registers a callback that
//! turns what was dragged into files, and this module never learns how.

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

/// Turn the dragged messages into files, when somebody finally asks for them.
///
/// Named messages rather than a selection, and deliberately: a selection can
/// be the predicate "everything in this mailbox", and there is no such thing
/// as a predicate handed to a file manager. Resolving it is the list's job —
/// see `MessageListView`'s drag source, which offers files only for a
/// selection it can name.
///
/// Asynchronous because it may have to fetch: a message whose source has not
/// been backfilled has nothing to export yet, and the drop is the moment the
/// user asked for it. The error is prose for the user — the drop failed and
/// they are entitled to know why.
pub type Materialise = Rc<
    dyn Fn(
        Vec<MessageId>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<gio::File>, String>> + 'static>>,
>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MessageFiles {
        pub(super) messages: RefCell<Option<Vec<MessageId>>>,
        pub(super) materialise: RefCell<Option<Materialise>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageFiles {
        const NAME: &'static str = "PostioMessageFiles";
        type Type = super::MessageFiles;
        type ParentType = gdk::ContentProvider;
    }

    impl ObjectImpl for MessageFiles {}

    impl ContentProviderImpl for MessageFiles {
        fn formats(&self) -> gdk::ContentFormats {
            // Start from the *type* and let GDK name the mime types, rather
            // than naming them here. That is what keeps the portal spelling --
            // `application/vnd.portal.filetransfer`, the one that carries
            // files out of a Flatpak sandbox -- without this module having to
            // know it exists, and what would go on working if GDK learned
            // another one.
            //
            // The expansion is not cosmetic. A provider that advertises only a
            // GType is never asked for `text/uri-list` when it sits inside a
            // `GdkContentProviderUnion`: the union routes a write to the child
            // whose formats *name* that mime type, and a type alone names
            // none. The drag source offers exactly such a union -- Postio's
            // own string payload beside these files -- so without this, every
            // drop outside the application fails with "Cannot provide contents
            // as text/uri-list".
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
            // Here, and only here, does anything get written to disk. Every
            // abandoned drag stops before this line.
            let messages = self.messages.borrow().clone();
            let materialise = self.materialise.borrow().clone();
            let (stream, mime_type) = (stream.clone(), mime_type.to_string());

            Box::pin(async move {
                let (Some(messages), Some(materialise)) = (messages, materialise) else {
                    // No handler registered: the application did not wire the
                    // export seam. Refusing is right -- a drop that silently
                    // produced nothing would look like it had worked.
                    return Err(glib::Error::new(
                        gio::IOErrorEnum::NotSupported,
                        "This build cannot export messages as files",
                    ));
                };
                let files = materialise(messages)
                    .await
                    .map_err(|reason| glib::Error::new(gio::IOErrorEnum::Failed, &reason))?;
                if files.is_empty() {
                    // `gdk_file_list_new_from_array` returns NULL for an empty
                    // array and gdk4-rs turns that into a panic, so this is a
                    // guard against aborting the application and not only a
                    // matter of taste. Refusing is also the right answer: a
                    // drop that handed over nothing and reported success is a
                    // drag the user believes worked.
                    return Err(glib::Error::new(
                        gio::IOErrorEnum::NotFound,
                        "There was nothing to export",
                    ));
                }
                // Handing back the type and letting GDK serialise it means
                // `text/uri-list` and the portal token are written by the same
                // code the rest of GTK uses.
                let value = gdk::FileList::from_array(&files).to_value();
                gdk::content_serialize_future(&stream, &mime_type, &value, io_priority).await
            })
        }
    }
}

glib::wrapper! {
    /// The files a drag of messages will hand over, produced on demand.
    pub struct MessageFiles(ObjectSubclass<imp::MessageFiles>)
        @extends gdk::ContentProvider;
}

impl MessageFiles {
    /// Offer the files for `messages`, produced by `materialise` at drop time.
    pub fn new(messages: Vec<MessageId>, materialise: Materialise) -> Self {
        let provider: Self = glib::Object::new();
        provider.imp().messages.replace(Some(messages));
        provider.imp().materialise.replace(Some(materialise));
        provider
    }
}
