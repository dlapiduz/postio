//! Materialising a message part as a file, for dragging it out of Postio.
//!
//! Messages are the shared presenter's (`postio_widgets::present::export`,
//! T245); a part is the classic parts panel's own, so what is here is the
//! half that names a [`postio_gtk::parts::Node`].
//!
//! Nothing is written until the drop lands: the content provider is lazy, and
//! calls in here only when the receiving application asks for the data.

use std::path::{Path, PathBuf};

use postio_client::Client;
use postio_model::MessageId;

/// Where one part goes under `into`, and which part it is: the panel's own
/// [`postio_gtk::parts::save_name`], the same one the save dialog offers, so
/// a part saved and a part dragged land under the same name. It already
/// refuses to let a part called `../../.bashrc` steer where the file goes.
///
/// A container is refused: `multipart/mixed` is a wrapper, and writing it
/// would make an empty file named after something that was never a file.
pub(crate) fn part_target(
    into: &Path,
    node: &postio_gtk::parts::Node,
) -> Result<(postio_model::ids::AttachmentId, PathBuf), String> {
    let attachment = node
        .attachment
        .ok_or("That part is not something with bytes of its own")?;
    Ok((attachment, into.join(postio_gtk::parts::save_name(node))))
}

/// Write one message part into `into`, under the name the sender gave it.
///
/// The store's owner writes the bytes (`Req::SavePart`), so a part that was
/// never downloaded is fetched exactly as `s` fetches it — the user named
/// this part by dragging it.
pub async fn export_part(
    client: &postio_client::Client,
    into: &Path,
    message: MessageId,
    node: &postio_gtk::parts::Node,
) -> Result<PathBuf, String> {
    let (attachment, path) = part_target(into, node)?;
    std::fs::create_dir_all(into).map_err(|error| error.to_string())?;
    client
        .save_part(message, attachment, path)
        .await
        .map_err(|error| error.to_string())
}

/// Let the list hand messages to another application as files: the shared
/// presenter's seam, with this app's export directory.
///
/// `client` is the window's: the rows and the writes are its calls.
pub fn install_for(
    window: &postio_gtk::window::Window,
    runtime: tokio::runtime::Handle,
    client: Client,
) {
    window
        .list()
        .connect_export(postio_widgets::present::export::materialiser(
            runtime,
            client,
            crate::paths::export_dir,
        ));
}

/// [`install_for`] on the composition root's runtime.
pub async fn install(window: &postio_gtk::window::Window, wiring: &crate::Wiring, client: Client) {
    install_for(window, wiring.runtime.clone(), client);
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_widgets::present::export::message_targets;

    use chrono::Utc;
    use postio_model::Message;
    use postio_runtime::Engine;
    use postio_storage::repository::MessageRepository;
    use postio_storage::test_support;
    use postio_storage::{BlobStore, Store};

    /// A store with an account and an inbox, and a blob directory beside it.
    struct World {
        database: Store,
        blobs: BlobStore,
        account: postio_model::Account,
        inbox: postio_model::MailboxId,
        _directory: tempfile::TempDir,
    }

    async fn world() -> World {
        let database = test_support::memory().await;
        let (account, inbox) = {
            let connection = database.connect().await.expect("a connection");
            test_support::account_with_inbox(&connection).await
        };
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(directory.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");
        World {
            database,
            blobs,
            account,
            inbox,
            _directory: directory,
        }
    }

    impl World {
        /// A message whose raw source is `raw`, or which has none at all.
        async fn message(&self, subject: Option<&str>, raw: Option<&[u8]>) -> MessageId {
            let connection = self.database.connect().await.expect("a connection");
            let mut message = Message::new(self.account.id, self.inbox, Utc::now());
            message.subject = subject.map(str::to_string);
            message.raw_blob_id = raw.map(|bytes| self.blobs.put(bytes).expect("a blob"));
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message")
        }
    }

    /// [`super::export_messages`] over a store rather than a client: the same
    /// subjects to names, and the files as the host writes them for
    /// `Req::ExportMessages`.
    async fn export_messages(
        database: &Store,
        blobs: &BlobStore,
        engine: Option<Engine>,
        into: &Path,
        messages: &[MessageId],
    ) -> Result<Vec<PathBuf>, String> {
        let mut subjects = Vec::with_capacity(messages.len());
        for message in messages {
            subjects.push(
                postio_host::parts::read_message(database, *message)
                    .await
                    .map(|row| row.subject)
                    .unwrap_or_default(),
            );
        }
        let targets = message_targets(into, messages, &subjects);
        std::fs::create_dir_all(into).map_err(|error| error.to_string())?;
        postio_host::export::write_messages(database, blobs, engine, &targets).await
    }

    /// [`super::export_part`] over a store rather than a client: the same
    /// node to path, and the bytes as the host writes them for
    /// `Req::SavePart`.
    async fn export_part(
        database: &Store,
        blobs: &BlobStore,
        engine: Option<Engine>,
        into: &Path,
        message: MessageId,
        node: &postio_gtk::parts::Node,
    ) -> Result<PathBuf, String> {
        let (attachment, path) = part_target(into, node)?;
        std::fs::create_dir_all(into).map_err(|error| error.to_string())?;
        postio_host::parts::save_part(database, blobs, engine, message, attachment, &path).await?;
        Ok(path)
    }

    /// The corpus spells mail this way; so does every fixture in this repo.
    const RAW: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Lunch on Thursday\r\n\
\r\n\
Half past twelve?\r\n";

    async fn exported(
        world: &World,
        into: &Path,
        messages: &[MessageId],
    ) -> Result<Vec<PathBuf>, String> {
        // Awaited, not blocked on: the test is already a `#[tokio::test]`,
        // and building a second runtime inside one panics.
        export_messages(&world.database, &world.blobs, None, into, messages).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_exported_message_is_the_bytes_the_server_sent() {
        // An .eml file that is not byte-identical to the source is one another
        // client may refuse, and the whole point of dragging out is that it
        // opens somewhere else.
        let world = world().await;
        let message = world.message(Some("Lunch on Thursday"), Some(RAW)).await;
        let into = tempfile::tempdir().expect("a directory");

        let files = exported(&world, into.path(), &[message])
            .await
            .expect("it exports");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_name().unwrap().to_str().unwrap(),
            "Lunch on Thursday.eml"
        );
        assert_eq!(std::fs::read(&files[0]).expect("a file"), RAW);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_whole_thread_arrives_as_separate_files() {
        let world = world().await;
        let mut messages: Vec<MessageId> = Vec::new();
        for _ in 0..3 {
            messages.push(world.message(Some("Lunch on Thursday"), Some(RAW)).await);
        }
        let into = tempfile::tempdir().expect("a directory");

        let files = exported(&world, into.path(), &messages)
            .await
            .expect("it exports");

        assert_eq!(files.len(), 3);
        let names: Vec<String> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "Lunch on Thursday.eml",
                "Lunch on Thursday (2).eml",
                "Lunch on Thursday (3).eml"
            ]
        );
        // Three names is not three files unless all three are on disk.
        for file in &files {
            assert!(file.exists(), "{file:?} was named but never written");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_files_come_back_in_the_order_they_were_asked_for() {
        // The drop hands over a `text/uri-list`, and a list in a different
        // order than the person selected reads as the wrong mail.
        let world = world().await;
        let first = world.message(Some("One"), Some(b"one")).await;
        let second = world.message(Some("Two"), Some(b"two")).await;
        let into = tempfile::tempdir().expect("a directory");

        let files = exported(&world, into.path(), &[second, first])
            .await
            .expect("it exports");

        let names: Vec<&str> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["Two.eml", "One.eml"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_subject_cannot_write_outside_the_directory_it_was_given() {
        // The naming rules are unit-tested above; this is the one that matters
        // in practice, because it is a real write to a real filesystem.
        let world = world().await;
        let message = world.message(Some("../../escaped"), Some(RAW)).await;
        let into = tempfile::tempdir().expect("a directory");

        let files = exported(&world, into.path(), &[message])
            .await
            .expect("it exports");

        assert_eq!(files[0].parent(), Some(into.path()));
        assert!(files[0].exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_that_was_never_downloaded_is_an_error_not_an_empty_file() {
        // Silence here would hand a file manager a zero-byte .eml, which
        // looks like a successful drag and is a lost message.
        let world = world().await;
        let message = world.message(Some("Never fetched"), None).await;
        let into = tempfile::tempdir().expect("a directory");

        let outcome = exported(&world, into.path(), &[message]).await;

        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(
            std::fs::read_dir(into.path()).unwrap().count(),
            0,
            "nothing should have been written"
        );
    }

    /// A two-part message: a text body and a named attachment.
    const WITH_ATTACHMENT: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: The plan\r\n\
Content-Type: multipart/mixed; boundary=\"edge\"\r\n\
\r\n\
--edge\r\n\
Content-Type: text/plain\r\n\
\r\n\
Attached.\r\n\
--edge\r\n\
Content-Type: text/csv\r\n\
Content-Disposition: attachment; filename=\"figures.csv\"\r\n\
\r\n\
one,two\r\n\
--edge--\r\n";

    #[tokio::test(flavor = "multi_thread")]
    async fn an_exported_part_is_the_bytes_the_sender_attached() {
        let world = world().await;
        let parsed = postio_model::mime::parse(WITH_ATTACHMENT);
        let message = {
            let connection = world.database.connect().await.expect("a connection");
            let mut message = Message::new(world.account.id, world.inbox, Utc::now());
            message.subject = Some("The plan".into());
            message.raw_blob_id = Some(world.blobs.put(WITH_ATTACHMENT).expect("a blob"));
            message.attachments = parsed
                .parts
                .iter()
                .map(|part| part.attachment.clone())
                .collect();
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message")
        };

        let row = crate::reading::read_message(&world.database, message)
            .await
            .expect("the row");
        let attachment = row
            .attachments
            .iter()
            .find(|part| part.filename.as_deref() == Some("figures.csv"))
            .expect("the fixture has a named attachment, or this could not fail");

        let node = postio_gtk::parts::Node {
            part_id: attachment.part_id.clone().unwrap_or_default(),
            depth: 1,
            mime: attachment.mime_type.clone(),
            filename: attachment.filename.clone(),
            size: attachment.size,
            downloaded: true,
            last: true,
            attachment: Some(attachment.id),
            content_id: None,
            inline: false,
        };

        let into = tempfile::tempdir().expect("a directory");
        let path = export_part(
            &world.database,
            &world.blobs,
            None,
            into.path(),
            message,
            &node,
        )
        .await
        .expect("it exports");

        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "figures.csv",
            "a dragged part must land under the name the save dialog would offer"
        );
        // No trailing CRLF: per RFC 2046 the line break before a boundary
        // belongs to the boundary, not to the part, and the parser is right to
        // take it off. The file is what the sender attached.
        assert_eq!(std::fs::read(&path).expect("the file"), b"one,two");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_container_has_no_bytes_to_export() {
        // `multipart/mixed` is a wrapper. Exporting it would write an empty
        // file named after something that was never a file.
        let world = world().await;
        let message = world.message(Some("The plan"), Some(WITH_ATTACHMENT)).await;
        let node = postio_gtk::parts::Node {
            part_id: String::new(),
            depth: 0,
            mime: "multipart/mixed".into(),
            filename: None,
            size: 0,
            downloaded: true,
            last: true,
            attachment: None,
            content_id: None,
            inline: false,
        };

        let into = tempfile::tempdir().expect("a directory");
        let outcome = export_part(
            &world.database,
            &world.blobs,
            None,
            into.path(),
            message,
            &node,
        )
        .await;

        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(std::fs::read_dir(into.path()).unwrap().count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exporting_nothing_writes_nothing_and_is_not_an_error() {
        let world = world().await;
        let into = tempfile::tempdir().expect("a directory");
        assert_eq!(exported(&world, into.path(), &[]).await, Ok(Vec::new()));
    }
}
