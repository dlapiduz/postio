//! Materialising mail as files, for dragging out of Postio.
//!
//! Dropping messages into a file manager, another mail client or an editor
//! means handing over *files*, so something has to turn a row in SQLite into
//! bytes on disk with a name a person would recognise. That is this module.
//!
//! # Nothing is written until the drop lands
//!
//! A drag of a large selection must not write a file per message on the
//! chance that it is dropped somewhere — see [`crate::export`]'s caller in
//! `postio-gtk`, which offers a content provider that calls in here only when
//! the receiving application actually asks for the data. This module is the
//! half that does the work; the laziness is upstream of it, and the test that
//! holds it there is `postio-gtk`'s.
//!
//! # The bytes are already stored
//!
//! An `.eml` file *is* the raw RFC 5322 source, which the sync engine has
//! already put in the blob store as `messages.raw_blob_id`. So an export is a
//! copy, not a serialisation — there is no round trip through the parser and
//! nothing that could make the file disagree with what the server sent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use postio_model::MessageId;
use postio_runtime::Engine;
use postio_storage::{BlobStore, Database};

/// How long an exported filename may get before the extension.
///
/// Filenames are bounded at 255 *bytes* on ext4 and btrfs, and a subject can
/// be a paragraph. 96 leaves room for the extension and a disambiguating
/// suffix while staying long enough that two similar subjects are still
/// telling apart on sight.
const MAX_STEM: usize = 96;

/// A filename for one message dragged out as `.eml`.
///
/// The subject, because that is what the person dragging recognises. Anything
/// that could steer where the file lands is taken out rather than escaped: a
/// message whose subject is `../../.bashrc` must produce a name inside the
/// directory it was given and nowhere else.
pub fn eml_name(subject: Option<&str>) -> String {
    let stem = subject.map(sanitize).filter(|stem| !stem.is_empty());
    match stem {
        Some(stem) => format!("{stem}.eml"),
        // Never the empty string, and never a name that leaks a message id: a
        // filename is something the user sees and may keep.
        None => "message.eml".to_string(),
    }
}

/// The part of a filename that comes from the user's mail.
fn sanitize(subject: &str) -> String {
    // Control characters are not a display problem here, they are a
    // correctness one: a newline in a filename is legal on Linux and ruins
    // every tool that reads a list of them.
    let collapsed: String = subject
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '/' | '\\') {
                ' '
            } else {
                character
            }
        })
        .collect();

    let mut stem = collapsed.split_whitespace().collect::<Vec<_>>().join(" ");
    // A leading dot makes a hidden file, and a name that is only dots is
    // `.` or `..` — a directory, not a file.
    stem = stem.trim_matches(['.', ' '].as_slice()).to_string();
    truncate_chars(&stem, MAX_STEM)
}

/// The first `limit` characters, never splitting one in half.
fn truncate_chars(text: &str, limit: usize) -> String {
    let mut out = String::new();
    for (count, character) in text.chars().enumerate() {
        if count == limit {
            break;
        }
        out.push(character);
    }
    out.trim_end().to_string()
}

/// Names for a whole drag, none of them colliding.
///
/// Twelve messages in one thread have one subject between them, so naming
/// each from its subject alone would write one file twelve times and hand
/// over eleven fewer messages than were dragged. The suffix counts within
/// this export only — it says nothing about what is already in the directory,
/// which is the receiving application's business.
pub fn unique_names<'a>(subjects: impl IntoIterator<Item = Option<&'a str>>) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut names = Vec::new();
    for subject in subjects {
        let name = eml_name(subject);
        let count = seen.entry(name.clone()).or_insert(0);
        *count += 1;
        names.push(if *count == 1 {
            name
        } else {
            let stem = name.strip_suffix(".eml").unwrap_or(&name);
            format!("{stem} ({count}).eml")
        });
    }
    names
}

/// Write every message in `messages` into `into` as an `.eml` file.
///
/// Returns the files in the order they were asked for, so the caller can hand
/// a receiving application a `text/uri-list` in the order the person selected
/// them rather than in whatever order the reads finished.
///
/// # It may reach the network, and only because the user asked
///
/// A message whose raw source has not been backfilled yet has nothing to
/// export, so this asks the engine for it and waits — the same path, and the
/// same justification, as saving an attachment that was never downloaded. The
/// user dragged these messages by name; fetching them is the thing they asked
/// for. With no engine, that message is an error rather than an empty file.
pub async fn export_messages(
    database: &Database,
    blobs: &BlobStore,
    engine: Option<Engine>,
    into: &Path,
    messages: &[MessageId],
) -> Result<Vec<PathBuf>, String> {
    // Every subject first, so the names can be made unique across the whole
    // drag before any of them is written. Doing it per message would need the
    // set anyway, one file at a time, and would rename as it went.
    let subjects: Vec<Option<String>> = messages
        .iter()
        .map(|message| {
            crate::reading::read_message(database, *message)
                .map(|row| row.subject)
                .unwrap_or_default()
        })
        .collect();
    let names = unique_names(subjects.iter().map(Option::as_deref));

    std::fs::create_dir_all(into).map_err(|error| error.to_string())?;

    let mut written = Vec::new();
    for (message, name) in messages.iter().zip(names) {
        let raw = match crate::reading::raw_blob(database, *message)? {
            Some(raw) => raw,
            None => {
                let engine = engine.clone().ok_or(
                    "This account is not syncing, so that message cannot be fetched to export",
                )?;
                if !engine
                    .request_body(*message)
                    .await
                    .map_err(|error| error.message().to_string())?
                {
                    return Err("There is nothing to fetch for that message".into());
                }
                crate::reading::wait_for_body(database, *message).await?
            }
        };

        let bytes = blobs.get(&raw).map_err(|error| error.to_string())?;
        let path = into.join(&name);
        std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use postio_model::Message;
    use postio_storage::repository::MessageRepository;
    use postio_storage::test_support;

    /// A store with an account and an inbox, and a blob directory beside it.
    struct World {
        database: Database,
        blobs: BlobStore,
        account: postio_model::Account,
        inbox: postio_model::MailboxId,
        _directory: tempfile::TempDir,
    }

    fn world() -> World {
        let database = test_support::memory();
        let (account, inbox) = {
            let connection = database.connection().expect("a connection");
            test_support::account_with_inbox(&connection)
        };
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(directory.path()).expect("a blob store");
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
        fn message(&self, subject: Option<&str>, raw: Option<&[u8]>) -> MessageId {
            let connection = self.database.connection().expect("a connection");
            let mut message = Message::new(self.account.id, self.inbox, Utc::now());
            message.subject = subject.map(str::to_string);
            message.raw_blob_id = raw.map(|bytes| self.blobs.put(bytes).expect("a blob"));
            MessageRepository::new(&connection)
                .create(&mut message)
                .expect("a message")
        }
    }

    /// The corpus spells mail this way; so does every fixture in this repo.
    const RAW: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Lunch on Thursday\r\n\
\r\n\
Half past twelve?\r\n";

    fn exported(
        world: &World,
        into: &Path,
        messages: &[MessageId],
    ) -> Result<Vec<PathBuf>, String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(export_messages(
                &world.database,
                &world.blobs,
                None,
                into,
                messages,
            ))
    }

    #[test]
    fn an_exported_message_is_the_bytes_the_server_sent() {
        // An .eml file that is not byte-identical to the source is one another
        // client may refuse, and the whole point of dragging out is that it
        // opens somewhere else.
        let world = world();
        let message = world.message(Some("Lunch on Thursday"), Some(RAW));
        let into = tempfile::tempdir().expect("a directory");

        let files = exported(&world, into.path(), &[message]).expect("it exports");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_name().unwrap().to_str().unwrap(),
            "Lunch on Thursday.eml"
        );
        assert_eq!(std::fs::read(&files[0]).expect("a file"), RAW);
    }

    #[test]
    fn a_whole_thread_arrives_as_separate_files() {
        let world = world();
        let messages: Vec<MessageId> = (0..3)
            .map(|_| world.message(Some("Lunch on Thursday"), Some(RAW)))
            .collect();
        let into = tempfile::tempdir().expect("a directory");

        let files = exported(&world, into.path(), &messages).expect("it exports");

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

    #[test]
    fn the_files_come_back_in_the_order_they_were_asked_for() {
        // The drop hands over a `text/uri-list`, and a list in a different
        // order than the person selected reads as the wrong mail.
        let world = world();
        let first = world.message(Some("One"), Some(b"one"));
        let second = world.message(Some("Two"), Some(b"two"));
        let into = tempfile::tempdir().expect("a directory");

        let files = exported(&world, into.path(), &[second, first]).expect("it exports");

        let names: Vec<&str> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["Two.eml", "One.eml"]);
    }

    #[test]
    fn a_subject_cannot_write_outside_the_directory_it_was_given() {
        // The naming rules are unit-tested above; this is the one that matters
        // in practice, because it is a real write to a real filesystem.
        let world = world();
        let message = world.message(Some("../../escaped"), Some(RAW));
        let into = tempfile::tempdir().expect("a directory");

        let files = exported(&world, into.path(), &[message]).expect("it exports");

        assert_eq!(files[0].parent(), Some(into.path()));
        assert!(files[0].exists());
    }

    #[test]
    fn a_message_that_was_never_downloaded_is_an_error_not_an_empty_file() {
        // Silence here would hand a file manager a zero-byte .eml, which
        // looks like a successful drag and is a lost message.
        let world = world();
        let message = world.message(Some("Never fetched"), None);
        let into = tempfile::tempdir().expect("a directory");

        let outcome = exported(&world, into.path(), &[message]);

        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(
            std::fs::read_dir(into.path()).unwrap().count(),
            0,
            "nothing should have been written"
        );
    }

    #[test]
    fn exporting_nothing_writes_nothing_and_is_not_an_error() {
        let world = world();
        let into = tempfile::tempdir().expect("a directory");
        assert_eq!(exported(&world, into.path(), &[]), Ok(Vec::new()));
    }

    #[test]
    fn a_subject_becomes_the_filename() {
        assert_eq!(eml_name(Some("Lunch on Thursday")), "Lunch on Thursday.eml");
    }

    #[test]
    fn a_message_with_no_subject_still_gets_a_name() {
        assert_eq!(eml_name(None), "message.eml");
        assert_eq!(eml_name(Some("   ")), "message.eml");
    }

    #[test]
    fn a_subject_cannot_steer_where_the_file_lands() {
        // The whole reason this function exists rather than the subject being
        // used directly.
        let name = eml_name(Some("../../.bashrc"));
        assert!(!name.contains('/'), "{name}");
        assert!(!name.starts_with('.'), "{name}");
        assert_eq!(name, "bashrc.eml");

        let name = eml_name(Some("a\\b"));
        assert!(!name.contains('\\'), "{name}");
    }

    #[test]
    fn a_name_that_is_only_dots_does_not_become_a_directory() {
        assert_eq!(eml_name(Some(".")), "message.eml");
        assert_eq!(eml_name(Some("..")), "message.eml");
    }

    #[test]
    fn a_newline_in_a_subject_does_not_reach_the_filename() {
        // Legal in a Linux filename and ruinous for everything that reads a
        // list of them, including `text/uri-list`.
        let name = eml_name(Some("Re: quarterly\nreport"));
        assert_eq!(name, "Re: quarterly report.eml");
    }

    #[test]
    fn a_very_long_subject_is_cut_to_something_a_filesystem_takes() {
        let name = eml_name(Some(&"a".repeat(400)));
        assert!(name.len() < 255, "{} bytes", name.len());
        assert!(name.ends_with(".eml"));
    }

    #[test]
    fn a_long_subject_is_never_cut_through_a_character() {
        // Cutting by bytes would split a multi-byte character and produce a
        // name that is not valid UTF-8.
        let name = eml_name(Some(&"é".repeat(400)));
        assert!(name.ends_with(".eml"));
        assert_eq!(name.strip_suffix(".eml").unwrap().chars().count(), MAX_STEM);
    }

    #[test]
    fn one_thread_does_not_write_one_file_twelve_times() {
        let names = unique_names([Some("Lunch"), Some("Lunch"), Some("Lunch")]);
        assert_eq!(
            names,
            vec!["Lunch.eml", "Lunch (2).eml", "Lunch (3).eml"],
            "a drag of a thread must hand over every message in it"
        );
    }

    #[test]
    fn messages_with_no_subject_are_told_apart_too() {
        let names = unique_names([None, None]);
        assert_eq!(names, vec!["message.eml", "message (2).eml"]);
    }

    #[test]
    fn unrelated_subjects_keep_their_own_names() {
        let names = unique_names([Some("One"), Some("Two")]);
        assert_eq!(names, vec!["One.eml", "Two.eml"]);
    }
}
