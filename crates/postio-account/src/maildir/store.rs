//! Reading a maildir: its folders, its messages, and their numbers.
//!
//! Everything the filesystem knows, turned into the shapes the rest of Postio
//! speaks — with [`UidList`](super::UidList) supplying the identity a maildir
//! does not have. Nothing here decides policy: which messages to fetch, when,
//! and what to do with them stays the sync machinery's, unchanged.

use std::path::{Path, PathBuf};

use io_maildir::client::MaildirClient;
use io_maildir::flag::{MaildirFlag, MaildirFlags};
use io_maildir::maildir::Maildir;
use postio_model::Flag;

use super::UidList;
use crate::backend::BackendError;

/// One folder of a maildir, as Postio names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// The path a caller selects by — `INBOX`, `Archives/2026`.
    pub path: String,
    /// Where it is on disk.
    pub directory: PathBuf,
}

/// One message in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its number, stable across runs — see [`UidList`](super::UidList).
    pub uid: u32,
    /// The file, so reading a body does not mean listing the folder again.
    pub file: PathBuf,
    /// The flags in its name, translated to Postio's.
    pub flags: Vec<Flag>,
}

/// What a folder looks like right now: its messages and its numbers.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The messages, lowest number first.
    pub entries: Vec<Entry>,
    /// This folder's `UIDVALIDITY`.
    pub validity: u32,
    /// The next number that will be handed out.
    pub next: u32,
}

/// A maildir tree on this machine.
#[derive(Debug, Clone)]
pub struct LocalStore {
    root: PathBuf,
    maildirpp: bool,
}

impl LocalStore {
    /// A store rooted at `root`, in whichever layout it turns out to be.
    ///
    /// **Stated, not guessed**: the layout is read off the tree once, here,
    /// rather than each caller assuming one. A root with dot-prefixed
    /// children beside its own `cur/new/tmp` is Maildir++ (Dovecot, `~/Mail`
    /// as one flat directory); anything else is the fs layout, where folders
    /// are ordinary nested directories.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        Self::looks_like_a_maildir(&root)?;
        let maildirpp = root.join("cur").is_dir() && has_dotted_child(&root);
        Ok(Self { root, maildirpp })
    }

    /// Where this store is.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether folders are dot-prefixed siblings rather than nested
    /// directories.
    pub fn is_maildirpp(&self) -> bool {
        self.maildirpp
    }

    /// Whether `root` is a maildir at all, and what is wrong with it if not.
    ///
    /// The message is the one a person reads in the Add Account sheet, so it
    /// says which directory and what was missing from it — a `~/Downloads`
    /// picked by mistake must not present itself as an account with no mail.
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

    /// The folders, `INBOX` first.
    pub fn folders(&self) -> Result<Vec<Folder>, BackendError> {
        let listed = self
            .client()
            .list_maildirs()
            .map_err(|error| unreadable("listing the maildir's folders", error))?;

        let mut folders: Vec<Folder> = listed
            .into_iter()
            .filter_map(|maildir| {
                let directory = PathBuf::from(maildir.path().to_string());
                let path = self.name_of(&directory)?;
                Some(Folder { path, directory })
            })
            .collect();
        folders.sort_by(|left, right| {
            // Inbox first, then alphabetical — the order every other client
            // shows; `postio_ui::sidebar` arranges the tree from here.
            (left.path != "INBOX", &left.path).cmp(&(right.path != "INBOX", &right.path))
        });
        folders.dedup_by(|left, right| left.path == right.path);
        Ok(folders)
    }

    /// The folder at `path`, if the store has one.
    pub fn folder(&self, path: &str) -> Result<Folder, BackendError> {
        self.folders()?
            .into_iter()
            .find(|folder| folder.path == path)
            .ok_or_else(|| BackendError::NoSuchMailbox {
                path: path.to_owned(),
            })
    }

    /// Everything in `folder`, numbered.
    ///
    /// Reads the folder's UID list, numbers anything new, forgets the names
    /// that have gone and writes it back — so the numbers this answers with
    /// are the numbers the next run will answer with, which is the whole
    /// reason a second pass does not import the mailbox again.
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

        // Best effort, deliberately: a folder Postio cannot write to is still
        // one it can read, and failing to save costs numbers that move on the
        // next run — not mail that cannot be opened now.
        if let Err(error) = uids.save(&folder.directory) {
            tracing::warn!(%error, "a maildir folder's UID list could not be saved");
        }

        Ok(Snapshot {
            entries,
            validity: uids.validity(),
            next: uids.next(),
        })
    }

    /// One message's bytes.
    pub fn read(&self, file: &Path) -> Result<Vec<u8>, BackendError> {
        std::fs::read(file).map_err(|error| BackendError::Io {
            context: "reading a message out of the maildir".to_owned(),
            reason: error.to_string(),
        })
    }

    fn client(&self) -> MaildirClient {
        let mut client = MaildirClient::new(self.root.display().to_string().as_str());
        client.store.maildirpp = self.maildirpp;
        client
    }

    /// What Postio calls the folder living at `directory`.
    ///
    /// The root is `INBOX` whichever layout this is: in Maildir++ that is what
    /// the root *is*, and in the fs layout a root holding mail of its own is
    /// the inbox of the account somebody just pointed at it.
    fn name_of(&self, directory: &Path) -> Option<String> {
        let relative = directory.strip_prefix(&self.root).ok()?;
        let mut parts: Vec<String> = Vec::new();
        for part in relative.components() {
            let part = part.as_os_str().to_str()?;
            let part = part.strip_prefix('.').unwrap_or(part);
            // Maildir++ spells nesting with dots inside one flat name.
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

/// Maildir's info letters as Postio's flags.
///
/// `Trashed` is `\Deleted`; `Passed` and custom keywords become nothing,
/// because Postio's flag set is IMAP's and neither has an IMAP meaning to
/// give them. Dropping them here is lossless in the direction that matters —
/// they stay in the filename, which is where the maildir keeps them.
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

    /// Builds `cur`, `new` and `tmp` under `directory`.
    fn maildir(directory: &Path) {
        for subdir in ["cur", "new", "tmp"] {
            std::fs::create_dir_all(directory.join(subdir)).unwrap();
        }
    }

    /// Drops a message into a folder's `cur`, returning its filename.
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
        assert!(
            !store.is_maildirpp(),
            "nested directories are the fs layout"
        );
    }

    #[test]
    fn a_dovecot_tree_reads_its_dotted_siblings_as_nested_folders() {
        let tree = tempfile::tempdir().unwrap();
        maildir(tree.path());
        maildir(&tree.path().join(".Archives"));
        maildir(&tree.path().join(".Archives.2026"));

        let store = LocalStore::open(tree.path()).unwrap();
        assert!(store.is_maildirpp(), "dotted siblings are Maildir++");

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

        // A second run, through a store opened from scratch: this is the
        // acceptance criterion — running it twice must not duplicate mail,
        // and it cannot duplicate mail if the numbers do not move.
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

        // Marking a message read renames the file — the flags live in its
        // name — so a UID list keyed on the whole name would hand this
        // message a second number and the mailbox would show it twice.
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
        // `P` (passed) and `X` (an opaque letter another client wrote) have
        // no IMAP counterpart, and must not become one.
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
