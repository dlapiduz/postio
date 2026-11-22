//! What the user decided about one correspondent: where mail arrives, which markers are off, how it
//! is drawn.
//! Kept per person, so a second joined address is covered by what they already chose (ADR 0048).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContactArrival {
    /// In the inbox, and never filtered away: a pinned sender.
    Inbox,
    Digest,
    Filtered,
}

impl ContactArrival {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Digest => "digest",
            Self::Filtered => "filtered",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "inbox" => Some(Self::Inbox),
            "digest" => Some(Self::Digest),
            "filtered" => Some(Self::Filtered),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestSetting {
    /// Kept from the rule it came from: the mail held and the deliveries made are filed under it.
    pub name: String,
    pub cadence: String,
    /// A weekday, or a day of the month, as the rule wrote it.
    pub day: Option<String>,
    /// `HH:MM`, local time.
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoppedMarker {
    Question,
    Todo,
}

impl StoppedMarker {
    pub const ALL: [Self; 2] = [Self::Question, Self::Todo];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Todo => "todo",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "question" => Some(Self::Question),
            "todo" => Some(Self::Todo),
            _ => None,
        }
    }

    pub fn bit(self) -> u8 {
        match self {
            Self::Question => 1,
            Self::Todo => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CardSwitch {
    Notify,
    HtmlAsSent,
    Remind,
    Archive,
}

impl CardSwitch {
    pub const REMIND_DAYS: u32 = 3;
    pub const ARCHIVE_DAYS: u32 = 30;

    pub fn is_on(self, settings: &ContactSettings) -> bool {
        match self {
            Self::Notify => settings.notify,
            Self::HtmlAsSent => settings.html_treatment.as_deref() == Some("paper"),
            Self::Remind => settings.remind_days.is_some(),
            Self::Archive => settings.archive_after_days.is_some(),
        }
    }

    pub fn set(self, settings: &mut ContactSettings, on: bool) {
        match self {
            Self::Notify => settings.notify = on,
            Self::HtmlAsSent => settings.html_treatment = on.then(|| "paper".to_owned()),
            Self::Remind => settings.remind_days = on.then_some(Self::REMIND_DAYS),
            Self::Archive => settings.archive_after_days = on.then_some(Self::ARCHIVE_DAYS),
        }
    }
}

/// Each switch decides whether a piece of work is done at all, not whether its result is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Suggestion {
    SaveAutomatically,
    RecogniseKinds,
    ReadSignatures,
    SuggestMerges,
}

impl Suggestion {
    pub const ALL: [Self; 4] = [
        Self::SaveAutomatically,
        Self::RecogniseKinds,
        Self::ReadSignatures,
        Self::SuggestMerges,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::SaveAutomatically => "save_automatically",
            Self::RecogniseKinds => "recognise_kinds",
            Self::ReadSignatures => "read_signatures",
            Self::SuggestMerges => "suggest_merges",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|one| one.name() == name)
    }

    /// Saving people automatically is the one that is off by default.
    pub fn default_on(self) -> bool {
        !matches!(self, Self::SaveAutomatically)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactSuggestions {
    on: [bool; 4],
}

impl Default for ContactSuggestions {
    fn default() -> Self {
        Self {
            on: Suggestion::ALL.map(Suggestion::default_on),
        }
    }
}

impl ContactSuggestions {
    pub fn is_on(&self, which: Suggestion) -> bool {
        self.on[Self::slot(which)]
    }

    #[must_use]
    pub fn with(mut self, which: Suggestion, on: bool) -> Self {
        self.on[Self::slot(which)] = on;
        self
    }

    fn slot(which: Suggestion) -> usize {
        Suggestion::ALL
            .iter()
            .position(|one| *one == which)
            .expect("every suggestion is in ALL")
    }
}

/// Everything decided about one correspondent. The default is "nothing", and
/// a correspondent with nothing decided has no row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContactSettings {
    pub arrival: Option<ContactArrival>,
    pub digest: Option<DigestSetting>,
    pub stop_markers: u8,
    pub notify: bool,
    pub html_treatment: Option<String>,
    pub remind_days: Option<u32>,
    pub archive_after_days: Option<u32>,
    /// "Ignore sender": the new-sender popover is never offered for them again.
    pub ignored: bool,
}

impl ContactSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn stops(&self, kind: StoppedMarker) -> bool {
        self.stop_markers & kind.bit() != 0
    }

    pub fn set_stop(&mut self, kind: StoppedMarker, off: bool) {
        if off {
            self.stop_markers |= kind.bit();
        } else {
            self.stop_markers &= !kind.bit();
        }
    }

    /// Takes what `other` decided where this one decided nothing, so a pin or digest is never
    /// silently dropped
    /// by a merge. Where both decided, this one stands. A marker kind is a gap per kind.
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
    fn the_four_switches_start_as_the_spec_says_and_each_changes_alone() {
        let start = ContactSuggestions::default();
        assert!(!start.is_on(Suggestion::SaveAutomatically));
        for which in [
            Suggestion::RecogniseKinds,
            Suggestion::ReadSignatures,
            Suggestion::SuggestMerges,
        ] {
            assert!(start.is_on(which), "{which:?} is on by default");
        }
        for which in Suggestion::ALL {
            let flipped = start.with(which, !start.is_on(which));
            for other in Suggestion::ALL {
                assert_eq!(
                    flipped.is_on(other),
                    start.is_on(other) != (other == which),
                    "{which:?} moved {other:?}"
                );
            }
            assert_eq!(Suggestion::from_name(which.name()), Some(which));
        }
        assert_eq!(Suggestion::from_name("autosave"), None);
    }

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
