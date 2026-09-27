//! Parses the automated-senders table, `data/senders.toml`, into rows.
//!
//! `build.rs` compiles this file a second time through `#[path]`, so the
//! parser that validates the shipped table at build time is the parser the
//! classifier loads it with: there is no second copy to drift (the idiom
//! `postio-account` uses for its provider presets). For that reason it
//! names nothing else in the crate, only `serde`, `toml` and `std`, and its
//! reasons are strings that [`super`] turns into the crate's own type.

use std::fmt;

use serde::Deserialize;

/// FR-113's vocabulary, spelled as the table and the store spell it.
pub const REASONS: &[&str] = &[
    "spam",
    "promotion",
    "notification",
    "receipt",
    "shipping",
    "social",
];

/// One `[[sender]]` entry, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// Each local-part pattern, as its lowercase words.
    pub local: Vec<Vec<String>>,
    /// Each subdomain label, lowercase.
    pub domain: Vec<String>,
    /// One of [`REASONS`].
    pub reason: String,
    pub source: Option<String>,
}

/// Why the table does not load: which entry, when one is to blame, and what
/// is wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    /// The entry's name, or its position when it has none; `None` when the
    /// file itself is not a table.
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

/// The file: a list of `[[sender]]` tables and nothing else.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    sender: Vec<toml::Table>,
}

/// One `[[sender]]` table, before it is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    #[serde(default)]
    local: Vec<String>,
    #[serde(default)]
    domain: Vec<String>,
    reason: String,
    source: Option<String>,
}

/// The table's rows, in file order, or the first thing wrong with it. An
/// entry is named by its `name`, or by its position when it has none, so
/// the error says where to look without quoting anything else.
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

/// An entry, checked: a name, a pattern at least, patterns that are the
/// right shape, and a reason from the vocabulary.
fn row(entry: Entry) -> Result<Row, String> {
    if entry.name.trim().is_empty() {
        return Err("its name is empty".to_owned());
    }
    if entry.local.is_empty() && entry.domain.is_empty() {
        return Err("it has no local or domain pattern".to_owned());
    }
    if !REASONS.contains(&entry.reason.as_str()) {
        return Err(format!(
            "reason {:?} is not one of {}",
            entry.reason,
            REASONS.join(", ")
        ));
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

/// The characters that part a local part's words.
pub const SEPARATORS: [char; 4] = ['.', '-', '_', '+'];

/// A local-part pattern as its words: letters and digits, parted by `.`,
/// `-`, `_` or `+`. The part before the at sign, never a whole address.
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

/// A domain pattern: one label of a subdomain, letters, digits and inner
/// hyphens.
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
