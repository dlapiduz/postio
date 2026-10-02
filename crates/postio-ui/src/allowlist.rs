//! Who may load remote images, kept across restarts.
//!
//! Remote images are blocked by default — `PRODUCT.md`'s "nothing leaves this
//! machine that the user did not ask for" starts at the tracking pixel — and
//! "show once" never touches this. This is the standing exception, and the
//! only place a decision to make one is recorded.
//!
//! # Why it is here rather than in a frontend
//!
//! `postio-gtk` has had one since `postio-xxz`, written against `glib`'s key
//! file and `$XDG_STATE_HOME`. Neither exists on macOS, and a second
//! implementation of a *privacy* rule is the one place ADR 0019 Q6's risk is
//! least acceptable: two allow lists means two answers to "may this sender
//! see me", and the wrong answer is silent and remote. So the rule and the
//! file format live here, and each frontend supplies a path.
//! [`RemoteImageAllowList`] is the same list under the name the desktop and
//! terminal frontends speak, at the path they have always used (#1273).
//!
//! # Addresses and domains
//!
//! Two grants, deliberately. "Always allow this address" is what a person
//! means about a correspondent; "always allow this domain" is what they mean
//! about a service that sends from `notices-3a7f@relay.example.net` and never
//! from the same address twice. A domain grant covers every address under it;
//! neither covers a subdomain of the other, because `mail.example.com` and
//! `example.com` can be different senders and guessing costs privacy.
//!
//! # A sender's treatment
//!
//! Beside the grants, [`RemoteImageAllowList`] keeps the treatment a person
//! chose to always see one sender's mail in (specs/007-postio-focus T213),
//! under a `[Treatment]` section of the same file. It is the same kind of
//! thing -- a standing answer about one sender's mail, a view preference
//! rather than mail data -- and [`AllowList::parse`] ignores the section, so
//! a frontend that does not offer treatments reads the grants unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use postio_body::treatment::Treatment;

/// Senders and domains whose remote images load without asking.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowList {
    addresses: BTreeSet<String>,
    domains: BTreeSet<String>,
}

impl AllowList {
    /// An empty list: nobody is allowed, which is the default state and the
    /// state any unreadable file falls back to.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a saved list. Anything unparseable reads as empty.
    ///
    /// **Fails closed.** A corrupt file means nobody is allow-listed, never
    /// "allow everything" and never an error that stops a message being read:
    /// the cost of the cautious reading is one extra click, and the cost of
    /// the other is a beacon nobody consented to.
    pub fn parse(text: &str) -> Self {
        let mut list = Self::new();
        let mut section = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line {
                "[addresses]" => section = Some(false),
                "[domains]" => section = Some(true),
                // Another section -- `[Treatment]`, which the shell beside
                // this keeps -- is nobody's grant, and its lines must not be
                // read as the previous section's.
                other if other.starts_with('[') && other.ends_with(']') => section = None,
                entry => match section {
                    Some(true) => list.allow_domain(entry),
                    Some(false) => list.allow(entry),
                    // A line before any section header belongs to nothing,
                    // and guessing which list it was meant for is guessing
                    // about consent.
                    None => {}
                },
            }
        }
        list
    }

    /// The file's text, ready to write.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# Senders whose remote images Postio loads without asking.\n\
             # Written by Postio; every line here was a deliberate choice.\n",
        );
        out.push_str("\n[addresses]\n");
        for address in &self.addresses {
            out.push_str(address);
            out.push('\n');
        }
        out.push_str("\n[domains]\n");
        for domain in &self.domains {
            out.push_str(domain);
            out.push('\n');
        }
        out
    }

    /// Read the list at `path`, or an empty one.
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Write the list to `path`, creating the directory if it is missing.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.render())
    }

    /// Whether `address` may load remote images — by its own grant or by one
    /// covering its domain.
    pub fn is_allowed(&self, address: &str) -> bool {
        let address = address.trim().to_lowercase();
        if address.is_empty() {
            return false;
        }
        if self.addresses.contains(&address) {
            return true;
        }
        domain_of(&address).is_some_and(|domain| self.domains.contains(&domain))
    }

    /// Always allow this address.
    pub fn allow(&mut self, address: &str) {
        let address = address.trim().to_lowercase();
        if !address.is_empty() {
            self.addresses.insert(address);
        }
    }

    /// Always allow every address at this domain.
    ///
    /// Takes an address or a bare domain, because the caller has an address
    /// and the menu item says "this domain".
    pub fn allow_domain(&mut self, address_or_domain: &str) {
        let text = address_or_domain.trim().to_lowercase();
        let domain = domain_of(&text).unwrap_or(text);
        if !domain.is_empty() {
            self.domains.insert(domain);
        }
    }

    /// Take a grant back.
    pub fn revoke(&mut self, address: &str) {
        let address = address.trim().to_lowercase();
        self.addresses.remove(&address);
        self.domains.remove(&address);
    }

    /// Every address allowed by name, for a settings pane to list.
    pub fn addresses(&self) -> impl Iterator<Item = &str> {
        self.addresses.iter().map(String::as_str)
    }

    /// Every domain allowed wholesale.
    pub fn domains(&self) -> impl Iterator<Item = &str> {
        self.domains.iter().map(String::as_str)
    }

    /// Whether anybody is allowed at all.
    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty() && self.domains.is_empty()
    }
}

/// The domain half of an address, lowercased.
fn domain_of(address: &str) -> Option<String> {
    address
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_lowercase())
        .filter(|domain| !domain.is_empty())
}

/// The key-file group the desktop app's old allow list kept its senders in.
const LEGACY_GROUP: &str = "AlwaysAllow";

/// The section a sender's chosen treatment lives under: the address is the
/// key, the treatment's attribute value (`app` or `paper`) the value.
const TREATMENT_SECTION: &str = "Treatment";

/// Senders whose remote images load without asking, across restarts, at the
/// path the desktop and terminal frontends share.
///
/// A thin shell over [`AllowList`], which is the one implementation every
/// frontend reads and writes (#1273). Two allow lists meant two answers to
/// "may this sender see me", and that is the least acceptable place for the
/// two to drift: the wrong answer is silent and remote. The shell stays
/// because the call sites in `postio-gtk` and `postio-tui` speak this
/// vocabulary -- `senders`, `save`, `path` -- and rewriting them to say the
/// same things differently would be churn without a reader.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteImageAllowList {
    inner: AllowList,
    /// The treatment a person chose to always see a sender's mail in
    /// (specs/007-postio-focus T213), by lowercased address.
    treatments: BTreeMap<String, Treatment>,
}

impl RemoteImageAllowList {
    /// Read the saved list, falling back to empty for anything missing or
    /// unreadable.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    /// As [`load`](Self::load), from a path you name.
    ///
    /// The shared format first, and the old `[AlwaysAllow]` key file only if
    /// that read nothing -- which is what a key file parses to, since
    /// [`AllowList::parse`] ignores every line before a section header it
    /// knows. A file in the old shape is migrated in place, once: there are
    /// no deployed installs to protect, but there
    /// is a maintainer with grants in a running build, and silently
    /// forgetting who they trusted would be the wrong kind of clean break.
    pub fn load_from(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let inner = AllowList::parse(&text);
        let treatments = treatments_in(&text);
        if !inner.is_empty() || text.trim().is_empty() {
            return Self { inner, treatments };
        }
        let Some(inner) = from_key_file(&text) else {
            return Self { inner, treatments };
        };
        let migrated = Self { inner, treatments };
        // Best-effort: the grants are in memory and correct either way, and
        // a write that fails only means the migration happens again next
        // launch.
        let _ = migrated.save_to(path);
        migrated
    }

    /// Whether `sender` has a standing "always allow" exception, by its own
    /// grant or by one covering its domain.
    ///
    /// `sender` is a bare address (`ada@example.com`), not a "Display Name
    /// `<addr>`" mailbox -- normalization here is only trimming and
    /// lowercasing, not address parsing.
    pub fn is_allowed(&self, sender: &str) -> bool {
        self.inner.is_allowed(sender)
    }

    /// Grant `sender` a standing exception, in memory only.
    ///
    /// Deliberately not persisted here: the frontend is what knows whether
    /// it is running against the real state directory or, in a test, a
    /// scratch path -- see [`save_to`](Self::save_to).
    pub fn allow(&mut self, sender: &str) {
        self.inner.allow(sender);
    }

    /// Whether any sender has a standing exception.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Every sender allowed by name, in address order -- what a settings
    /// pane lists to manage (#871).
    pub fn senders(&self) -> impl Iterator<Item = &str> {
        self.inner.addresses()
    }

    /// Revoke `sender`'s standing exception, in memory only -- the same
    /// split as [`allow`](Self::allow).
    pub fn revoke(&mut self, sender: &str) {
        self.inner.revoke(sender);
    }

    /// The treatment `sender`'s mail is always drawn in, if the person chose
    /// one ("Always for this sender", specs/007-postio-focus T213).
    pub fn treatment_for(&self, sender: &str) -> Option<Treatment> {
        self.treatments.get(&normalize(sender)).copied()
    }

    /// Remember `treatment` for `sender`, or forget their choice with
    /// `None`, in memory only -- [`save_to`](Self::save_to) persists it, as
    /// for [`allow`](Self::allow).
    pub fn set_treatment(&mut self, sender: &str, treatment: Option<Treatment>) {
        let sender = normalize(sender);
        if sender.is_empty() {
            return;
        }
        match treatment {
            Some(treatment) => {
                self.treatments.insert(sender, treatment);
            }
            None => {
                self.treatments.remove(&sender);
            }
        }
    }

    /// Persist to [`path`](Self::path).
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    /// As [`save`](Self::save), to a path you name.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut text = self.inner.render();
        if !self.treatments.is_empty() {
            text.push_str(&format!("\n[{TREATMENT_SECTION}]\n"));
            for (sender, treatment) in &self.treatments {
                text.push_str(&format!("{sender}={}\n", treatment.attribute_value()));
            }
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

/// An address as the lists key it: trimmed and lowercased.
fn normalize(address: &str) -> String {
    address.trim().to_lowercase()
}

/// The `[Treatment]` section of a saved list: each sender's chosen
/// treatment. A value no treatment answers to is no choice, so the rule
/// decides for that sender as if nothing were written.
fn treatments_in(text: &str) -> BTreeMap<String, Treatment> {
    let mut in_section = false;
    let mut treatments = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_section = name == TREATMENT_SECTION;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && let Some(treatment) = Treatment::from_attribute(value.trim())
        {
            let key = normalize(key);
            if !key.is_empty() {
                treatments.insert(key, treatment);
            }
        }
    }
    treatments
}

/// The desktop app's old `[AlwaysAllow]` key file, if that is what `text` is.
///
/// Read without GLib: for one group of boolean keys the format is a few
/// lines of text, and reading it here is what lets a frontend with no GLib
/// keep the grants the desktop app made.
fn from_key_file(text: &str) -> Option<AllowList> {
    let mut in_group = false;
    let mut list = AllowList::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(group) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_group = group == LEGACY_GROUP;
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && value.trim() == "true"
        {
            list.allow(key.trim());
        }
    }
    (!list.is_empty()).then_some(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_is_allowed_to_begin_with() {
        // The default that matters: a fresh install shows no remote image
        // until somebody says so.
        assert!(!AllowList::new().is_allowed("ada@example.com"));
    }

    #[test]
    fn an_allowed_address_is_matched_however_it_is_capitalised() {
        // `Ada@Example.com` and `ada@example.com` are one sender, and a
        // grant that missed the second would silently keep blocking.
        let mut list = AllowList::new();
        list.allow("Ada@Example.com");
        assert!(list.is_allowed("ada@example.com"));
        assert!(list.is_allowed("  ADA@EXAMPLE.COM "));
    }

    #[test]
    fn a_domain_grant_covers_the_addresses_under_it() {
        // What the second menu item is for: a service that sends from
        // `notices-3a7f@relay.example.net` and never twice from the same
        // address cannot be allowed one address at a time.
        let mut list = AllowList::new();
        list.allow_domain("notices-3a7f@relay.example.net");
        assert!(list.is_allowed("notices-9c21@relay.example.net"));
        assert!(
            !list.is_allowed("someone@example.net"),
            "and not its parent"
        );
    }

    #[test]
    fn a_domain_grant_does_not_reach_into_subdomains() {
        // `mail.example.com` and `example.com` can be different senders, and
        // guessing costs privacy rather than convenience.
        let mut list = AllowList::new();
        list.allow_domain("example.com");
        assert!(!list.is_allowed("someone@mail.example.com"));
    }

    #[test]
    fn an_address_grant_does_not_leak_to_the_rest_of_the_domain() {
        let mut list = AllowList::new();
        list.allow("ada@example.com");
        assert!(!list.is_allowed("someone-else@example.com"));
    }

    #[test]
    fn a_grant_survives_being_written_and_read_back() {
        let mut list = AllowList::new();
        list.allow("ada@example.com");
        list.allow_domain("relay.example.net");

        let read = AllowList::parse(&list.render());

        assert_eq!(read, list);
        assert!(read.is_allowed("ada@example.com"));
        assert!(read.is_allowed("anything@relay.example.net"));
    }

    #[test]
    fn a_corrupt_file_allows_nobody_rather_than_everybody() {
        // Fails closed: one extra click against a beacon nobody consented to.
        let list = AllowList::parse("this is not the file you are looking for\n\0\u{1}");
        assert!(list.is_empty());
        assert!(!list.is_allowed("ada@example.com"));
    }

    #[test]
    fn a_line_before_any_heading_belongs_to_nothing() {
        // Guessing which list a stray line was meant for is guessing about
        // consent, and the guess that costs is the generous one.
        let list = AllowList::parse("ada@example.com\n\n[domains]\nrelay.example.net\n");
        assert!(!list.is_allowed("ada@example.com"));
        assert!(list.is_allowed("someone@relay.example.net"));
    }

    #[test]
    fn taking_a_grant_back_takes_it_back() {
        let mut list = AllowList::new();
        list.allow("ada@example.com");
        list.revoke("ada@example.com");
        assert!(!list.is_allowed("ada@example.com"));
    }

    #[test]
    fn a_missing_file_is_an_empty_list_rather_than_an_error() {
        let list = AllowList::load_from(Path::new("/nowhere/at/all/allowed-senders.toml"));
        assert!(list.is_empty());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("postio-allowlist-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("remote-images.ini")
    }

    #[test]
    fn the_key_file_the_desktop_app_wrote_is_read_and_migrated() {
        // What GLib's key file wrote before the list was shared: nobody
        // allowed then is forgotten now, and a sender explicitly *not*
        // allowed does not arrive allowed.
        let path = scratch("glib-written");
        std::fs::write(
            &path,
            "[AlwaysAllow]\nada@example.com=true\nbea@example.com=false\n",
        )
        .unwrap();

        let list = RemoteImageAllowList::load_from(&path);

        assert!(list.is_allowed("ada@example.com"));
        assert!(!list.is_allowed("bea@example.com"));
        // Written back in the shared shape, so it happens once rather than
        // every launch.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[addresses]"), "not rewritten: {text:?}");
        assert!(RemoteImageAllowList::load_from(&path).is_allowed("ada@example.com"));
    }

    #[test]
    fn a_list_already_in_the_shared_format_is_left_alone() {
        let path = scratch("already-shared");
        let mut list = RemoteImageAllowList::default();
        list.allow("ada@example.com");
        list.save_to(&path).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        assert!(RemoteImageAllowList::load_from(&path).is_allowed("ada@example.com"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn an_allowed_sender_survives_a_round_trip_through_the_shell() {
        let path = scratch("round-trip");
        let mut list = RemoteImageAllowList::default();
        list.allow("ADA@example.com");
        list.save_to(&path).expect("the allow list should write");

        let reloaded = RemoteImageAllowList::load_from(&path);
        assert!(reloaded.is_allowed("  Ada@Example.com  "));
        assert!(!reloaded.is_allowed("tracker@shop.example.org"));
        assert_eq!(
            reloaded.senders().collect::<Vec<_>>(),
            vec!["ada@example.com"]
        );
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
    fn a_senders_treatment_survives_a_round_trip_beside_their_images() {
        let path = scratch("treatment-round-trip");
        let mut list = RemoteImageAllowList::default();
        list.allow("ada@example.com");
        list.set_treatment(" News@Example.com ", Some(Treatment::Paper));
        list.set_treatment("bea@example.org", Some(Treatment::AppColours));
        list.save_to(&path).unwrap();

        let reloaded = RemoteImageAllowList::load_from(&path);
        assert!(
            reloaded.is_allowed("ada@example.com"),
            "the images were lost"
        );
        assert_eq!(
            reloaded.treatment_for("news@example.com"),
            Some(Treatment::Paper)
        );
        assert_eq!(
            reloaded.treatment_for("BEA@example.org"),
            Some(Treatment::AppColours)
        );
        assert_eq!(reloaded.treatment_for("ada@example.com"), None);
        assert!(
            !reloaded.is_allowed("news@example.com"),
            "choosing paper allowed the sender's images"
        );
    }

    #[test]
    fn forgetting_a_senders_treatment_leaves_the_rule_to_decide() {
        let path = scratch("treatment-forget");
        let mut list = RemoteImageAllowList::default();
        list.set_treatment("news@example.com", Some(Treatment::Paper));
        list.set_treatment("news@example.com", None);
        list.save_to(&path).unwrap();
        assert_eq!(
            RemoteImageAllowList::load_from(&path).treatment_for("news@example.com"),
            None
        );
    }

    #[test]
    fn an_unknown_treatment_in_the_file_is_no_choice() {
        let path = scratch("treatment-unknown");
        std::fs::write(&path, "[Treatment]\nnews@example.com=sepia\n").unwrap();
        assert_eq!(
            RemoteImageAllowList::load_from(&path).treatment_for("news@example.com"),
            None
        );
    }

    #[test]
    fn a_treatment_is_nobodys_grant() {
        // The section follows `[domains]` in the file, and a reader that did
        // not know it would take `news@example.com=paper` for a domain.
        let list = AllowList::parse(
            "[addresses]\nada@example.com\n\n[domains]\nrelay.example.net\n\n\
             [Treatment]\nnews@example.com=paper\n",
        );
        assert!(list.is_allowed("ada@example.com"));
        assert!(!list.is_allowed("news@example.com"));
        assert_eq!(list.domains().count(), 1);
    }
}
