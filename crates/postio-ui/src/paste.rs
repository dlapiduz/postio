//! What a paste is: files to attach, or text to insert.
//!
//! A terminal hands a dropped file to the program as its path, pasted --
//! `'/home/ada/My Report.pdf'`, `/home/ada/My\ Report.pdf`, or a
//! `file:///home/ada/My%20Report.pdf` URI -- and a paste of copied files
//! arrives the same way. That is the whole of drag and drop in a terminal
//! (research R7). So a paste is read as files only when **every** word in it
//! is a path; a sentence that happens to mention one is a sentence.
//!
//! Of the paths, a readable regular file is attached. One that clearly names
//! a file and cannot be had -- a folder, no permission, a `file://` that
//! points nowhere -- is said, by name, and nothing else changes. A bare path
//! to nothing is text: a person can type a path into a message.

use std::path::{Path, PathBuf};

/// One part of a paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteItem {
    /// A file to attach.
    File(PathBuf),
    /// A file that was named and cannot be attached, and why.
    Unreadable {
        /// Which.
        path: PathBuf,
        /// Why, as a sentence.
        reason: String,
    },
    /// Words to insert.
    Text(String),
}

/// What `pasted` is.
pub fn classify(pasted: &str) -> Vec<PasteItem> {
    let text = || vec![PasteItem::Text(pasted.to_owned())];
    let Some(words) = words(pasted) else {
        return text();
    };
    if words.is_empty() {
        return text();
    }
    let mut items = Vec::with_capacity(words.len());
    for word in &words {
        let (path, from_uri) = match word.strip_prefix("file://") {
            Some(rest) => (PathBuf::from(percent_decode(rest)), true),
            None if word.starts_with('/') => (PathBuf::from(word), false),
            None => match word.strip_prefix("~/") {
                Some(rest) => match std::env::var_os("HOME") {
                    Some(home) => (Path::new(&home).join(rest), false),
                    None => return text(),
                },
                // Not a path, so the paste is words.
                None => return text(),
            },
        };
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => match std::fs::File::open(&path) {
                Ok(_) => items.push(PasteItem::File(path)),
                Err(error) => items.push(PasteItem::Unreadable {
                    reason: format!("{} cannot be read: {error}", path.display()),
                    path,
                }),
            },
            Ok(meta) if meta.is_dir() => items.push(PasteItem::Unreadable {
                reason: format!(
                    "{} is a folder; attach the files in it instead",
                    path.display()
                ),
                path,
            }),
            Ok(_) => items.push(PasteItem::Unreadable {
                reason: format!("{} is not a file", path.display()),
                path,
            }),
            // A `file://` names a file on purpose; a bare path to nothing may
            // be someone typing a path into a message.
            Err(error) if from_uri => items.push(PasteItem::Unreadable {
                reason: format!("{} is not there: {error}", path.display()),
                path,
            }),
            Err(_) => return text(),
        }
    }
    items
}

/// `pasted` split into words the way a shell would, quotes and backslash
/// escapes and all -- how a terminal writes a dropped path with a space in
/// it. `None` for a paste with an unclosed quote, which is words.
fn words(pasted: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = pasted.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// `%20` and friends, as a `file://` URI spells them.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = text.get(index + 1..index + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| {
                let path = dir.join(name);
                std::fs::write(&path, b"x").unwrap();
                path
            })
            .collect()
    }

    #[test]
    fn a_dropped_file_is_attached() {
        let dir = tempfile::tempdir().unwrap();
        let paths = files(dir.path(), &["report.pdf"]);
        assert_eq!(
            classify(&paths[0].display().to_string()),
            vec![PasteItem::File(paths[0].clone())]
        );
    }

    #[test]
    fn quoted_escaped_and_uri_forms_all_name_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = files(dir.path(), &["My Report.pdf", "b c.png", "d.txt"]);
        let quoted = format!("'{}'", paths[0].display());
        let escaped = paths[1].display().to_string().replace(' ', "\\ ");
        let uri = format!(
            "file://{}",
            paths[2].display().to_string().replace(' ', "%20")
        );
        let pasted = format!("{quoted} {escaped}\n{uri}");
        assert_eq!(
            classify(&pasted),
            paths
                .iter()
                .cloned()
                .map(PasteItem::File)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_sentence_that_mentions_a_path_is_text() {
        let dir = tempfile::tempdir().unwrap();
        let paths = files(dir.path(), &["notes.txt"]);
        let pasted = format!("the notes are in {} if you need them", paths[0].display());
        assert_eq!(classify(&pasted), vec![PasteItem::Text(pasted.clone())]);
    }

    #[test]
    fn a_folder_is_named_and_not_attached() {
        let dir = tempfile::tempdir().unwrap();
        let items = classify(&dir.path().display().to_string());
        assert!(
            matches!(&items[..], [PasteItem::Unreadable { path, .. }] if path == dir.path()),
            "{items:?}"
        );
    }

    #[test]
    fn a_file_uri_to_nothing_is_named() {
        let items = classify("file:///nowhere/at/all.pdf");
        assert!(
            matches!(&items[..], [PasteItem::Unreadable { .. }]),
            "{items:?}"
        );
    }

    #[test]
    fn a_bare_path_to_nothing_is_just_text() {
        assert_eq!(
            classify("/usr/local/thing-that-is-not-here"),
            vec![PasteItem::Text("/usr/local/thing-that-is-not-here".into())]
        );
    }

    #[test]
    fn plain_words_are_text() {
        assert_eq!(
            classify("Hello there"),
            vec![PasteItem::Text("Hello there".into())]
        );
    }
}
