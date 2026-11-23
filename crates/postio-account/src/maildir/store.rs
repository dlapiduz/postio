use std::path::{Path, PathBuf};

use io_maildir::client::MaildirClient;
use io_maildir::flag::{MaildirFlag, MaildirFlags};
use io_maildir::maildir::{Maildir, MaildirSubdir};
use postio_model::Flag;

use super::UidList;
use crate::backend::BackendError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub path: String,
    pub directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub uid: u32,
    pub file: PathBuf,
    pub flags: Vec<Flag>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub validity: u32,
    pub next: u32,
}

#[derive(Debug, Clone)]
pub struct LocalStore {
    root: PathBuf,
    maildirpp: bool,
}

impl LocalStore {
    /// The layout is read off the tree once: dot-prefixed children beside the root's `cur/new/tmp`
    /// mean Maildir++; otherwise folders are nested directories.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        Self::looks_like_a_maildir(&root)?;
        Ok(Self::at(root))
    }

    /// Unchecked, for a caller that reports an unreadable tree itself.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let maildirpp = root.join("cur").is_dir() && has_dotted_child(&root);
        Self { root, maildirpp }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The message is shown in the Add Account sheet, so it names the directory and what is
    /// missing: a `~/Downloads` picked by mistake must not look like an account with no mail.
    pub fn looks_like_a_maildir(root: &Path) -> Result<(), String> {
        if !root.is_dir() {
            return Err(format!("{} is not a directory.", root.display()));
        }
        if root.join("cur").is_dir() {
            return Ok(());
        }
        let holds_one = std::fs::read_dir(root)
            .map_err(|error| format!("{} could not be read: {error}", root.display()))?
            .filter_map(Result::ok)
            .any(|entry| entry.path().join("cur").is_dir());
        if holds_one {
            Ok(())
        } else {
            Err(format!(
                "{} has no cur/new/tmp in it and holds no folder that has, \
                 so it is not a maildir. Postio will not guess at another format.",
                root.display()
            ))
        }
    }

    pub fn folders(&self) -> Result<Vec<Folder>, BackendError> {
        let listed = self
            .client()
            .list_maildirs()
            .map_err(|error| unreadable("listing the maildir's folders", error))?;
        // An unreadable folder is left out rather than failing the listing; counted, never named,
        // since a folder name is the person's own words.
        if !listed.unreadable.is_empty() {
            tracing::warn!(
                unreadable = listed.unreadable.len(),
                "maildir folders could not be read and were left out"
            );
        }

        let mut folders: Vec<Folder> = listed
            .maildirs
            .into_iter()
            .filter_map(|maildir| {
                let directory = PathBuf::from(maildir.path().to_string());
                let path = self.name_of(&directory)?;
                Some(Folder { path, directory })
            })
            .collect();
        folders.sort_by(|left, right| {
            (left.path != "INBOX", &left.path).cmp(&(right.path != "INBOX", &right.path))
        });
        folders.dedup_by(|left, right| left.path == right.path);
        Ok(folders)
    }

    pub fn folder(&self, path: &str) -> Result<Folder, BackendError> {
        self.folders()?
            .into_iter()
            .find(|folder| folder.path == path)
            .ok_or_else(|| BackendError::NoSuchMailbox {
                path: path.to_owned(),
            })
    }

    /// Idempotent: `create_dir_all` already is. A maildir folder is `cur`, `new` and `tmp`; a bare
    /// directory would list as nothing and read as a folder that lost its mail.
    pub fn create(&self, path: &str) -> Result<Folder, BackendError> {
        let directory = self.directory_for(path)?;
        for subdir in ["cur", "new", "tmp"] {
            std::fs::create_dir_all(directory.join(subdir)).map_err(|error| BackendError::Io {
                context: format!("making the maildir folder {}", directory.display()),
                reason: error.to_string(),
            })?;
        }
        Ok(Folder {
            path: path.to_owned(),
            directory,
        })
    }

    /// Maildir++ nests with dots in one flat name; the fs layout nests directories. Wrong way round
    /// makes a folder the next `LIST` cannot see. A component that is `.`, `..` or empty is
    /// refused, not normalised, so nothing is made outside the user's tree.
    fn directory_for(&self, path: &str) -> Result<PathBuf, BackendError> {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let refuse = |reason: &str| BackendError::Rejected {
            command: "CREATE".to_owned(),
            reason: reason.to_owned(),
        };
        if parts.is_empty() {
            return Err(refuse("a folder with no name cannot be made"));
        }
        if parts
            .iter()
            .any(|part| *part == "." || *part == ".." || part.contains(std::path::MAIN_SEPARATOR))
        {
            return Err(refuse(
                "a folder name that walks out of the maildir will not be made",
            ));
        }
        if self.maildirpp {
            Ok(self.root.join(format!(".{}", parts.join("."))))
        } else {
            Ok(parts
                .iter()
                .fold(self.root.clone(), |directory, part| directory.join(part)))
        }
    }

    /// Numbers anything new and forgets gone names, then writes it back, so the next run answers
    /// with the same numbers and a second pass does not import again.
    pub fn snapshot(&self, folder: &Folder, fresh_validity: u32) -> Result<Snapshot, BackendError> {
        let maildir = Maildir::from_path(folder.directory.display().to_string().as_str());
        let listed = self
            .client()
            .list_entries(maildir)
            .map_err(|error| unreadable("listing a maildir folder's messages", error))?;

        let mut uids = UidList::load(&folder.directory, fresh_validity);
        let mut names = Vec::new();
        let mut entries = Vec::new();
        for entry in listed {
            let file = PathBuf::from(entry.path().to_string());
            let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            names.push(name.to_owned());
            entries.push(Entry {
                uid: uids.uid_for(name),
                flags: flags_of(&entry.flags()),
                file,
            });
        }
        uids.retain(&names);
        entries.sort_by_key(|entry| entry.uid);

        // Best effort: a folder Postio cannot write is still readable, and a failed save costs
        // numbers that move next run.
        if let Err(error) = uids.save(&folder.directory) {
            tracing::warn!(%error, "a maildir folder's UID list could not be saved");
        }

        Ok(Snapshot {
            entries,
            validity: uids.validity(),
            next: uids.next(),
        })
    }

    pub fn read(&self, file: &Path) -> Result<Vec<u8>, BackendError> {
        std::fs::read(file).map_err(|error| BackendError::Io {
            context: "reading a message out of the maildir".to_owned(),
            reason: error.to_string(),
        })
    }

    /// A rename. Only the five letters Postio understands are replaced: dropping another client's
    /// keyword on "mark as read" would lose its data.
    pub fn set_flags(&self, folder: &Folder, id: &str, flags: &[Flag]) -> Result<(), BackendError> {
        let client = self.client();
        let maildir = self.maildir(folder);
        let wanted = maildir_flags(flags);
        let unwanted = maildir_flags(&removable(flags));
        client
            .add_flags(maildir.clone(), id, wanted)
            .map_err(|error| unreadable("setting a message's flags", error))?;
        client
            .remove_flags(maildir, id, unwanted)
            .map_err(|error| unreadable("clearing a message's flags", error))
    }

    pub fn flags(&self, folder: &Folder, id: &str) -> Result<Vec<Flag>, BackendError> {
        let (_path, _subdir, flags) = self
            .client()
            .locate(self.maildir(folder), id)
            .map_err(|error| unreadable("finding a message in the maildir", error))?;
        Ok(flags_of(&flags))
    }

    pub fn move_to(&self, folder: &Folder, id: &str, target: &Folder) -> Result<(), BackendError> {
        self.client()
            .r#move(id, self.maildir(folder), self.maildir(target), None)
            .map(|_renamed| ())
            .map_err(|error| unreadable("moving a message between folders", error))
    }

    pub fn copy_to(&self, folder: &Folder, id: &str, target: &Folder) -> Result<(), BackendError> {
        self.client()
            .copy(id, self.maildir(folder), self.maildir(target), None)
            .map(|_copied| ())
            .map_err(|error| unreadable("copying a message between folders", error))
    }

    pub fn delete(&self, folder: &Folder, id: &str) -> Result<(), BackendError> {
        self.client()
            .delete_entry(self.maildir(folder), id)
            .map_err(|error| unreadable("deleting a message", error))
    }

    /// Lands in `cur` when read and `new` otherwise, so every other client sharing the tree sees a
    /// delivered message.
    pub fn store(
        &self,
        folder: &Folder,
        bytes: Vec<u8>,
        flags: &[Flag],
    ) -> Result<String, BackendError> {
        let subdir = if flags.contains(&Flag::Seen) {
            MaildirSubdir::Cur
        } else {
            MaildirSubdir::New
        };
        self.client()
            .store(self.maildir(folder), subdir, maildir_flags(flags), bytes)
            .map(|(id, _path)| id)
            .map_err(|error| unreadable("writing a message into the maildir", error))
    }

    fn maildir(&self, folder: &Folder) -> Maildir {
        Maildir::from_path(folder.directory.display().to_string().as_str())
    }

    fn client(&self) -> MaildirClient {
        let mut client = MaildirClient::new(self.root.display().to_string().as_str());
        client.store.maildirpp = self.maildirpp;
        client
    }

    /// The root is `INBOX` in either layout.
    fn name_of(&self, directory: &Path) -> Option<String> {
        let relative = directory.strip_prefix(&self.root).ok()?;
        let mut parts: Vec<String> = Vec::new();
        for part in relative.components() {
            let part = part.as_os_str().to_str()?;
            let part = part.strip_prefix('.').unwrap_or(part);
            parts.extend(part.split('.').map(str::to_owned));
        }
        if parts.is_empty() || parts.iter().all(String::is_empty) {
            return Some("INBOX".to_owned());
        }
        if parts.len() == 1 && parts[0].eq_ignore_ascii_case("inbox") {
            return Some("INBOX".to_owned());
        }
        Some(parts.join("/"))
    }
}

/// `Trashed` is `\Deleted`; `Passed` and custom keywords map to nothing but stay in the filename.
fn flags_of(flags: &MaildirFlags) -> Vec<Flag> {
    [
        (MaildirFlag::Seen, Flag::Seen),
        (MaildirFlag::Replied, Flag::Answered),
        (MaildirFlag::Flagged, Flag::Flagged),
        (MaildirFlag::Draft, Flag::Draft),
        (MaildirFlag::Trashed, Flag::Deleted),
    ]
    .into_iter()
    .filter(|(maildir, _)| flags.contains(maildir))
    .map(|(_, postio)| postio)
    .collect()
}

fn maildir_flags(flags: &[Flag]) -> MaildirFlags {
    let mut out = MaildirFlags::default();
    for flag in flags {
        let letter = match flag {
            Flag::Seen => MaildirFlag::Seen,
            Flag::Answered => MaildirFlag::Replied,
            Flag::Flagged => MaildirFlag::Flagged,
            Flag::Draft => MaildirFlag::Draft,
            Flag::Deleted => MaildirFlag::Trashed,
            _ => continue,
        };
        out.extend(MaildirFlags::from_iter([letter]));
    }
    out
}

fn removable(kept: &[Flag]) -> Vec<Flag> {
    [
        Flag::Seen,
        Flag::Answered,
        Flag::Flagged,
        Flag::Draft,
        Flag::Deleted,
    ]
    .into_iter()
    .filter(|flag| !kept.contains(flag))
    .collect()
}

fn has_dotted_child(root: &Path) -> bool {
    let Ok(children) = std::fs::read_dir(root) else {
        return false;
    };
    children.filter_map(Result::ok).any(|child| {
        child
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.') && child.path().join("cur").is_dir())
    })
}

fn unreadable(context: &str, error: impl std::fmt::Display) -> BackendError {
    BackendError::Io {
        context: context.to_owned(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maildir(directory: &Path) {
        for subdir in ["cur", "new", "tmp"] {
            std::fs::create_dir_all(directory.join(subdir)).unwrap();
        }
    }

    fn message(folder: &Path, id: &str, info: &str) -> String {
        let name = if info.is_empty() {
            format!("{id}.host")
        } else {
            format!("{id}.host:2,{info}")
        };
        std::fs::write(
            folder.join("cur").join(&name),
            format!("Subject: {id}\r\n\r\nbody of {id}\r\n"),
        )
        .unwrap();
        name
    }

    #[test]
    fn a_directory_that_is_not_a_maildir_says_which_one_and_what_was_missing() {
        let tree = tempfile::tempdir().unwrap();
        std::fs::write(tree.path().join("a-letter.txt"), "not mail").unwrap();

        let refusal = LocalStore::open(tree.path()).unwrap_err();

        assert!(
            refusal.contains(&tree.path().display().to_string()),
            "the refusal must name the directory: {refusal}"
        );
        assert!(
            refusal.contains("cur/new/tmp"),
            "the refusal must say what was missing: {refusal}"
        );
        assert!(
            refusal.contains("will not guess"),
            "the refusal must say Postio is not going to try another format: {refusal}"
        );
    }

    #[test]
    fn a_file_is_not_a_maildir_either() {
        let tree = tempfile::tempdir().unwrap();
        let file = tree.path().join("mbox");
        std::fs::write(&file, "From ada@example.com\n").unwrap();

        let refusal = LocalStore::open(&file).unwrap_err();

        assert!(
            refusal.contains("is not a directory"),
            "a file must be refused as a file: {refusal}"
        );
    }

    #[test]
    fn nested_folders_are_named_by_their_path_with_the_inbox_first() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        maildir(&tree.path().join("Archives"));
        maildir(&tree.path().join("Archives").join("2026"));
        maildir(&tree.path().join("Sent"));

        let store = LocalStore::open(tree.path()).unwrap();
        let names: Vec<String> = store
            .folders()
            .unwrap()
            .into_iter()
            .map(|folder| folder.path)
            .collect();

        assert_eq!(names, ["INBOX", "Archives", "Archives/2026", "Sent"]);
    }

    #[test]
    fn a_dovecot_tree_reads_its_dotted_siblings_as_nested_folders() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        maildir(&tree.path().join(".Archives"));
        maildir(&tree.path().join(".Archives.2026"));

        let store = LocalStore::open(tree.path()).unwrap();
        let names: Vec<String> = store
            .folders()
            .unwrap()
            .into_iter()
            .map(|folder| folder.path)
            .collect();

        assert_eq!(names, ["INBOX", "Archives", "Archives/2026"]);
    }

    #[test]
    fn a_folder_that_is_not_there_is_reported_as_missing_not_as_empty() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        let store = LocalStore::open(tree.path()).unwrap();

        let error = store.folder("Drafts").unwrap_err();

        assert!(
            matches!(error, BackendError::NoSuchMailbox { ref path } if path == "Drafts"),
            "expected a missing mailbox, got {error}"
        );
    }

    #[test]
    fn every_message_gets_a_number_and_keeps_it_on_a_second_pass() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        message(tree.path(), "1000", "S");
        message(tree.path(), "1001", "");

        let store = LocalStore::open(tree.path()).unwrap();
        let inbox = store.folder("INBOX").unwrap();

        let first = store.snapshot(&inbox, 7).unwrap();
        assert_eq!(first.entries.len(), 2);
        assert_eq!(first.next, 3, "two messages consume UIDs 1 and 2");

        let store = LocalStore::open(tree.path()).unwrap();
        let again = store.snapshot(&store.folder("INBOX").unwrap(), 99).unwrap();

        assert_eq!(
            uids(&first),
            uids(&again),
            "the same messages must keep the same numbers"
        );
        assert_eq!(
            first.validity, again.validity,
            "a second pass must not renumber the UID space"
        );
    }

    #[test]
    fn reading_a_message_does_not_mint_it_a_second_number() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        message(tree.path(), "1000", "");

        let store = LocalStore::open(tree.path()).unwrap();
        let before = store.snapshot(&store.folder("INBOX").unwrap(), 7).unwrap();

        // Marking read renames the file, so a UID list keyed on the whole name would number it
        // twice.
        std::fs::rename(
            tree.path().join("cur").join("1000.host"),
            tree.path().join("cur").join("1000.host:2,S"),
        )
        .unwrap();

        let after = store.snapshot(&store.folder("INBOX").unwrap(), 7).unwrap();

        assert_eq!(uids(&before), uids(&after), "flags must not renumber mail");
        assert_eq!(after.entries[0].flags, [Flag::Seen]);
    }

    #[test]
    fn deleting_a_message_does_not_renumber_the_ones_that_are_left() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        message(tree.path(), "1000", "");
        message(tree.path(), "1001", "");
        message(tree.path(), "1002", "");

        let store = LocalStore::open(tree.path()).unwrap();
        let inbox = store.folder("INBOX").unwrap();
        let before = store.snapshot(&inbox, 7).unwrap();
        let third = before.entries[2].uid;

        std::fs::remove_file(tree.path().join("cur").join("1000.host")).unwrap();
        let after = store.snapshot(&inbox, 7).unwrap();

        assert_eq!(after.entries.len(), 2);
        assert_eq!(
            after.entries[1].uid, third,
            "a survivor keeps the number it had"
        );
        assert_eq!(
            after.next, before.next,
            "numbers are never handed out twice, even after a deletion"
        );

        message(tree.path(), "1003", "");
        let grown = store.snapshot(&inbox, 7).unwrap();
        assert_eq!(
            grown.entries.last().unwrap().uid,
            before.next,
            "new mail takes the next unused number, not a freed one"
        );
    }

    #[test]
    fn the_letters_in_a_filename_become_postio_flags() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        message(tree.path(), "1000", "DFRSTPX");

        let store = LocalStore::open(tree.path()).unwrap();
        let snapshot = store.snapshot(&store.folder("INBOX").unwrap(), 7).unwrap();

        let mut flags = snapshot.entries[0].flags.clone();
        flags.sort();
        let mut expected = vec![
            Flag::Seen,
            Flag::Answered,
            Flag::Flagged,
            Flag::Deleted,
            Flag::Draft,
        ];
        expected.sort();
        assert_eq!(flags, expected);
    }

    #[test]
    fn unread_mail_waiting_in_new_is_listed_too() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        std::fs::write(
            tree.path().join("new").join("1000.host"),
            "Subject: hi\r\n\r\n",
        )
        .unwrap();

        let store = LocalStore::open(tree.path()).unwrap();
        let snapshot = store.snapshot(&store.folder("INBOX").unwrap(), 7).unwrap();

        assert_eq!(snapshot.entries.len(), 1, "a delivery in new/ is mail");
        assert!(
            snapshot.entries[0].flags.is_empty(),
            "mail in new/ has not been read"
        );
    }

    #[test]
    fn a_message_reads_back_as_the_bytes_on_disk() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        message(tree.path(), "1000", "S");

        let store = LocalStore::open(tree.path()).unwrap();
        let snapshot = store.snapshot(&store.folder("INBOX").unwrap(), 7).unwrap();

        let bytes = store.read(&snapshot.entries[0].file).unwrap();

        assert_eq!(bytes, b"Subject: 1000\r\n\r\nbody of 1000\r\n");
    }

    #[test]
    fn a_message_that_has_gone_says_so_rather_than_reading_as_empty() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        let store = LocalStore::open(tree.path()).unwrap();

        let error = store
            .read(&tree.path().join("cur").join("gone.host"))
            .unwrap_err();

        assert!(
            matches!(error, BackendError::Io { .. }),
            "expected an I/O failure, got {error}"
        );
    }

    fn uids(snapshot: &Snapshot) -> Vec<u32> {
        snapshot.entries.iter().map(|entry| entry.uid).collect()
    }
}
