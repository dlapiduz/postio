//! Materialising mail as `.eml` files for a drag out. An export is a copy of the stored raw source,
//! not a serialisation; the
//! laziness is upstream in [`crate::drag_out`], which calls here only when a drop asks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use postio_client::Client;
use postio_model::MessageId;
use postio_model::listing::MailStore;

/// Filenames are bounded at 255 bytes on ext4 and btrfs and a subject can be a paragraph.
const MAX_STEM: usize = 96;

/// Anything that could steer where the file lands is removed rather than escaped: a subject of
/// `../../.bashrc` must stay
/// inside its directory.
pub fn eml_name(subject: Option<&str>) -> String {
    let stem = subject.map(sanitize).filter(|stem| !stem.is_empty());
    match stem {
        Some(stem) => format!("{stem}.eml"),
        None => "message.eml".to_string(),
    }
}

fn sanitize(subject: &str) -> String {
    // A newline in a filename is legal on Linux and ruins every tool that reads a list of them.
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
    // A name of only dots is `.` or `..`, a directory.
    stem = stem.trim_matches(['.', ' '].as_slice()).to_string();
    truncate_chars(&stem, MAX_STEM)
}

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

/// Twelve messages in one thread share a subject, so unsuffixed names would write one file twelve
/// times. The suffix counts
/// within this export only.
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

pub fn message_targets(
    into: &Path,
    messages: &[MessageId],
    subjects: &[Option<String>],
) -> Vec<(MessageId, PathBuf)> {
    // Every subject first, so names are unique across the whole drag before any is written.
    let names = unique_names(subjects.iter().map(Option::as_deref));
    messages
        .iter()
        .zip(names)
        .map(|(message, name)| (*message, into.join(name)))
        .collect()
}

/// Returns the files in the order asked for, so the receiver's `text/uri-list` follows the
/// selection order. A message whose
/// raw source is not backfilled is fetched by the host and waited on: the user dragged it by name.
/// With no engine that
/// message is an error, not an empty file.
pub async fn export_messages(
    client: &Client,
    into: &Path,
    messages: &[MessageId],
) -> Result<Vec<PathBuf>, String> {
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

/// Registered once when the window is fed; a build that skips it drags inside Postio and offers
/// nothing to the desktop.
/// A receiver reads the path after the drop, so the files go in the cache directory (`export_dir`),
/// where the system may
/// reclaim them. `runtime` awaits the writes off the main loop: the host may be another process and
/// may wait on a backfill.
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
        // Cutting by bytes would split a multi-byte character.
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
