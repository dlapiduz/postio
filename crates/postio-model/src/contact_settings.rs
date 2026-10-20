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
