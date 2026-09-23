//! The remote-image allow list: `postio-xxz`.
//!
//! Remote images are blocked by default — see [`super::sanitize::RemoteImages`]
//! — and stay blocked for a message even after "show once", which never
//! touches this file. This is only the standing exception: "always allow
//! images from this sender", kept across restarts.
//!
//! Same shape as `state.rs`'s `WindowState`, on purpose: a plain key file
//! under `$XDG_STATE_HOME`, best-effort throughout. A missing or corrupt file
//! means nobody is allow-listed yet, never a failure to open the reader.
//! Allow-listing is view preference, not mail data — it does not belong in
//! `postio-core`'s database any more than a dragged divider does.

use std::path::{Path, PathBuf};

use gtk::glib;
use postio_ui::allowlist::AllowList;

/// The key-file group every allowed sender's key lives under.
///
/// Presence of a key is the signal — the sender's (lowercased) address is
/// the key itself, so the file self-documents who is allow-listed without
/// needing a list-valued key `glib::KeyFile` has no setter for.
const GROUP: &str = "AlwaysAllow";

/// Senders whose remote images load without asking, across restarts.
///
/// A thin shell over [`postio_ui::allowlist::AllowList`], which is the one
/// implementation both frontends read and write (#1273). Two allow lists
/// meant two answers to "may this sender see me", and that is the least
/// acceptable place for the two to drift: the wrong answer is silent and
/// remote.
///
/// The shell stays because the thirty-odd call sites in this crate speak
/// this vocabulary — `senders`, `save`, a `glib::Error` — and rewriting them
/// to say the same things differently would be churn without a reader.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteImageAllowList {
    inner: AllowList,
}

impl RemoteImageAllowList {
    /// Read the saved list, falling back to empty for anything missing or
    /// unreadable.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    /// As [`load`](Self::load), from a path you name.
    ///
    /// The shared format first, and the old key file only if that read
    /// nothing — which is what an `[AlwaysAllow]` file parses to, since
    /// `AllowList::parse` ignores every line before a section header it
    /// knows. A file in the old shape is migrated in place, once, and said
    /// so in the log: there are no deployed installs to protect, but there
    /// is a maintainer with grants in a running build, and silently
    /// forgetting who they trusted would be the wrong kind of clean break.
    pub fn load_from(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let inner = AllowList::parse(&text);
        if !inner.is_empty() || text.trim().is_empty() {
            return Self { inner };
        }
        let Some(inner) = Self::from_key_file(path) else {
            return Self { inner };
        };
        let migrated = Self { inner };
        match migrated.save_to(path) {
            Ok(()) => tracing::info!(
                path = %path.display(),
                senders = migrated.inner.addresses().count(),
                "migrated the remote-image allow list to the shared format"
            ),
            // The grants are in memory and correct either way; what is lost
            // is only that the migration has to happen again next launch.
            Err(error) => tracing::warn!(%error, "the migrated allow list could not be written"),
        }
        migrated
    }

    /// The old `[AlwaysAllow]` key file, if that is what is there.
    fn from_key_file(path: &Path) -> Option<AllowList> {
        let key_file = glib::KeyFile::new();
        key_file
            .load_from_file(path, glib::KeyFileFlags::NONE)
            .ok()?;
        let keys = key_file.keys(GROUP).ok()?;
        let mut list = AllowList::new();
        for key in keys.iter() {
            if key_file.boolean(GROUP, key).unwrap_or(false) {
                list.allow(key);
            }
        }
        (!list.is_empty()).then_some(list)
    }

    /// Whether `sender` has a standing "always allow" exception.
    ///
    /// `sender` is a bare address (`ada@example.com`), not a "Display Name
    /// `<addr>`" mailbox — normalization here is only trimming and
    /// lowercasing, not address parsing.
    pub fn is_allowed(&self, sender: &str) -> bool {
        self.inner.is_allowed(sender)
    }

    /// Grant `sender` a standing exception, in memory only.
    ///
    /// Deliberately not persisted here: [`super::view::Reader`] is what
    /// knows whether it is running against the real
    /// `$XDG_STATE_HOME/postio/remote-images.ini` or, in a test, a scratch
    /// path — see [`save_to`](Self::save_to). Call that (or `save`
    /// (Self::save)) once the mutation is one the caller wants to keep.
    pub fn allow(&mut self, sender: &str) {
        self.inner.allow(sender);
    }

    /// Whether any sender has a standing exception. Cheap enough for a
    /// caller to skip building a menu entry when there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Every sender with a standing exception, address order (`BTreeSet`
    /// already keeps it sorted) — what a settings pane lists to manage
    /// (#871).
    pub fn senders(&self) -> impl Iterator<Item = &str> {
        self.inner.addresses()
    }

    /// Revokes `sender`'s standing exception, in memory only — same split
    /// as [`allow`](Self::allow): the caller decides whether and when to
    /// persist with [`save`](Self::save)/[`save_to`](Self::save_to).
    pub fn revoke(&mut self, sender: &str) {
        self.inner.revoke(sender);
    }

    /// Persist to `$XDG_STATE_HOME/postio/remote-images.ini`.
    pub fn save(&self) -> Result<(), glib::Error> {
        self.save_to(&Self::path())
    }

    /// As [`save`](Self::save), to a path you name — what the tests use, and
    /// what a caller uses when it is not writing to the real state
    /// directory.
    pub fn save_to(&self, path: &Path) -> Result<(), glib::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                glib::Error::new(
                    glib::FileError::Failed,
                    &format!("cannot create {}: {error}", parent.display()),
                )
            })?;
        }
        self.inner.save_to(path).map_err(|error| {
            glib::Error::new(
                glib::FileError::Failed,
                &format!("cannot write {}: {error}", path.display()),
            )
        })
    }

    /// `$XDG_STATE_HOME/postio/remote-images.ini`.
    pub fn path() -> PathBuf {
        glib::user_state_dir()
            .join("postio")
            .join("remote-images.ini")
    }
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
        list.allow("ADA@example.com");
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
    fn grants_in_the_old_key_file_survive_the_move_to_the_shared_format() {
        // There are no deployed installs to protect, and this is still worth
        // writing: the maintainer has grants in a running build, and
        // silently forgetting who they trusted is the wrong kind of clean
        // break — the failure is invisible until a sender they had allowed
        // is quietly blocked again.
        let path = scratch("migrate");
        let key_file = glib::KeyFile::new();
        key_file.set_boolean(GROUP, "ada@example.com", true);
        key_file.set_boolean(GROUP, "grace@example.net", true);
        // A sender who was explicitly *not* allowed must not arrive allowed.
        key_file.set_boolean(GROUP, "alan@example.org", false);
        key_file.save_to_file(&path).unwrap();

        let list = RemoteImageAllowList::load_from(&path);

        assert!(list.is_allowed("ada@example.com"));
        assert!(list.is_allowed("grace@example.net"));
        assert!(!list.is_allowed("alan@example.org"));

        // Written back in the new shape, so it happens once rather than
        // every launch — and reads back the same.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[addresses]"), "not rewritten: {text:?}");
        assert!(RemoteImageAllowList::load_from(&path).is_allowed("ada@example.com"));
    }

    #[test]
    fn a_list_already_in_the_shared_format_is_left_alone() {
        // The migration must not fire on every load, and must never treat a
        // deliberately empty list as an old file to convert.
        let path = scratch("already-shared");
        let mut list = RemoteImageAllowList::default();
        list.allow("ada@example.com");
        list.save_to(&path).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let read_back = RemoteImageAllowList::load_from(&path);

        assert!(read_back.is_allowed("ada@example.com"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
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
