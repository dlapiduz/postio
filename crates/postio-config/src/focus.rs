//! `[focus]` -- Focus's own settings (spec 007 FR-160, contracts/config.md).
//!
//! ```toml
//! [focus]
//! filtering = true        # FR-119: file spam and updates away as they arrive
//!
//! [focus.filter]
//! never = ["ada@example.org", "@example.net"]   # never filtered (FR-111, FR-116)
//!
//! [[focus.digests]]
//! name    = "Newsletters"
//! match   = ["from:news@localfirst.example", "from:editor@ledger.example"]
//! cadence = "weekly"      # daily | weekly | monthly
//! day     = "saturday"    # weekly: a weekday; monthly: 1-28; daily: none
//! at      = "16:00"       # local time
//! ```
//!
//! The file's order is the order rules are listed and matched in, and the
//! first rule that matches holds the message (ADR 0008 Q4). A rule's fields
//! are read as written -- a cadence as a string, a day as whatever TOML value
//! it is -- so a wrong value is a semantic problem [`crate::validate`]
//! reports, and the rule is left out ([`FocusConfig::applicable_digests`]),
//! rather than a schema error that would stop the whole file loading (ADR
//! 0008 Q6). A rule's queries stay text: the query language parses them,
//! not this crate.

use std::fmt;

use chrono::{NaiveTime, Weekday};
use postio_model::EmailAddress;
use serde::{Deserialize, Serialize};

use crate::Extras;

/// The `[focus]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FocusConfig {
    /// Whether Focus files spam and automated updates away as they arrive
    /// (FR-119). On when Focus first opens. `false` stops filing new mail
    /// into Filtered; what is already there stays where it is.
    #[serde(default = "crate::yes")]
    pub filtering: bool,
    /// `[focus.filter]`: the user's own word on filtering.
    #[serde(default, skip_serializing_if = "FocusFilter::is_empty")]
    pub filter: FocusFilter,
    /// `[[focus.digests]]`, in the file's order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub digests: Vec<DigestRule>,
    /// `[focus.model]`: the person's own model, off unless present (FR-166).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::model::FocusModel>,
    /// Keys in `[focus]` this version of Postio does not know.
    #[serde(flatten)]
    pub extras: Extras,
}

impl Default for FocusConfig {
    fn default() -> Self {
        FocusConfig {
            filtering: true,
            filter: FocusFilter::default(),
            digests: Vec::new(),
            model: None,
            extras: Extras::default(),
        }
    }
}

/// `[focus.filter]`: what the user has told Focus about filtering.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FocusFilter {
    /// Senders Focus never files away: pinned (FR-111), or restored from
    /// Filtered with `R` (FR-116). Each entry is an address
    /// (`ada@example.org`) or a whole domain (`@example.org`); one that is
    /// neither is reported by validation and pins nobody.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub never: Vec<String>,
    /// Marker kinds the person stopped for a sender, by dismissing them
    /// again and again (FR-108): the detector does not mark that kind in
    /// that sender's mail any more.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_markers: Vec<StopMarker>,
    /// Keys in `[focus.filter]` this version of Postio does not know.
    #[serde(flatten)]
    pub extras: Extras,
}

/// One `[focus.filter] stop_markers` entry: a marker kind stopped for one
/// sender (contracts/config.md). Read as written, so an entry with a field
/// missing or wrong is validation's to report and stops nothing, rather
/// than an error that would stop the whole file loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopMarker {
    /// The sender's address.
    #[serde(default)]
    pub sender: String,
    /// `question` or `todo`.
    #[serde(default)]
    pub kind: String,
}

impl StopMarker {
    /// Whether the entry names a sender -- an address, or a whole domain as
    /// `never` does -- and a kind of marker that can be dismissed: what
    /// validation asks of it.
    pub fn is_well_formed(&self) -> bool {
        never_entry(&self.sender).is_some()
            && matches!(
                self.kind.trim().to_ascii_lowercase().as_str(),
                "question" | "todo"
            )
    }

    /// Whether it stops `kind` markers in mail from `sender`.
    fn covers(&self, sender: &EmailAddress, kind: &str) -> bool {
        if !self.kind.trim().eq_ignore_ascii_case(kind) {
            return false;
        }
        let Some((_, domain)) = sender.address.rsplit_once('@') else {
            return false;
        };
        match never_entry(&self.sender) {
            Some(NeverEntry::Address(stopped)) => stopped.eq_ignore_ascii_case(&sender.address),
            Some(NeverEntry::Domain(stopped)) => stopped.eq_ignore_ascii_case(domain),
            None => false,
        }
    }
}

impl FocusFilter {
    /// Nothing in the section: it is not written back.
    pub fn is_empty(&self) -> bool {
        self.never.is_empty() && self.stop_markers.is_empty() && self.extras.is_empty()
    }

    /// Whether the person stopped `kind` markers -- `question` or `todo` --
    /// for mail from `sender`, in any case.
    pub fn stops(&self, sender: &EmailAddress, kind: &str) -> bool {
        self.stop_markers
            .iter()
            .any(|stop| stop.is_well_formed() && stop.covers(sender, kind))
    }

    /// Whether `address` is a sender Focus never files away: named by an
    /// entry, in any case, or at a domain an entry names whole -- that
    /// domain only, not the ones under it.
    pub fn never_filters(&self, address: &EmailAddress) -> bool {
        let Some((_, domain)) = address.address.rsplit_once('@') else {
            return false;
        };
        self.never.iter().any(|entry| match never_entry(entry) {
            Some(NeverEntry::Address(pinned)) => pinned.eq_ignore_ascii_case(&address.address),
            Some(NeverEntry::Domain(pinned)) => pinned.eq_ignore_ascii_case(domain),
            None => false,
        })
    }
}

/// What a `[focus.filter] never` entry names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NeverEntry<'a> {
    /// One address.
    Address(&'a str),
    /// Every address at a domain.
    Domain(&'a str),
}

/// What `entry` names, or `None` when it is neither an address nor a whole
/// domain: an `@` with something on each side of it, or one at the start
/// with a domain after it, and no space or second `@` anywhere.
pub(crate) fn never_entry(entry: &str) -> Option<NeverEntry<'_>> {
    let entry = entry.trim();
    if entry.chars().any(char::is_whitespace) || entry.matches('@').count() != 1 {
        return None;
    }
    let (local, domain) = entry.split_once('@')?;
    if domain.is_empty() {
        return None;
    }
    Some(if local.is_empty() {
        NeverEntry::Domain(domain)
    } else {
        NeverEntry::Address(entry)
    })
}

/// One `[[focus.digests]]` rule, as written: a digest by sender, and later
/// by list or search (spec 007 FR-127).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DigestRule {
    /// What the digest is called: its row, and the rules list, name it.
    /// Unique among the rules.
    #[serde(default)]
    pub name: String,
    /// Queries in the one query language; the rule holds a message when any
    /// of them matches it. A list, because the language has no `OR`.
    #[serde(rename = "match", default)]
    pub queries: Vec<String>,
    /// `daily`, `weekly` or `monthly`.
    #[serde(default)]
    pub cadence: String,
    /// A weekday, or a day of the month.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<toml::Value>,
    /// `HH:MM`, local time.
    #[serde(default)]
    pub at: String,
    /// Keys in the rule this version of Postio does not know.
    #[serde(flatten)]
    pub extras: Extras,
}

/// When a digest comes due, as [`DigestRule::due`] reads it: a cadence, the
/// day it names, and a local time. `postio_ui::schedule::next_due` turns it
/// into the next instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// Every day.
    Daily {
        /// The local time.
        at: NaiveTime,
    },
    /// Every week.
    Weekly {
        /// The weekday.
        day: Weekday,
        /// The local time.
        at: NaiveTime,
    },
    /// Every month.
    Monthly {
        /// The day of the month, from 1 to 28, so every month has one.
        day: u32,
        /// The local time.
        at: NaiveTime,
    },
}

/// Why a rule's cadence, day and time name no time. Each says what to
/// write instead, for the validity line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DueError {
    /// `cadence` is not `daily`, `weekly` or `monthly`.
    UnknownCadence(String),
    /// A daily rule was given a `day`.
    DayForDaily,
    /// A weekly rule's `day` is missing, or is not a weekday.
    NotAWeekday,
    /// A monthly rule's `day` is missing, or is not a day from 1 to 28.
    NotADayOfTheMonth,
    /// `at` is not a time of day.
    NotATime,
}

impl fmt::Display for DueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DueError::UnknownCadence(cadence) => write!(
                f,
                "`{cadence}` is not a cadence; use `daily`, `weekly` or `monthly`"
            ),
            DueError::DayForDaily => {
                f.write_str("a daily digest comes every day, so it takes no `day`")
            }
            DueError::NotAWeekday => {
                f.write_str("a weekly digest needs a `day` that is a weekday, such as `saturday`")
            }
            DueError::NotADayOfTheMonth => f.write_str(
                "a monthly digest needs a `day` from 1 to 28, so that every month has one",
            ),
            DueError::NotATime => f.write_str("`at` must be a time of day, such as `16:00`"),
        }
    }
}

impl DigestRule {
    /// A rule called `name`, holding what any of `queries` matches, coming
    /// `due`: what the rule dialog writes (spec 007 FR-120), spelled as a
    /// person would write it by hand.
    pub fn from_due(name: impl Into<String>, queries: Vec<String>, due: Due) -> Self {
        let (cadence, day, at) = match due {
            Due::Daily { at } => ("daily", None, at),
            Due::Weekly { day, at } => (
                "weekly",
                Some(toml::Value::String(weekday_name(day).to_owned())),
                at,
            ),
            Due::Monthly { day, at } => ("monthly", Some(toml::Value::Integer(i64::from(day))), at),
        };
        DigestRule {
            name: name.into(),
            queries,
            cadence: cadence.to_owned(),
            day,
            at: at.format("%H:%M").to_string(),
            extras: Extras::default(),
        }
    }

    /// When this rule comes due, read from its cadence, day and time: the
    /// one reading of them, which validation reports on and the due timer
    /// computes from (`postio_ui::schedule::next_due`).
    pub fn due(&self) -> Result<Due, DueError> {
        let cadence = self.cadence.trim().to_ascii_lowercase();
        if !matches!(cadence.as_str(), "daily" | "weekly" | "monthly") {
            return Err(DueError::UnknownCadence(self.cadence.clone()));
        }
        let at = time_of_day(&self.at).ok_or(DueError::NotATime)?;
        match cadence.as_str() {
            "daily" => match self.day {
                None => Ok(Due::Daily { at }),
                Some(_) => Err(DueError::DayForDaily),
            },
            "weekly" => self
                .day
                .as_ref()
                .and_then(toml::Value::as_str)
                .and_then(weekday)
                .map(|day| Due::Weekly { day, at })
                .ok_or(DueError::NotAWeekday),
            _ => self
                .day
                .as_ref()
                .and_then(toml::Value::as_integer)
                .filter(|day| (1..=28).contains(day))
                .and_then(|day| u32::try_from(day).ok())
                .map(|day| Due::Monthly { day, at })
                .ok_or(DueError::NotADayOfTheMonth),
        }
    }
}

impl FocusConfig {
    /// The model to ask for `feature`, and its endpoint: `None` unless
    /// `[focus.model]` is present, usable, and has `feature`'s switch on.
    /// Reading it connects to nothing and looks nothing up (FR-166).
    pub fn model_for(
        &self,
        feature: crate::model::ModelFeature,
    ) -> Option<(crate::model::ModelEndpoint, &str)> {
        let model = self.model.as_ref()?;
        if !model.switched_on(feature) {
            return None;
        }
        model.usable().ok()
    }

    /// The rules that apply, in the file's order, with when each comes due.
    ///
    /// A rule validation reports is left out and the others still apply
    /// (ADR 0008 Q6): one with no name or no query, a blank query, a
    /// cadence, day or time that names no time, or a name an earlier rule
    /// already has. Whether each query reads in full in the one query
    /// language is for the parser to say (`postio_ui::digest`), since this
    /// crate keeps queries as text.
    pub fn applicable_digests(&self) -> Vec<(&DigestRule, Due)> {
        let mut named: Vec<&str> = Vec::new();
        self.digests
            .iter()
            .filter_map(|rule| {
                let name = rule.name.trim();
                if name.is_empty() || named.contains(&name) {
                    return None;
                }
                named.push(name);
                if rule.queries.is_empty()
                    || rule.queries.iter().any(|query| query.trim().is_empty())
                {
                    return None;
                }
                rule.due().ok().map(|due| (rule, due))
            })
            .collect()
    }
}

/// `HH:MM` on the 24-hour clock, the hour in one digit or two.
fn time_of_day(text: &str) -> Option<NaiveTime> {
    let (hour, minute) = text.trim().split_once(':')?;
    let digits = |part: &str, widths: std::ops::RangeInclusive<usize>| {
        (widths.contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    NaiveTime::from_hms_opt(digits(hour, 1..=2)?, digits(minute, 2..=2)?, 0)
}

/// A weekday's name, as a rule spells it.
fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "monday",
        Weekday::Tue => "tuesday",
        Weekday::Wed => "wednesday",
        Weekday::Thu => "thursday",
        Weekday::Fri => "friday",
        Weekday::Sat => "saturday",
        Weekday::Sun => "sunday",
    }
}

/// A weekday by its name, or its first three letters, in any case.
fn weekday(name: &str) -> Option<Weekday> {
    let name = name.trim().to_ascii_lowercase();
    [
        ("monday", Weekday::Mon),
        ("tuesday", Weekday::Tue),
        ("wednesday", Weekday::Wed),
        ("thursday", Weekday::Thu),
        ("friday", Weekday::Fri),
        ("saturday", Weekday::Sat),
        ("sunday", Weekday::Sun),
    ]
    .into_iter()
    .find(|(full, _)| name == *full || name == full[..3])
    .map(|(_, day)| day)
}
