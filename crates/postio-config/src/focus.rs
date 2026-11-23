use std::fmt;

use chrono::{NaiveTime, Weekday};
use postio_model::EmailAddress;
use serde::{Deserialize, Serialize};

use crate::Extras;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FocusConfig {
    /// `false` stops filing new mail into Filtered; what is already there stays.
    #[serde(default = "crate::yes")]
    pub filtering: bool,
    #[serde(default)]
    pub reading: Reading,
    #[serde(default)]
    pub after_unsubscribe: AfterUnsubscribe,
    #[serde(default, skip_serializing_if = "FocusFilter::is_empty")]
    pub filter: FocusFilter,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub digests: Vec<DigestRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::model::FocusModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<FocusVault>,
    /// Never read from or written to the file.
    #[serde(skip)]
    pub contacts: Vec<crate::contact_rules::ContactRule>,
    #[serde(flatten)]
    pub extras: Extras,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reading {
    #[default]
    Dialog,
    Pane,
}

impl Reading {
    pub fn as_str(self) -> &'static str {
        match self {
            Reading::Dialog => "dialog",
            Reading::Pane => "pane",
        }
    }
}

/// Neither happens unless the notice's button is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AfterUnsubscribe {
    #[default]
    Archive,
    Delete,
}

impl AfterUnsubscribe {
    pub fn as_str(self) -> &'static str {
        match self {
            AfterUnsubscribe::Archive => "archive",
            AfterUnsubscribe::Delete => "delete",
        }
    }
}

impl Default for FocusConfig {
    fn default() -> Self {
        FocusConfig {
            filtering: true,
            reading: Reading::default(),
            after_unsubscribe: AfterUnsubscribe::default(),
            filter: FocusFilter::default(),
            digests: Vec::new(),
            model: None,
            vault: None,
            contacts: Vec::new(),
            extras: Extras::default(),
        }
    }
}

/// The notes are relative to the vault and stay inside it. `postio-vault` reads and writes the
/// vault;
/// this crate only reads the section.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FocusVault {
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects: Option<String>,
    #[serde(flatten)]
    pub extras: Extras,
}

impl FocusVault {
    /// `None` when the section names no absolute folder.
    pub fn root(&self) -> Option<std::path::PathBuf> {
        let path = self.path.trim();
        let root = match path.strip_prefix("~/") {
            Some(rest) => std::path::PathBuf::from(std::env::var_os("HOME")?).join(rest),
            None => std::path::PathBuf::from(path),
        };
        root.is_absolute().then_some(root)
    }

    pub fn tasks_note(&self) -> std::path::PathBuf {
        self.tasks_note
            .as_deref()
            .map(str::trim)
            .filter(|note| !note.is_empty())
            .unwrap_or("Tasks.md")
            .into()
    }

    pub fn projects(&self) -> Option<std::path::PathBuf> {
        self.projects
            .as_deref()
            .map(str::trim)
            .filter(|folder| !folder.is_empty())
            .map(Into::into)
    }
}

pub(crate) fn stays_inside(path: &std::path::Path) -> bool {
    use std::path::Component;
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FocusFilter {
    /// Each entry is an address or a whole domain (`@example.org`); one that is neither is reported
    /// by validation and pins nobody.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub never: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_markers: Vec<StopMarker>,
    #[serde(flatten)]
    pub extras: Extras,
}

/// Read as written, so an entry with a field missing or wrong is validation's to report and stops
/// nothing, rather than an error that would stop the whole file loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopMarker {
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub kind: String,
}

impl StopMarker {
    pub fn is_well_formed(&self) -> bool {
        never_entry(&self.sender).is_some()
            && matches!(
                self.kind.trim().to_ascii_lowercase().as_str(),
                "question" | "todo"
            )
    }

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
    pub fn is_empty(&self) -> bool {
        self.never.is_empty() && self.stop_markers.is_empty() && self.extras.is_empty()
    }

    pub fn stops(&self, sender: &EmailAddress, kind: &str) -> bool {
        self.stop_markers
            .iter()
            .any(|stop| stop.is_well_formed() && stop.covers(sender, kind))
    }

    /// A domain entry covers that domain only, not the ones under it.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeverEntry<'a> {
    Address(&'a str),
    Domain(&'a str),
}

pub fn never_entry(entry: &str) -> Option<NeverEntry<'_>> {
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DigestRule {
    /// Unique among the rules.
    #[serde(default)]
    pub name: String,
    /// A list, because the language has no `OR`.
    #[serde(rename = "match", default)]
    pub queries: Vec<String>,
    #[serde(default)]
    pub cadence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<toml::Value>,
    #[serde(default)]
    pub at: String,
    #[serde(flatten)]
    pub extras: Extras,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Daily { at: NaiveTime },
    Weekly { day: Weekday, at: NaiveTime },
    Monthly { day: u32, at: NaiveTime },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DueError {
    UnknownCadence(String),
    DayForDaily,
    NotAWeekday,
    NotADayOfTheMonth,
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
    /// Reading it connects to nothing and looks nothing up.
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

    /// A rule validation reports is left out and the others still apply (ADR 0008 Q6). Whether each
    /// query reads in full is for the parser to say (`postio_ui::digest`), since this crate keeps
    /// queries as text.
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

fn time_of_day(text: &str) -> Option<NaiveTime> {
    let (hour, minute) = text.trim().split_once(':')?;
    let digits = |part: &str, widths: std::ops::RangeInclusive<usize>| {
        (widths.contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    NaiveTime::from_hms_opt(digits(hour, 1..=2)?, digits(minute, 2..=2)?, 0)
}

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
