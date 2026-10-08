//! Materialising mail as files, for dragging out of Postio (T245).
//!
//! Dropping messages into a file manager, another mail client or an editor
//! means handing over *files*, so something has to turn a row in SQLite into
//! bytes on disk with a name a person would recognise. That is this module.
//!
//! # Nothing is written until the drop lands
//!
//! A drag of a large selection must not write a file per message on the
//! chance that it is dropped somewhere — see [`crate::drag_out`], which offers a
//! content provider that calls in here only when the receiving application
//! actually asks for the data. This module is the half that does the work; the
//! laziness is upstream of it, and the tests that hold it there are
//! `widgets_suite`'s.
//!
//! # The bytes are already stored
//!
//! An `.eml` file *is* the raw RFC 5322 source, which the sync engine has
//! already put in the blob store as `messages.raw_blob_id`. So an export is a
//! copy, not a serialisation — there is no round trip through the parser and
//! nothing that could make the file disagree with what the server sent.
//!
//! The copy is the store's owner's (`Req::ExportMessages`,
//! `postio_host::export`); what each file is called is decided here, the way
//! a saved part's path is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use postio_client::Client;
use postio_model::MessageId;
use postio_model::listing::MailStore;

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

/// Where each of `messages` goes under `into`, named by its subject and
/// never colliding with another in the same drag. `subjects` are the
/// messages', in the same order.
pub fn message_targets(
    into: &Path,
    messages: &[MessageId],
    subjects: &[Option<String>],
) -> Vec<(MessageId, PathBuf)> {
    // Every subject first, so the names can be made unique across the whole
    // drag before any of them is written. Doing it per message would need the
    // set anyway, one file at a time, and would rename as it went.
    let names = unique_names(subjects.iter().map(Option::as_deref));
    messages
        .iter()
        .zip(names)
        .map(|(message, name)| (*message, into.join(name)))
        .collect()
}

/// Write every message in `messages` into `into` as an `.eml` file.
///
/// Returns the files in the order they were asked for, so the caller can hand
/// a receiving application a `text/uri-list` in the order the person selected
/// them rather than in whatever order the reads finished.
///
/// Two calls, where it was a read per message and then the writes: the rows
/// for the names, then the whole batch for the host to write.
///
/// # It may reach the network, and only because the user asked
///
/// A message whose raw source has not been backfilled yet has nothing to
/// export, so the host asks the engine for it and waits — the same path, and
/// the same justification, as saving an attachment that was never downloaded.
/// The user dragged these messages by name; fetching them is the thing they
/// asked for. With no engine, that message is an error rather than an empty
/// file.
pub async fn export_messages(
    client: &Client,
    into: &Path,
    messages: &[MessageId],
) -> Result<Vec<PathBuf>, String> {
    // A row that cannot be read is named from no subject, as before: the
    // write below is what says whether the message can be exported at all.
    let rows = client
        .message_rows(messages.to_vec())
        .await
        .unwrap_or_default();
    let subjects: Vec<Option<String>> = messages
        .iter()
        .map(|message| {
            rows.iter()
                .find(|row| row.id == *message)
                .and_then(|row| row.subject.clone())
        })
        .collect();
    let targets = message_targets(into, messages, &subjects);

    std::fs::create_dir_all(into).map_err(|error| error.to_string())?;
    client
        .export_messages(targets)
        .await
        .map_err(|error| error.to_string())
}

/// The list's export seam, answered through `client`: what the list's drag
/// hands a file manager is produced here, when the drop asks.
///
/// Both desktop apps' lists offer a drag; this is the half they cannot have,
/// registered once when the window is fed -- a build that skips it drags
/// perfectly well inside Postio and simply offers nothing to the desktop.
///
/// # Why the files go in the cache
///
/// A receiving application is handed a *path* and reads it after the drop, so
/// the bytes have to exist somewhere both processes can see. They are copies
/// of mail that is already stored, so `export_dir` -- the app's, which is
/// `postio_session::paths::export_dir` -- puts them under the cache
/// directory, where the system is allowed to reclaim them.
///
/// `runtime` is where the writes are awaited, off the main loop: the store's
/// owner may be another process, and the host's half creates the directory,
/// writes files and may wait on a backfill. The drop is already asynchronous
/// from GTK's point of view, so the one thing that must not happen is doing
/// it on the UI thread.
pub fn materialiser(
    runtime: tokio::runtime::Handle,
    client: Client,
    export_dir: impl Fn() -> PathBuf + 'static,
) -> crate::drag_out::Materialise {
    std::rc::Rc::new(move |messages: Vec<MessageId>| {
        let (client, runtime) = (client.clone(), runtime.clone());
        let into = export_dir();
        Box::pin(async move {
            let (send, receive) = async_channel::bounded(1);
            runtime.spawn(async move {
                let outcome = export_messages(&client, &into, &messages).await;
                let _ = send.send(outcome).await;
            });
            let paths = receive
                .recv()
                .await
                .map_err(|_| "The export did not finish".to_string())??;
            Ok(paths.iter().map(gtk::gio::File::for_path).collect())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
