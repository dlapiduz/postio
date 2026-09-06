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
//! file format live here, and each frontend supplies a path. `postio-gtk`
//! still has its own; adopting this one, with a migration, is #1273.
//!
//! # Addresses and domains
//!
//! Two grants, deliberately. "Always allow this address" is what a person
//! means about a correspondent; "always allow this domain" is what they mean
//! about a service that sends from `notices-3a7f@relay.example.net` and never
//! from the same address twice. A domain grant covers every address under it;
//! neither covers a subdomain of the other, because `mail.example.com` and
//! `example.com` can be different senders and guessing costs privacy.

use std::collections::BTreeSet;
use std::path::Path;

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
}
