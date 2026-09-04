//! Human-readable validation, and the one-line validity indicator.
//!
//! Canvas 3f has no OK/Cancel dialog for settings: `config.toml` *is* the
//! settings UI, edits apply live, and a single always-visible line says either
//!
//! ```text
//! valid · parsed in 2 ms
//! line 12 · `ctrl+` ends with a modifier and no key
//! ```
//!
//! Everything in this module serves that line. Errors therefore carry a
//! position ([`ValidationError::line`], [`ValidationError::column`]) and prose
//! a person can act on without knowing serde. Nothing here ever quotes a value
//! from a secret-bearing key — see [`crate::secrets`].
//!
//! Validation is deliberately *not* fatal. [`check_str`] reports as much as it
//! can about a file it cannot fully load, so the app can keep running on the
//! last good configuration while the line explains what is wrong.
//!
//! ```
//! use postio_config::validate;
//!
//! let checked = validate::check_str("[keys]\nreply = \"ctrl+\"\n");
//! assert!(!checked.validation.is_valid());
//! assert_eq!(checked.validation.first_error().unwrap().line, 2);
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

use postio_model::MailboxRole;
use postio_model::rule::RuleError;
use toml::{Table, Value};

use crate::source::SourceMap;
use crate::{Config, keys, secrets};

/// What kind of problem an error describes.
///
/// The first two mean the file could not be loaded at all, so they are shown
/// ahead of merely questionable settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The file is not valid TOML.
    Syntax,
    /// The file is valid TOML but does not fit the schema — an unknown enum
    /// value, a string where a number belongs.
    Schema,
    /// The file loads, but the settings do not make sense together.
    Semantic,
    /// The file could not be read.
    Io,
}

impl ErrorKind {
    /// Whether this problem stops the configuration from loading.
    pub fn is_blocking(self) -> bool {
        !matches!(self, ErrorKind::Semantic)
    }
}

/// One problem with the configuration, positioned in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// What kind of problem this is.
    pub kind: ErrorKind,
    /// Dotted path of the setting at fault, e.g. `accounts.personal.imap.host`.
    pub path: String,
    /// One-based line in `config.toml`.
    pub line: usize,
    /// One-based column, counted in characters.
    pub column: usize,
    /// Plain-language explanation, short enough for the validity line.
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// The outcome of validating a configuration.
#[derive(Debug, Clone, Default)]
pub struct Validation {
    errors: Vec<ValidationError>,
    elapsed: Duration,
}

impl Validation {
    /// Every problem found, blocking ones first and otherwise reading down the
    /// file.
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Whether the configuration is usable as written.
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// The problem the validity line shows.
    pub fn first_error(&self) -> Option<&ValidationError> {
        self.errors.first()
    }

    /// How long parsing and validating took — the design shows this.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// `valid`, or the first error as `line 12: …`.
    pub fn status(&self) -> String {
        match self.first_error() {
            None => "valid".to_string(),
            Some(err) => err.to_string(),
        }
    }

    /// The whole validity line: [`Validation::status`] plus the parse timing.
    pub fn status_line(&self) -> String {
        format!("{} · parsed in {}", self.status(), self.timing())
    }

    /// Just the timing phrase, e.g. `2 ms`.
    pub fn timing(&self) -> String {
        let millis = self.elapsed.as_millis();
        if millis == 0 {
            "<1 ms".to_string()
        } else {
            format!("{millis} ms")
        }
    }
}

/// A parsed configuration and what is wrong with it.
///
/// `config` is `None` when the file could not be loaded at all; in that case
/// the caller keeps whatever configuration it already had (see
/// [`crate::live::LiveConfig`]).
#[derive(Debug, Clone)]
pub struct Checked {
    /// The configuration, when the file loaded cleanly.
    pub config: Option<Config>,
    /// What is wrong with it.
    pub validation: Validation,
}

/// Parse and validate a TOML document.
pub fn check_str(text: &str) -> Checked {
    let started = Instant::now();
    let mut errors = Vec::new();
    let config = check_text(text, &mut errors);
    finish(config, errors, started)
}

/// Parse and validate `config.toml` at `path`.
///
/// A missing file is not a problem: it yields valid defaults, because first run
/// needs nothing on disk.
pub fn check_path(path: &Path) -> Checked {
    let started = Instant::now();
    match std::fs::read_to_string(path) {
        Ok(text) => check_str(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            finish(Some(Config::default()), Vec::new(), started)
        }
        Err(err) => finish(
            None,
            vec![ValidationError {
                kind: ErrorKind::Io,
                path: String::new(),
                line: 1,
                column: 1,
                message: format!("cannot read {}: {err}", path.display()),
            }],
            started,
        ),
    }
}

fn finish(config: Option<Config>, mut errors: Vec<ValidationError>, started: Instant) -> Checked {
    errors.sort_by_key(|err| (!err.kind.is_blocking(), err.line, err.column));
    Checked {
        config: if errors.iter().any(|e| e.kind.is_blocking()) {
            None
        } else {
            config
        },
        validation: Validation {
            errors,
            elapsed: started.elapsed(),
        },
    }
}

/// Everything between reading the text and reporting on it.
///
/// Returns the configuration if it loaded, even when semantic errors were
/// found — a half-wrong file is still worth showing the rest of.
fn check_text(text: &str, errors: &mut Vec<ValidationError>) -> Option<Config> {
    let map = match SourceMap::parse(text) {
        Ok(map) => map,
        Err(err) => {
            errors.push(syntax_error(text, &err));
            return None;
        }
    };

    check_secrets(&map, errors);
    check_retired_accounts(&map, errors);

    let config = match Config::parse_raw(text) {
        Ok(config) => Some(config),
        Err(err) => {
            errors.push(schema_error(&map, &err));
            recover(text, &map, errors)
        }
    };

    if let Some(config) = &config {
        check_keys(config, &map, errors);
        check_sync(config, &map, errors);
        check_filters(config, &map, errors);
        check_rules(config, &map, errors);
        check_mailboxes(config, &map, errors);
    }
    config
}

/// Re-parse with the offending keys removed, so one bad value does not hide
/// every other problem in the file.
///
/// The result is only ever used to produce further errors: `check_str` reports
/// `config: None` whenever anything blocking was found.
fn recover(text: &str, map: &SourceMap, errors: &mut Vec<ValidationError>) -> Option<Config> {
    let mut table: Table = toml::from_str(text).ok()?;
    secrets::strip_secrets(&mut table);
    for _ in 0..16 {
        match Config::from_table(table.clone()) {
            Ok(config) => return Some(config),
            Err(err) => {
                let path = key_path(&err)?;
                if !remove_path(&mut table, &path) {
                    return None;
                }
                errors.push(schema_error(map, &err));
            }
        }
    }
    None
}

// ------------------------------------------------------------ toml errors --

fn syntax_error(text: &str, err: &toml::de::Error) -> ValidationError {
    let lines = crate::source::LineIndex::new(text);
    let (line, column) = err.span().map_or((1, 1), |span| lines.at(text, span.start));
    ValidationError {
        kind: ErrorKind::Syntax,
        path: String::new(),
        line,
        column,
        message: one_line(err.message()),
    }
}

fn schema_error(map: &SourceMap, err: &toml::de::Error) -> ValidationError {
    let path = key_path(err).unwrap_or_default();
    let (line, column) = match err.span() {
        Some(span) => map.at(span.start),
        None if !path.is_empty() => map.locate_value(&path),
        None => (1, 1),
    };
    ValidationError {
        kind: ErrorKind::Schema,
        path,
        line,
        column,
        message: one_line(err.message()),
    }
}

/// The dotted key a typed error happened under.
///
/// `toml` renders it as a trailing ``in `a.b.c` `` line when the error carries
/// no source snippet, which is the case for everything deserialized from an
/// already-parsed table.
fn key_path(err: &toml::de::Error) -> Option<String> {
    err.to_string()
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("in `")?.strip_suffix('`'))
        .map(str::to_string)
}

fn remove_path(table: &mut Table, path: &str) -> bool {
    let mut segments: Vec<&str> = path.split('.').collect();
    let Some(last) = segments.pop() else {
        return false;
    };
    let mut current = table;
    for segment in segments {
        match current.get_mut(segment) {
            Some(Value::Table(nested)) => current = nested,
            _ => return false,
        }
    }
    current.remove(last).is_some()
}

/// Collapse a parser message to one redacted line fit for the validity line.
fn one_line(message: &str) -> String {
    secrets::redact_secret_lines(message)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

// ------------------------------------------------------------ the checks --

fn push(
    errors: &mut Vec<ValidationError>,
    map: &SourceMap,
    path: String,
    at_value: bool,
    message: String,
) {
    let (line, column) = if at_value {
        map.locate_value(&path)
    } else {
        map.locate_key(&path)
    };
    errors.push(ValidationError {
        kind: ErrorKind::Semantic,
        path,
        line,
        column,
        message,
    });
}

/// `[accounts.<id>]` parses and is not read by anything (#470).
///
/// It described a real-looking editing path for an existing account's host,
/// port, security and display name. `postio-app` takes all of those from
/// SQLite, written once by onboarding, so editing the section saved,
/// re-parsed without complaint, and changed nothing about the account that
/// was running. A schema that round-trips and does nothing is worse than no
/// schema, because it looks editable.
///
/// Reported rather than rejected: `Semantic`, so the file still loads and
/// every other setting in it still applies. The section is also preserved on
/// round-trip like any unknown key, so retiring it does not eat what is
/// already in somebody's file.
fn check_retired_accounts(map: &SourceMap, errors: &mut Vec<ValidationError>) {
    let mut seen: Vec<&str> = Vec::new();
    for path in map.paths() {
        let Some(rest) = path.strip_prefix("accounts.") else {
            continue;
        };
        // One report per account table, at the table, not one per leaf key.
        let id = rest.split('.').next().unwrap_or(rest);
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let table = format!("accounts.{id}");
        let (line, column) = map.locate_key(&table);
        errors.push(ValidationError {
            kind: ErrorKind::Semantic,
            path: table.clone(),
            line,
            column,
            message: format!("`[{table}]` is ignored — accounts are managed in the list above"),
        });
    }
}

/// A secret in the file is a problem to fix, never a value to echo back.
fn check_secrets(map: &SourceMap, errors: &mut Vec<ValidationError>) {
    for path in map.paths() {
        let leaf = path.rsplit('.').next().unwrap_or(path);
        if !secrets::is_secret_key(leaf) {
            continue;
        }
        let (line, column) = map.locate_key(path);
        errors.push(ValidationError {
            kind: ErrorKind::Semantic,
            path: path.to_string(),
            line,
            column,
            message: format!(
                "`{leaf}` cannot be stored in config.toml; move it into the Secret Service keyring and reference it with `keyring_entry`"
            ),
        });
    }
}

fn check_keys(config: &Config, map: &SourceMap, errors: &mut Vec<ValidationError>) {
    for (command, binding) in config.keys.overrides() {
        if let Some(problem) = keys::binding_problem(binding) {
            push(
                errors,
                map,
                format!("keys.{command}"),
                false,
                format!("the binding for `{command}` is not usable: {problem}"),
            );
        }
    }

    let resolved = config.keys.resolved();
    let mut by_binding: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (command, binding) in &resolved {
        by_binding
            .entry(binding.trim())
            .or_default()
            .push(command.as_str());
    }
    for (binding, commands) in by_binding {
        if commands.len() < 2 {
            continue;
        }
        // Report against the override that caused it, which is the one the user
        // can see and fix.
        let path = commands
            .iter()
            .map(|command| format!("keys.{command}"))
            .filter(|path| map.key_offset(path).is_some())
            .max_by_key(|path| map.key_offset(path).unwrap_or(0))
            .unwrap_or_else(|| format!("keys.{}", commands[0]));
        push(
            errors,
            map,
            path,
            false,
            format!(
                "`{binding}` is bound to {} {}",
                if commands.len() == 2 {
                    "both"
                } else {
                    "all of"
                },
                and_list(&commands)
            ),
        );
    }
}

fn check_sync(config: &Config, map: &SourceMap, errors: &mut Vec<ValidationError>) {
    if config.sync.poll_interval_secs == 0 {
        push(
            errors,
            map,
            "sync.poll_interval_secs".to_string(),
            true,
            "`poll_interval_secs` cannot be 0; give it a number of seconds".to_string(),
        );
    }
    if config.sync.max_connections == 0 {
        push(
            errors,
            map,
            "sync.max_connections".to_string(),
            true,
            "`max_connections` cannot be 0; Postio needs at least one connection".to_string(),
        );
    }
    if config.sync.initial_sync_messages == 0 {
        push(
            errors,
            map,
            "sync.initial_sync_messages".to_string(),
            true,
            "`initial_sync_messages` cannot be 0; the first sync would fetch nothing".to_string(),
        );
    }
}

fn check_mailboxes(config: &Config, map: &SourceMap, errors: &mut Vec<ValidationError>) {
    for (key, path) in &config.mailboxes {
        let Some(role) = MailboxRole::from_name(key) else {
            push(
                errors,
                map,
                format!("mailboxes.{key}"),
                false,
                format!(
                    "`{key}` is not a mailbox role. Expected one of {}.",
                    and_list(&["archive", "sent", "drafts", "trash", "junk", "flagged"])
                ),
            );
            continue;
        };
        if !crate::overridable(role) {
            push(
                errors,
                map,
                format!("mailboxes.{key}"),
                false,
                format!(
                    "`{key}` cannot be pointed at another folder. IMAP names \
                     the inbox itself, and an ordinary folder is what a folder \
                     is without a role."
                ),
            );
            continue;
        }
        if path.trim().is_empty() {
            push(
                errors,
                map,
                format!("mailboxes.{key}"),
                false,
                format!("`{key}` names no folder"),
            );
        }
    }
}

fn check_filters(config: &Config, map: &SourceMap, errors: &mut Vec<ValidationError>) {
    for (name, filter) in &config.filters {
        if filter.query.trim().is_empty() {
            push(
                errors,
                map,
                format!("filters.{name}.query"),
                false,
                format!("filter `{name}` has an empty query"),
            );
        }
    }
}

/// `[[rules]]`, one entry at a time (ADR 0008 Q4).
///
/// Every entry is reported independently, because the point of the array is
/// that the rules are separate things: one broken rule must not hide the six
/// under it, and a caller may still run the ones that are fine.
///
/// The message reads `rule `x` <what is wrong>`, which is why
/// `postio_model::rule::RuleError`'s own text is phrased as a clause. That
/// line is all a user gets — canvas 3f has no dialog, only the validity
/// line — so it has to name the rule, the thing that is wrong, and where
/// possible what to write instead.
fn check_rules(config: &Config, map: &SourceMap, errors: &mut Vec<ValidationError>) {
    for (index, (entry, parsed)) in config.rules.iter().zip(config.rules()).enumerate() {
        let label = if entry.name.trim().is_empty() {
            format!("rule {}", index + 1)
        } else {
            format!("rule `{}`", entry.name.trim())
        };

        // A nameless rule parses perfectly and is unreportable afterwards:
        // Attention, the settings panel and every log line about it can only
        // say "rule 3", which is a different rule the moment one is inserted
        // above it.
        if entry.name.trim().is_empty() {
            push(
                errors,
                map,
                format!("rules[{index}].name"),
                false,
                format!("{label} has no name, so nothing can say which rule acted"),
            );
        }

        let Err(problem) = parsed else {
            continue;
        };
        // Point at the key the fix goes in, so the validity line's cursor
        // lands where the user has to type.
        let key = match &problem {
            RuleError::NoQuery => String::new(),
            RuleError::QueryAndFilter { .. }
            | RuleError::UnknownFilter { .. }
            | RuleError::FilterHasNoQuery { .. } => ".filter".to_owned(),
            RuleError::NoActions
            | RuleError::UnknownAction { .. }
            | RuleError::ActionNeedsArgument { .. }
            | RuleError::ActionTakesNoArgument { .. }
            | RuleError::ForwardNeedsAnAddress { .. }
            | RuleError::StopIsARuleKey
            | RuleError::DeleteIsNotAnAction => ".actions".to_owned(),
        };
        push(
            errors,
            map,
            format!("rules[{index}]{key}"),
            false,
            format!("{label} {problem}"),
        );
    }
}

fn and_list(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("`{item}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, head)) => format!("{} and {last}", head.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_read_like_prose() {
        assert_eq!(and_list(&["a"]), "`a`");
        assert_eq!(and_list(&["a", "b"]), "`a` and `b`");
        assert_eq!(and_list(&["a", "b", "c"]), "`a`, `b` and `c`");
    }

    #[test]
    fn a_typed_error_names_the_key_it_happened_under() {
        let err = Config::parse_raw("[ui]\ndensity = \"enormous\"\n").unwrap_err();
        assert_eq!(key_path(&err).as_deref(), Some("ui.density"));
    }

    #[test]
    fn removing_a_dotted_path_reaches_into_tables() {
        let mut table: Table = toml::from_str("[ui]\ndensity = \"x\"\n").unwrap();
        assert!(remove_path(&mut table, "ui.density"));
        assert!(!remove_path(&mut table, "ui.density"));
        assert!(!remove_path(&mut table, "nope.nothing"));
    }

    #[test]
    fn messages_are_collapsed_to_one_redacted_line() {
        let out = one_line("first line\n\npassword = \"hunter2\"");
        assert!(!out.contains('\n'), "{out}");
        assert!(!out.contains("hunter2"), "{out}");
    }
}
