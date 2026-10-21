//! What the user has decided about one correspondent (specs/011-focus-contacts
//! S3): where their mail arrives, which markers are off for them, and how
//! their mail is drawn.
//!
//! These were once kept per address in `config.toml` and a file beside it.
//! They are the person's now, so a second address joined to them is covered
//! by what they already chose (ADR 0048).

use serde::{Deserialize, Serialize};

/// Where a correspondent's mail arrives. `None` is "by the default rules".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContactArrival {
    /// In the inbox, and never filtered away: a pinned sender.
    Inbox,
    /// Held for the digest the contact's settings name.
    Digest,
    /// Filed away as Filtered.
    Filtered,
}

impl ContactArrival {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Digest => "digest",
            Self::Filtered => "filtered",
        }
    }

    /// The inverse of [`ContactArrival::as_str`]; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "inbox" => Some(Self::Inbox),
            "digest" => Some(Self::Digest),
            "filtered" => Some(Self::Filtered),
            _ => None,
        }
    }
}

/// The digest a correspondent's mail is held for: the rule a
/// `[[focus.digests]]` entry with one `from:` query would have been.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestSetting {
    /// What the digest is called. Kept from the rule it came from, because
    /// the mail it holds and the deliveries it made are filed under it.
    pub name: String,
    /// `daily`, `weekly` or `monthly`.
    pub cadence: String,
    /// A weekday, or a day of the month, as the rule wrote it.
    pub day: Option<String>,
    /// `HH:MM`, local time.
    pub at: String,
}

/// A marker kind that can be turned off for one correspondent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoppedMarker {
    /// Questions asked of the user.
    Question,
    /// To-dos the mail gives them.
    Todo,
}

impl StoppedMarker {
    /// Both kinds.
    pub const ALL: [Self; 2] = [Self::Question, Self::Todo];

    /// The word `[focus.filter] stop_markers` and the store use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Todo => "todo",
        }
    }

    /// The kind a word names, in any case.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "question" => Some(Self::Question),
            "todo" => Some(Self::Todo),
            _ => None,
        }
    }

    /// This kind's bit in `contact_settings.stop_markers`.
    pub fn bit(self) -> u8 {
        match self {
            Self::Question => 1,
            Self::Todo => 2,
        }
    }
}

/// A switch on a contact's card: one thing the user decides about them with
/// a click (specs/011-focus-contacts S8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CardSwitch {
    /// A message from them notifies.
    Notify,
    /// Their HTML mail is always drawn as sent.
    HtmlAsSent,
    /// Remind after a few days without a reply, from the start.
    Remind,
    /// Archive their newsletters after a month.
    Archive,
}

impl CardSwitch {
    /// How many days "remind by default" waits.
    pub const REMIND_DAYS: u32 = 3;
    /// How many days "archive issues" keeps them in the inbox.
    pub const ARCHIVE_DAYS: u32 = 30;

    /// Whether `settings` have this switch on.
    pub fn is_on(self, settings: &ContactSettings) -> bool {
        match self {
            Self::Notify => settings.notify,
            Self::HtmlAsSent => settings.html_treatment.as_deref() == Some("paper"),
            Self::Remind => settings.remind_days.is_some(),
            Self::Archive => settings.archive_after_days.is_some(),
        }
    }

    /// Turns this switch on or off in `settings`.
    pub fn set(self, settings: &mut ContactSettings, on: bool) {
        match self {
            Self::Notify => settings.notify = on,
            Self::HtmlAsSent => settings.html_treatment = on.then(|| "paper".to_owned()),
            Self::Remind => settings.remind_days = on.then_some(Self::REMIND_DAYS),
            Self::Archive => settings.archive_after_days = on.then_some(Self::ARCHIVE_DAYS),
        }
    }
}

/// Everything decided about one correspondent. The default is "nothing", and
/// a correspondent with nothing decided has no row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactSettings {
    /// Where their mail arrives.
    pub arrival: Option<ContactArrival>,
    /// Their digest, when [`ContactSettings::arrival`] is `Digest`.
    pub digest: Option<DigestSetting>,
    /// Bits of [`StoppedMarker::bit`] for the marker kinds that are off.
    pub stop_markers: u8,
    /// Whether a message from them notifies.
    pub notify: bool,
    /// How their HTML mail is always drawn: `app` or `paper`.
    pub html_treatment: Option<String>,
    /// Remind after this many days without a reply.
    pub remind_days: Option<u32>,
    /// Archive their newsletters after this many days.
    pub archive_after_days: Option<u32>,
}

impl ContactSettings {
    /// Whether nothing is decided, so that no row is kept.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Whether `kind` markers are off for them.
    pub fn stops(&self, kind: StoppedMarker) -> bool {
        self.stop_markers & kind.bit() != 0
    }

    /// Turns `kind` markers off or back on.
    pub fn set_stop(&mut self, kind: StoppedMarker, off: bool) {
        if off {
            self.stop_markers |= kind.bit();
        } else {
            self.stop_markers &= !kind.bit();
        }
    }

    /// Takes what `other` decided where this one decided nothing -- a merge
    /// keeps the kept person's word and fills its gaps from the absorbed
    /// (specs/011-focus-contacts Q8), so a pin or a digest is never silently
    /// dropped. Where both decided, this one stands: an `inbox` pin is not
    /// replaced by a digest, and a digest comes with the arrival it belongs
    /// to. A marker kind is a gap per kind.
    pub fn absorb(&mut self, other: &ContactSettings) {
        if self.arrival.is_none() && other.arrival.is_some() {
            self.arrival = other.arrival;
            self.digest = other.digest.clone();
        }
        self.stop_markers |= other.stop_markers;
        self.notify |= other.notify;
        if self.html_treatment.is_none() {
            self.html_treatment = other.html_treatment.clone();
        }
        self.remind_days = self.remind_days.or(other.remind_days);
        self.archive_after_days = self.archive_after_days.or(other.archive_after_days);
    }

    /// The marker kinds that are off, in a fixed order.
    pub fn stopped(&self) -> Vec<StoppedMarker> {
        StoppedMarker::ALL
            .into_iter()
            .filter(|kind| self.stops(*kind))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_and_marker_names_round_trip_and_refuse_the_unknown() {
        for arrival in [
            ContactArrival::Inbox,
            ContactArrival::Digest,
            ContactArrival::Filtered,
        ] {
            assert_eq!(ContactArrival::from_name(arrival.as_str()), Some(arrival));
        }
        assert_eq!(ContactArrival::from_name("spam"), None);
        for kind in StoppedMarker::ALL {
            assert_eq!(StoppedMarker::from_name(kind.as_str()), Some(kind));
        }
        assert_eq!(
            StoppedMarker::from_name(" ToDo "),
            Some(StoppedMarker::Todo)
        );
        assert_eq!(StoppedMarker::from_name("invite"), None);
    }

    fn digest(name: &str) -> DigestSetting {
        DigestSetting {
            name: name.into(),
            cadence: "daily".into(),
            day: None,
            at: "08:00".into(),
        }
    }

    #[test]
    fn absorbing_fills_the_gaps_and_the_kept_persons_word_stands() {
        let mut kept = ContactSettings {
            html_treatment: Some("app".into()),
            remind_days: Some(3),
            ..ContactSettings::default()
        };
        kept.set_stop(StoppedMarker::Todo, true);
        let mut absorbed = ContactSettings {
            arrival: Some(ContactArrival::Digest),
            digest: Some(digest("Ledger")),
            notify: true,
            html_treatment: Some("paper".into()),
            remind_days: Some(9),
            archive_after_days: Some(30),
            ..ContactSettings::default()
        };
        absorbed.set_stop(StoppedMarker::Question, true);

        kept.absorb(&absorbed);

        assert_eq!(
            kept.arrival,
            Some(ContactArrival::Digest),
            "a gap, so the absorbed's"
        );
        assert_eq!(kept.digest, Some(digest("Ledger")), "with its digest");
        assert_eq!(
            kept.stopped(),
            [StoppedMarker::Question, StoppedMarker::Todo]
        );
        assert!(kept.notify);
        assert_eq!(
            kept.html_treatment.as_deref(),
            Some("app"),
            "both held one: the kept's"
        );
        assert_eq!(kept.remind_days, Some(3));
        assert_eq!(kept.archive_after_days, Some(30));
    }

    #[test]
    fn a_pin_is_not_replaced_by_the_absorbed_ones_digest() {
        let mut kept = ContactSettings {
            arrival: Some(ContactArrival::Inbox),
            ..ContactSettings::default()
        };
        kept.absorb(&ContactSettings {
            arrival: Some(ContactArrival::Digest),
            digest: Some(digest("Ledger")),
            ..ContactSettings::default()
        });
        assert_eq!(kept.arrival, Some(ContactArrival::Inbox));
        assert_eq!(kept.digest, None, "a digest only comes with its arrival");
    }

    #[test]
    fn stopping_a_kind_sets_its_bit_and_nothing_else() {
        let mut settings = ContactSettings::default();
        assert!(settings.is_default());
        settings.set_stop(StoppedMarker::Todo, true);
        assert!(settings.stops(StoppedMarker::Todo) && !settings.stops(StoppedMarker::Question));
        assert!(!settings.is_default());
        settings.set_stop(StoppedMarker::Question, true);
        assert_eq!(
            settings.stopped(),
            [StoppedMarker::Question, StoppedMarker::Todo]
        );
        settings.set_stop(StoppedMarker::Todo, false);
        settings.set_stop(StoppedMarker::Question, false);
        assert!(settings.is_default(), "a row with nothing in it is no row");
    }
}

#[cfg(test)]
mod switch_tests {
    use super::*;

    #[test]
    fn each_switch_turns_on_and_back_off_and_leaves_the_others_alone() {
        for switch in [
            CardSwitch::Notify,
            CardSwitch::HtmlAsSent,
            CardSwitch::Remind,
            CardSwitch::Archive,
        ] {
            let mut settings = ContactSettings::default();
            assert!(!switch.is_on(&settings));
            switch.set(&mut settings, true);
            assert!(switch.is_on(&settings), "{switch:?}");
            for other in [CardSwitch::Notify, CardSwitch::HtmlAsSent] {
                if other != switch {
                    assert!(!other.is_on(&settings));
                }
            }
            switch.set(&mut settings, false);
            assert!(settings.is_default(), "{switch:?} leaves nothing behind");
        }
    }
}
