//! Persistent remote-image allow list: the one implementation of this privacy rule, shared
//! by every frontend (ADR 0019 Q6, #1273). Grants are per address or per domain; neither
//! covers a subdomain of the other. It also keeps a sender's chosen treatment.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use postio_body::treatment::Treatment;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowList {
    addresses: BTreeSet<String>,
    domains: BTreeSet<String>,
}

impl AllowList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a saved list. Fails closed: anything unparseable reads as empty, never as
    /// "allow everything".
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
                // Another section's lines must not be read as the previous section's grants.
                other if other.starts_with('[') && other.ends_with(']') => section = None,
                entry => match section {
                    Some(true) => list.allow_domain(entry),
                    Some(false) => list.allow(entry),
                    // A line before any section belongs to nothing; guessing is guessing about
                    // consent.
                    None => {}
                },
            }
        }
        list
    }

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

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.render())
    }

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

    pub fn allow(&mut self, address: &str) {
        let address = address.trim().to_lowercase();
        if !address.is_empty() {
            self.addresses.insert(address);
        }
    }

    pub fn allow_domain(&mut self, address_or_domain: &str) {
        let text = address_or_domain.trim().to_lowercase();
        let domain = domain_of(&text).unwrap_or(text);
        if !domain.is_empty() {
            self.domains.insert(domain);
        }
    }

    pub fn revoke(&mut self, address: &str) {
        let address = address.trim().to_lowercase();
        self.addresses.remove(&address);
        self.domains.remove(&address);
    }

    pub fn addresses(&self) -> impl Iterator<Item = &str> {
        self.addresses.iter().map(String::as_str)
    }

    pub fn domains(&self) -> impl Iterator<Item = &str> {
        self.domains.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty() && self.domains.is_empty()
    }
}

fn domain_of(address: &str) -> Option<String> {
    address
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_lowercase())
        .filter(|domain| !domain.is_empty())
}

const LEGACY_GROUP: &str = "AlwaysAllow";

/// Section for a sender's chosen treatment: address key, `app` or `paper` value.
const TREATMENT_SECTION: &str = "Treatment";

/// The desktop and terminal frontends' vocabulary (`senders`, `save`, `path`) over
/// `AllowList`, the one implementation (#1273).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteImageAllowList {
    inner: AllowList,
    treatments: BTreeMap<String, Treatment>,
}

impl RemoteImageAllowList {
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    /// Reads the shared format, then the old `[AlwaysAllow]` key file if that read nothing;
    /// an old-shape file is migrated in place.
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
        // Best-effort: the grants are in memory either way; a failed write only repeats the
        // migration next launch.
        let _ = migrated.save_to(path);
        migrated
    }

    /// `sender` is a bare address, not a "Name <addr>" mailbox; it is only trimmed and lowercased.
    pub fn is_allowed(&self, sender: &str) -> bool {
        self.inner.is_allowed(sender)
    }

    /// In memory only: the frontend decides where it is saved.
    pub fn allow(&mut self, sender: &str) {
        self.inner.allow(sender);
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn grants(&self) -> &AllowList {
        &self.inner
    }

    /// Mutable grants; saving keeps the treatments beside them.
    pub fn grants_mut(&mut self) -> &mut AllowList {
        &mut self.inner
    }

    pub fn senders(&self) -> impl Iterator<Item = &str> {
        self.inner.addresses()
    }

    pub fn revoke(&mut self, sender: &str) {
        self.inner.revoke(sender);
    }

    pub fn treatment_for(&self, sender: &str) -> Option<Treatment> {
        self.treatments.get(&normalize(sender)).copied()
    }

    /// `None` forgets the choice. In memory only.
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

    pub fn treatments(&self) -> impl Iterator<Item = (&str, Treatment)> {
        self.treatments
            .iter()
            .map(|(sender, treatment)| (sender.as_str(), *treatment))
    }

    pub fn clear_treatments(&mut self) {
        self.treatments.clear();
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

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

    /// `$XDG_STATE_HOME/postio/remote-images.ini`, falling back to `~/.local/state` as GLib does.
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

fn normalize(address: &str) -> String {
    address.trim().to_lowercase()
}

/// The `[Treatment]` section; a value no treatment answers to is no choice.
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

/// The old `[AlwaysAllow]` key file, read without GLib so a frontend without it keeps those grants.
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
        assert!(!AllowList::new().is_allowed("ada@example.com"));
    }

    #[test]
    fn an_allowed_address_is_matched_however_it_is_capitalised() {
        let mut list = AllowList::new();
        list.allow("Ada@Example.com");
        assert!(list.is_allowed("ada@example.com"));
        assert!(list.is_allowed("  ADA@EXAMPLE.COM "));
    }

    #[test]
    fn a_domain_grant_covers_the_addresses_under_it() {
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
        let list = AllowList::parse("this is not the file you are looking for\n\0\u{1}");
        assert!(list.is_empty());
        assert!(!list.is_allowed("ada@example.com"));
    }

    #[test]
    fn a_line_before_any_heading_belongs_to_nothing() {
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
        let path = scratch("glib-written");
        std::fs::write(
            &path,
            "[AlwaysAllow]\nada@example.com=true\nbea@example.com=false\n",
        )
        .unwrap();

        let list = RemoteImageAllowList::load_from(&path);

        assert!(list.is_allowed("ada@example.com"));
        assert!(!list.is_allowed("bea@example.com"));
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
    fn a_grant_changed_through_the_shell_keeps_the_treatments() {
        // A revoke through `AllowList` alone must not forget treatments.
        let path = scratch("treatment-kept-by-grants");
        let mut list = RemoteImageAllowList::default();
        list.set_treatment("news@example.com", Some(Treatment::Paper));
        list.save_to(&path).unwrap();

        let mut list = RemoteImageAllowList::load_from(&path);
        list.grants_mut().allow_domain("example.org");
        list.save_to(&path).unwrap();

        let reloaded = RemoteImageAllowList::load_from(&path);
        assert!(reloaded.is_allowed("bea@example.org"));
        assert_eq!(
            reloaded.treatment_for("news@example.com"),
            Some(Treatment::Paper)
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
        // The section follows `[domains]`; an unaware reader would take its lines for domains.
        let list = AllowList::parse(
            "[addresses]\nada@example.com\n\n[domains]\nrelay.example.net\n\n\
             [Treatment]\nnews@example.com=paper\n",
        );
        assert!(list.is_allowed("ada@example.com"));
        assert!(!list.is_allowed("news@example.com"));
        assert_eq!(list.domains().count(), 1);
    }
}
