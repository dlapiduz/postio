//! The remote-image allow list: `postio-xxz`.
//!
//! Remote images are blocked by default — see [`postio_body::sanitize::RemoteImages`]
//! — and stay blocked for a message even after "show once", which never
//! touches this file. This is only the standing exception: "always allow
//! images from this sender", kept across restarts.
//!
//! A plain key file under `$XDG_STATE_HOME`, best-effort throughout. A missing or corrupt file
//! means nobody is allow-listed yet, never a failure to open the reader.
//! Allow-listing is view preference, not mail data — it does not belong in
//! `postio-core`'s database any more than a dragged divider does.

//!
//! Moved from `postio-gtk` so every frontend honours the same standing
//! exceptions (`specs/005-tui-frontend` T044). It reads and writes the file
//! the desktop app always has -- the GLib key-file format, which for one
//! group of boolean keys is a few lines of text -- so nothing already allowed
//! is forgotten.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The key-file group every allowed sender's key lives under.
///
/// Presence of a key is the signal — the sender's (lowercased) address is
/// the key itself, so the file self-documents who is allow-listed without
/// needing a list-valued key `glib::KeyFile` has no setter for.
const GROUP: &str = "AlwaysAllow";

/// Senders whose remote images load without asking, across restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteImageAllowList {
    senders: BTreeSet<String>,
}

impl RemoteImageAllowList {
    /// Read the saved list, falling back to empty for anything missing or
    /// unreadable.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    /// As [`load`](Self::load), from a path you name.
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let mut in_group = false;
        let mut senders = BTreeSet::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(group) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                in_group = group == GROUP;
                continue;
            }
            if !in_group {
                continue;
            }
            if let Some((key, value)) = line.split_once('=')
                && value.trim() == "true"
            {
                senders.insert(key.trim().to_owned());
            }
        }
        RemoteImageAllowList { senders }
    }

    /// Whether `sender` has a standing "always allow" exception.
    ///
    /// `sender` is a bare address (`ada@example.com`), not a "Display Name
    /// `<addr>`" mailbox — normalization here is only trimming and
    /// lowercasing, not address parsing.
    pub fn is_allowed(&self, sender: &str) -> bool {
        self.senders.contains(&normalize(sender))
    }

    /// Grant `sender` a standing exception, in memory only.
    ///
    /// Deliberately not persisted here: the frontend's reader is what
    /// knows whether it is running against the real
    /// `$XDG_STATE_HOME/postio/remote-images.ini` or, in a test, a scratch
    /// path — see [`save_to`](Self::save_to). Call that (or `save`
    /// (Self::save)) once the mutation is one the caller wants to keep.
    pub fn allow(&mut self, sender: &str) {
        self.senders.insert(normalize(sender));
    }

    /// Whether any sender has a standing exception. Cheap enough for a
    /// caller to skip building a menu entry when there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.senders.is_empty()
    }

    /// Every sender with a standing exception, address order (`BTreeSet`
    /// already keeps it sorted) — what a settings pane lists to manage
    /// (#871).
    pub fn senders(&self) -> impl Iterator<Item = &str> {
        self.senders.iter().map(String::as_str)
    }

    /// Revokes `sender`'s standing exception, in memory only — same split
    /// as [`allow`](Self::allow): the caller decides whether and when to
    /// persist with [`save`](Self::save)/[`save_to`](Self::save_to).
    pub fn revoke(&mut self, sender: &str) {
        self.senders.remove(&normalize(sender));
    }

    /// Persist to `$XDG_STATE_HOME/postio/remote-images.ini`.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    /// As [`save`](Self::save), to a path you name — what the tests use, and
    /// what a caller uses when it is not writing to the real state
    /// directory.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut text = format!("[{GROUP}]\n");
        for sender in &self.senders {
            text.push_str(&format!("{sender}=true\n"));
        }
        std::fs::write(path, text)
    }

    /// `$XDG_STATE_HOME/postio/remote-images.ini`.
    ///
    /// `$XDG_STATE_HOME`, else `~/.local/state`: where GLib's
    /// `user_state_dir` puts it, so the desktop app finds the file it wrote.
    pub fn path() -> PathBuf {
        let state = std::env::var_os("XDG_STATE_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local").join("state"))
            })
            .unwrap_or_else(|| PathBuf::from(".local/state"));
        state.join("postio").join("remote-images.ini")
    }
}

/// Case-insensitive, trimmed — `Ada@Example.Com` and `ada@example.com ` name
/// the same standing exception.
fn normalize(sender: &str) -> String {
    sender.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("postio-allowlist-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("remote-images.ini")
    }

    #[test]
    fn the_file_the_desktop_app_wrote_is_read() {
        // What GLib's key file wrote before this moved: nobody allowed then
        // is forgotten now.
        let path = scratch("glib-written");
        std::fs::write(
            &path,
            "[AlwaysAllow]\nada@example.com=true\nbea@example.com=false\n",
        )
        .unwrap();
        let list = RemoteImageAllowList::load_from(&path);
        assert!(list.is_allowed("ada@example.com"));
        assert!(!list.is_allowed("bea@example.com"));
    }

    #[test]
    fn a_missing_file_allows_nobody() {
        let path = scratch("missing").with_file_name("nothing-here.ini");
        assert!(!RemoteImageAllowList::load_from(&path).is_allowed("ada@example.com"));
    }

    #[test]
    fn an_allowed_sender_survives_a_round_trip() {
        let path = scratch("round-trip");
        let mut list = RemoteImageAllowList::default();
        // A bare address, matching what `allow()` expects: this module does
        // not parse a "Display Name <addr>" form, that is the caller's job
        // (`postio_model::EmailAddress::address` before it ever gets here).
        list.senders.insert(normalize("ADA@example.com"));
        list.save_to(&path).expect("the allow list should write");

        let reloaded = RemoteImageAllowList::load_from(&path);
        assert!(reloaded.is_allowed("ada@example.com"));
        assert!(reloaded.is_allowed("  Ada@Example.com  "));
    }

    #[test]
    fn allowing_a_sender_is_case_and_whitespace_insensitive() {
        let path = scratch("normalize");
        let mut list = RemoteImageAllowList::default();
        list.allow(" Ada@Example.com ");
        list.save_to(&path).unwrap();
        assert!(RemoteImageAllowList::load_from(&path).is_allowed("ada@example.com"));
    }

    #[test]
    fn a_sender_not_on_the_list_is_still_blocked() {
        let path = scratch("other-sender");
        let mut list = RemoteImageAllowList::default();
        list.allow("ada@example.com");
        list.save_to(&path).unwrap();

        let reloaded = RemoteImageAllowList::load_from(&path);
        assert!(!reloaded.is_allowed("tracker@shop.example.org"));
    }

    #[test]
    fn allow_does_not_touch_disk_on_its_own() {
        let path = scratch("not-persisted");
        let mut list = RemoteImageAllowList::default();
        list.allow("ada@example.com");
        assert!(list.is_allowed("ada@example.com"), "in memory, immediately");
        assert!(
            !path.exists(),
            "allow() must not write until save/save_to is called"
        );
    }

    #[test]
    fn a_corrupt_file_falls_back_to_empty_rather_than_failing() {
        let path = scratch("corrupt");
        std::fs::write(&path, b"this is not a key file at all\x00\x01").unwrap();
        assert!(RemoteImageAllowList::load_from(&path).is_empty());
    }

    #[test]
    fn the_list_lives_beside_the_other_state_not_in_the_config() {
        let path = RemoteImageAllowList::path();
        assert!(
            path.ends_with("postio/remote-images.ini"),
            "{}",
            path.display()
        );
        assert!(!path.to_string_lossy().contains("/.config/"));
    }

    #[test]
    fn senders_lists_everyone_allowed_in_sorted_order() {
        let mut list = RemoteImageAllowList::default();
        list.allow("zed@example.com");
        list.allow("ada@example.com");
        assert_eq!(
            list.senders().collect::<Vec<_>>(),
            vec!["ada@example.com", "zed@example.com"]
        );
    }

    #[test]
    fn revoking_a_sender_removes_the_exception() {
        let mut list = RemoteImageAllowList::default();
        list.allow("ada@example.com");
        list.revoke("ada@example.com");
        assert!(!list.is_allowed("ada@example.com"));
        assert!(list.is_empty());
    }

    #[test]
    fn revoking_a_sender_not_on_the_list_is_a_no_op() {
        let mut list = RemoteImageAllowList::default();
        list.allow("ada@example.com");
        list.revoke("nobody@example.com");
        assert!(list.is_allowed("ada@example.com"));
    }
}
