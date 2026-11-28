//! Parsed here and compiled a second time by `build.rs` through `#[path]`, so the parser that
//! validates the shipped table at build time is the one the classifier loads. It names nothing else
//! in the crate.

use std::fmt;

use serde::Deserialize;

pub const REASONS: &[&str] = &[
    "spam",
    "promotion",
    "notification",
    "receipt",
    "shipping",
    "social",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub local: Vec<Vec<String>>,
    pub domain: Vec<String>,
    pub reason: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    /// `None` when the file itself is not a table.
    pub entry: Option<String>,
    pub problem: String,
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.entry {
            Some(entry) => write!(f, "senders table, entry {entry}: {}", self.problem),
            None => write!(f, "senders table: {}", self.problem),
        }
    }
}

impl std::error::Error for TableError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    sender: Vec<toml::Table>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    #[serde(default)]
    local: Vec<String>,
    #[serde(default)]
    domain: Vec<String>,
    reason: Option<String>,
    source: Option<String>,
}

/// An entry is named by its `name`, or by its position when it has none, so the error says where to
/// look without quoting anything else.
pub fn parse(text: &str) -> Result<Vec<Row>, TableError> {
    let file: File = toml::from_str(text).map_err(|error| TableError {
        entry: None,
        problem: error.to_string(),
    })?;
    let mut rows: Vec<Row> = Vec::new();
    for (index, table) in file.sender.into_iter().enumerate() {
        let entry = match table.get("name").and_then(toml::Value::as_str) {
            Some(name) => format!("`{name}`"),
            None => format!("#{}", index + 1),
        };
        let fail = |problem: String| TableError {
            entry: Some(entry.clone()),
            problem,
        };
        let raw: Entry = table
            .try_into()
            .map_err(|error: toml::de::Error| fail(error.message().to_owned()))?;
        let row = row(raw).map_err(fail)?;
        if rows.iter().any(|earlier| earlier.name == row.name) {
            return Err(fail("another entry has this name".to_owned()));
        }
        rows.push(row);
    }
    Ok(rows)
}

fn row(entry: Entry) -> Result<Row, String> {
    if entry.name.trim().is_empty() {
        return Err("its name is empty".to_owned());
    }
    if entry.local.is_empty() && entry.domain.is_empty() {
        return Err("it has no local or domain pattern".to_owned());
    }
    if let Some(reason) = &entry.reason
        && !REASONS.contains(&reason.as_str())
    {
        return Err(format!(
            "reason {reason:?} is not one of {}",
            REASONS.join(", ")
        ));
    }
    if entry.reason.is_none() && entry.source.is_some() {
        return Err("a source names why mail is filed, and it has no reason".to_owned());
    }
    if entry
        .source
        .as_deref()
        .is_some_and(|source| source.trim().is_empty())
    {
        return Err("its source is empty".to_owned());
    }
    let local = entry
        .local
        .iter()
        .map(|pattern| local_words(pattern))
        .collect::<Result<Vec<_>, _>>()?;
    let domain = entry
        .domain
        .iter()
        .map(|label| domain_label(label))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Row {
        name: entry.name,
        local,
        domain,
        reason: entry.reason,
        source: entry.source,
    })
}

pub const SEPARATORS: [char; 4] = ['.', '-', '_', '+'];

fn local_words(pattern: &str) -> Result<Vec<String>, String> {
    let words: Vec<String> = pattern
        .split(SEPARATORS)
        .map(str::to_ascii_lowercase)
        .collect();
    let well_formed = words
        .iter()
        .all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric()));
    if well_formed {
        Ok(words)
    } else {
        Err(format!(
            "local pattern {pattern:?} is not letters and digits parted by . - _ or +"
        ))
    }
}

fn domain_label(label: &str) -> Result<String, String> {
    let well_formed = !label.is_empty()
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if well_formed {
        Ok(label.to_ascii_lowercase())
    } else {
        Err(format!(
            "domain pattern {label:?} is not one label of letters, digits and hyphens"
        ))
    }
}
