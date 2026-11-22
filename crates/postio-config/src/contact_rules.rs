//! The engine reads both homes of a correspondent's decisions: the file, and
//! `FocusConfig::with_contacts`, which adds them as the entries the file would have held, so every
//! decision function answers for both.

use postio_model::{ContactSettings, DigestSetting, StoppedMarker};

use crate::focus::{DigestRule, FocusConfig, NeverEntry, StopMarker, never_entry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactRule {
    pub contact: i64,
    pub name: String,
    pub addresses: Vec<String>,
    pub settings: ContactSettings,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Movable {
    pub pins: Vec<String>,
    pub stops: Vec<(String, StoppedMarker)>,
    pub digests: Vec<(String, DigestRule)>,
}

pub fn movable(focus: &FocusConfig) -> Movable {
    let mut moving = Movable::default();
    for entry in &focus.filter.never {
        if let Some(NeverEntry::Address(address)) = never_entry(entry) {
            let address = address.to_lowercase();
            if !moving.pins.contains(&address) {
                moving.pins.push(address);
            }
        }
    }
    for stop in &focus.filter.stop_markers {
        if let (true, Some(NeverEntry::Address(address)), Some(kind)) = (
            stop.is_well_formed(),
            never_entry(&stop.sender),
            StoppedMarker::from_name(&stop.kind),
        ) {
            let entry = (address.to_lowercase(), kind);
            if !moving.stops.contains(&entry) {
                moving.stops.push(entry);
            }
        }
    }
    for (rule, _) in focus.applicable_digests() {
        if let Some(address) = digest_address(rule) {
            moving.digests.push((address, rule.clone()));
        }
    }
    moving
}

pub fn digest_address(rule: &DigestRule) -> Option<String> {
    let [query] = rule.queries.as_slice() else {
        return None;
    };
    if !rule.extras.is_empty() || rule.due().is_err() || rule.name.trim().is_empty() {
        return None;
    }
    let query = query.trim();
    let value = query.get(..5).filter(|p| p.eq_ignore_ascii_case("from:"))?;
    let value = query[value.len()..].trim().trim_matches('"');
    match never_entry(value)? {
        NeverEntry::Address(address) => Some(address.to_lowercase()),
        NeverEntry::Domain(_) => None,
    }
}

pub fn setting_of(rule: &DigestRule) -> DigestSetting {
    DigestSetting {
        name: rule.name.trim().to_owned(),
        cadence: rule.cadence.trim().to_ascii_lowercase(),
        day: rule.day.as_ref().map(|day| match day {
            toml::Value::String(text) => text.trim().to_owned(),
            other => other.to_string(),
        }),
        at: rule.at.trim().to_owned(),
    }
}

pub fn rule_of(setting: &DigestSetting, addresses: &[String]) -> DigestRule {
    let day = setting.day.as_ref().map(|day| match day.parse::<i64>() {
        Ok(number) if setting.cadence == "monthly" => toml::Value::Integer(number),
        _ => toml::Value::String(day.clone()),
    });
    DigestRule {
        name: setting.name.clone(),
        queries: addresses
            .iter()
            .map(|address| format!("from:{address}"))
            .collect(),
        cadence: setting.cadence.clone(),
        day,
        at: setting.at.clone(),
        extras: crate::Extras::default(),
    }
}

impl FocusConfig {
    /// `rules` is remembered, so a page can name the correspondent behind an entry.
    #[must_use]
    pub fn with_contacts(&self, rules: &[ContactRule]) -> FocusConfig {
        let mut merged = self.clone();
        for rule in rules {
            if rule.settings.arrival == Some(postio_model::ContactArrival::Inbox) {
                merged.filter.never.extend(rule.addresses.iter().cloned());
            }
            for kind in rule.settings.stopped() {
                merged
                    .filter
                    .stop_markers
                    .extend(rule.addresses.iter().map(|address| StopMarker {
                        sender: address.clone(),
                        kind: kind.as_str().to_owned(),
                    }));
            }
            if rule.settings.arrival == Some(postio_model::ContactArrival::Digest)
                && let Some(setting) = &rule.settings.digest
            {
                merged.digests.push(rule_of(setting, &rule.addresses));
            }
        }
        merged.contacts = rules.to_vec();
        merged
    }

    pub fn contacts(&self) -> &[ContactRule] {
        &self.contacts
    }

    pub fn contact_pinned(&self, entry: &str) -> Option<&ContactRule> {
        let entry = entry.trim().to_lowercase();
        self.contacts.iter().find(|rule| {
            rule.settings.arrival == Some(postio_model::ContactArrival::Inbox)
                && rule.addresses.contains(&entry)
        })
    }

    pub fn contact_stopped(&self, sender: &str, kind: &str) -> Option<&ContactRule> {
        let sender = sender.trim().to_lowercase();
        let kind = StoppedMarker::from_name(kind)?;
        self.contacts
            .iter()
            .find(|rule| rule.settings.stops(kind) && rule.addresses.contains(&sender))
    }

    pub fn contact_digest(&self, name: &str) -> Option<&ContactRule> {
        self.contacts.iter().find(|rule| {
            rule.settings.arrival == Some(postio_model::ContactArrival::Digest)
                && rule
                    .settings
                    .digest
                    .as_ref()
                    .is_some_and(|digest| digest.name == name.trim())
        })
    }
}

#[cfg(test)]
mod tests {
    use postio_model::{ContactArrival, EmailAddress};

    use super::*;
    use crate::Config;

    const FILE: &str = r#"
[focus.filter]
never = ["Ada@Example.org", "@example.net", "bob@example.org", "not an entry"]
stop_markers = [
  { sender = "ada@example.org", kind = "question" },
  { sender = "ada@example.org", kind = "todo" },
  { sender = "@example.net", kind = "todo" },
  { sender = "bob@example.org", kind = "invite" },
]

[[focus.digests]]
name = "Ledger"
match = ["from:news@ledger.example"]
cadence = "weekly"
day = "saturday"
at = "16:00"

[[focus.digests]]
name = "Lists"
match = ["list:dev.lists.example"]
cadence = "daily"
at = "08:00"

[[focus.digests]]
name = "Two"
match = ["from:a@ledger.example", "from:b@ledger.example"]
cadence = "monthly"
day = 3
at = "09:30"

[[focus.digests]]
name = "Domain"
match = ["from:@promo.example"]
cadence = "daily"
at = "07:00"

[[focus.digests]]
name = "Broken"
match = ["from:late@ledger.example"]
cadence = "fortnightly"
at = "07:00"
"#;

    fn focus(text: &str) -> FocusConfig {
        Config::from_toml_str(text).expect("a config").focus
    }

    #[test]
    fn only_what_one_correspondent_can_hold_is_movable() {
        let moving = movable(&focus(FILE));
        assert_eq!(moving.pins, ["ada@example.org", "bob@example.org"]);
        assert_eq!(
            moving.stops,
            [
                ("ada@example.org".to_owned(), StoppedMarker::Question),
                ("ada@example.org".to_owned(), StoppedMarker::Todo)
            ],
            "a domain and an unknown kind stay in the file"
        );
        assert_eq!(
            moving.digests.len(),
            1,
            "one address, one query, a valid time"
        );
        assert_eq!(moving.digests[0].0, "news@ledger.example");
        assert_eq!(moving.digests[0].1.name, "Ledger");
    }

    #[test]
    fn a_contacts_rules_read_as_the_entries_the_file_would_have_held() {
        let ada = ContactRule {
            contact: 1,
            name: "Ada Lovelace".into(),
            addresses: vec!["ada@work.example".into(), "ada@home.example".into()],
            settings: {
                let mut settings = ContactSettings {
                    arrival: Some(ContactArrival::Inbox),
                    ..ContactSettings::default()
                };
                settings.set_stop(StoppedMarker::Todo, true);
                settings
            },
        };
        let ledger = ContactRule {
            contact: 2,
            name: "Ledger Weekly".into(),
            addresses: vec!["news@ledger.example".into()],
            settings: ContactSettings {
                arrival: Some(ContactArrival::Digest),
                digest: Some(DigestSetting {
                    name: "Ledger".into(),
                    cadence: "weekly".into(),
                    day: Some("saturday".into()),
                    at: "16:00".into(),
                }),
                ..ContactSettings::default()
            },
        };
        let merged =
            focus("[focus.filter]\nnever = [\"@example.net\"]\n").with_contacts(&[ada, ledger]);

        let at = |address: &str| EmailAddress::new(None::<String>, address);
        assert!(
            merged.filter.never_filters(&at("ADA@home.example")),
            "every address"
        );
        assert!(
            merged.filter.never_filters(&at("anyone@example.net")),
            "the file still applies"
        );
        assert!(!merged.filter.never_filters(&at("eve@home.example")));
        assert!(merged.filter.stops(&at("ada@work.example"), "todo"));
        assert!(!merged.filter.stops(&at("ada@work.example"), "question"));
        let digests = merged.applicable_digests();
        assert_eq!(digests.len(), 1);
        assert_eq!(digests[0].0.name, "Ledger");
        assert_eq!(digests[0].0.queries, ["from:news@ledger.example"]);
        assert_eq!(
            merged.contact_pinned("Ada@Home.example").unwrap().name,
            "Ada Lovelace"
        );
        assert_eq!(
            merged.contact_digest("Ledger").unwrap().name,
            "Ledger Weekly"
        );
        assert!(merged.contact_stopped("ada@work.example", "todo").is_some());
        assert!(
            merged.contact_pinned("@example.net").is_none(),
            "that one is the file's"
        );
    }

    #[test]
    fn moving_entries_onto_correspondents_changes_no_decision() {
        let before = focus(FILE);
        let moving = movable(&before);

        let mut text = FILE.to_owned();
        for pin in &moving.pins {
            text = crate::focus_edit::set_never(&text, pin, false)
                .unwrap()
                .unwrap_or(text);
        }
        for (sender, kind) in &moving.stops {
            text = crate::focus_edit::set_stop_marker(&text, sender, kind.as_str(), false)
                .unwrap()
                .unwrap_or(text);
        }
        for (_, rule) in &moving.digests {
            text = crate::focus_edit::remove_digest_rule(&text, &rule.name)
                .unwrap()
                .map(|(text, _, _)| text)
                .unwrap_or(text);
        }
        let file_after = focus(&text);
        assert!(
            file_after
                .filter
                .never
                .iter()
                .all(|e| !e.eq_ignore_ascii_case("ada@example.org"))
        );

        let mut rules: Vec<ContactRule> = Vec::new();
        let rule_for = |address: &str, rules: &mut Vec<ContactRule>| -> usize {
            if let Some(at) = rules
                .iter()
                .position(|r| r.addresses == [address.to_owned()])
            {
                return at;
            }
            rules.push(ContactRule {
                contact: rules.len() as i64 + 1,
                name: address.into(),
                addresses: vec![address.into()],
                settings: ContactSettings::default(),
            });
            rules.len() - 1
        };
        for pin in &moving.pins {
            let at = rule_for(pin, &mut rules);
            rules[at].settings.arrival = Some(ContactArrival::Inbox);
        }
        for (sender, kind) in &moving.stops {
            let at = rule_for(sender, &mut rules);
            rules[at].settings.set_stop(*kind, true);
        }
        for (address, rule) in &moving.digests {
            let at = rule_for(address, &mut rules);
            rules[at].settings.arrival = Some(ContactArrival::Digest);
            rules[at].settings.digest = Some(setting_of(rule));
        }
        let after = file_after.with_contacts(&rules);

        let senders = [
            "ada@example.org",
            "Ada@Example.ORG",
            "bob@example.org",
            "carol@example.org",
            "x@example.net",
            "news@ledger.example",
            "a@ledger.example",
            "late@ledger.example",
            "nobody@nowhere.example",
        ];
        for sender in senders {
            let sender = EmailAddress::new(None::<String>, sender);
            assert_eq!(
                before.filter.never_filters(&sender),
                after.filter.never_filters(&sender),
                "pinned: {}",
                sender.address
            );
            for kind in ["question", "todo", "invite"] {
                assert_eq!(
                    before.filter.stops(&sender, kind),
                    after.filter.stops(&sender, kind),
                    "{kind} stopped: {}",
                    sender.address
                );
            }
        }
        let describe = |config: &FocusConfig| -> Vec<(String, Vec<String>, crate::focus::Due)> {
            let mut all: Vec<_> = config
                .applicable_digests()
                .into_iter()
                .map(|(rule, due)| {
                    (
                        rule.name.clone(),
                        rule.queries.iter().map(|q| q.to_lowercase()).collect(),
                        due,
                    )
                })
                .collect();
            all.sort_by(|a, b| a.0.cmp(&b.0));
            all
        };
        assert_eq!(describe(&before), describe(&after));
    }
}
