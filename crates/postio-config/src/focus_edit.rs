//! Postio Focus's own writes to `config.toml` (spec 007, contracts/config.md):
//! a sender restored from Filtered, a marker kind stopped by repeated
//! dismissal, a digest rule made, edited, narrowed or removed.
//!
//! Each is a function from the file's text to its new text, touching only
//! the entry it is about, through `toml_edit`'s format-preserving document
//! model: every other line -- other sections, comments, layout -- survives,
//! as the settings panel's own writes promise ([`crate::patch_filters`]).
//! The caller writes the result with [`crate::save::write_atomically`], so
//! the change reaches every running surface through the watcher exactly as
//! `$EDITOR`'s would.
//!
//! Each answers `None` when the file already says what was asked, so a
//! caller writes nothing and records nothing to take back.

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

use crate::focus::DigestRule;
use crate::{ConfigError, Result};

/// `text` as a document, or the parse error with any secret redacted.
fn document(text: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>()
        .map_err(|err| ConfigError::parse(None, &err))
}

/// `doc` as text, for a file that said `text` before the edit.
///
/// A file of nothing but comments holds them as the document's trailing
/// text, which `toml_edit` writes after every table -- so a table added to
/// it would land above what the person wrote. Such a file keeps its text on
/// top, and the edit follows it.
fn render(text: &str, mut doc: DocumentMut) -> String {
    let had_nothing = text
        .parse::<DocumentMut>()
        .is_ok_and(|before| before.as_table().is_empty());
    if !had_nothing || text.trim().is_empty() {
        return doc.to_string();
    }
    doc.set_trailing("");
    let mut lead = text.to_owned();
    if !lead.ends_with('\n') {
        lead.push('\n');
    }
    format!("{lead}{doc}")
}

/// `[focus.filter]`, made when it is not there. `[focus]` is made implicit
/// when it has to be made at all, so a file that says nothing else of Focus
/// gains a `[focus.filter]` header and not an empty `[focus]` above it.
fn filter_table(doc: &mut DocumentMut) -> Result<&mut Table> {
    let focus = doc
        .as_table_mut()
        .entry("focus")
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| shape("[focus] is not a table"))?;
    focus
        .entry("filter")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| shape("[focus.filter] is not a table"))
}

/// The file says something in a shape Focus cannot edit around.
fn shape(what: &str) -> ConfigError {
    ConfigError::Serialize(format!(
        "{what}, so Postio will not write to it; fix that line and try again"
    ))
}

/// `key` of `table` as an array, made when it is not there.
fn array_at<'a>(table: &'a mut Table, key: &str) -> Result<&'a mut Array> {
    table
        .entry(key)
        .or_insert_with(|| Item::Value(Value::Array(Array::new())))
        .as_array_mut()
        .ok_or_else(|| shape(&format!("`{key}` is not a list")))
}

/// `[focus.filter] never` with `entry` in it, or without it: a sender
/// restored from Filtered, pinned so it is never filtered again (FR-116),
/// or that pin taken back. An entry is matched as the validator reads one,
/// ignoring case. `None` when the file already says so.
pub fn set_never(text: &str, entry: &str, present: bool) -> Result<Option<String>> {
    let mut doc = document(text)?;
    let filter = filter_table(&mut doc)?;
    let never = array_at(filter, "never")?;
    let at = never.iter().position(|value| {
        value
            .as_str()
            .is_some_and(|written| written.trim().eq_ignore_ascii_case(entry.trim()))
    });
    match (at, present) {
        (Some(_), true) | (None, false) => return Ok(None),
        (None, true) => never.push(entry.trim()),
        (Some(at), false) => {
            never.remove(at);
        }
    }
    Ok(Some(render(text, doc)))
}

/// `[focus] reading` set to `reading`: where `Return` opens a message, as
/// `toggle_reading_pane` chose it (T232). `None` when the file already says
/// so -- and a file that says nothing says `dialog`, the default.
pub fn set_reading(text: &str, reading: crate::Reading) -> Result<Option<String>> {
    let mut doc = document(text)?;
    let written = doc
        .get("focus")
        .and_then(|focus| focus.get("reading"))
        .and_then(Item::as_str)
        .map(str::to_owned);
    let says = written.as_deref().unwrap_or(crate::Reading::default().as_str());
    if says == reading.as_str() {
        return Ok(None);
    }
    let focus = doc
        .as_table_mut()
        .entry("focus")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| shape("[focus] is not a table"))?;
    // A section made implicit by `[focus.filter]` alone is written out now
    // that it holds a key of its own.
    focus.set_implicit(false);
    focus.insert("reading", toml_edit::value(reading.as_str()));
    Ok(Some(render(text, doc)))
}

/// `[focus.filter] stop_markers` with `{ sender, kind }` in it, or without
/// it: a marker kind the person stopped for a sender by dismissing it
/// again and again (FR-108), or that correction taken back. `None` when the
/// file already says so.
pub fn set_stop_marker(
    text: &str,
    sender: &str,
    kind: &str,
    present: bool,
) -> Result<Option<String>> {
    let mut doc = document(text)?;
    let filter = filter_table(&mut doc)?;
    let stops = array_at(filter, "stop_markers")?;
    let at = stops.iter().position(|value| {
        value.as_inline_table().is_some_and(|stop| {
            stop.get("sender")
                .and_then(Value::as_str)
                .is_some_and(|written| written.trim().eq_ignore_ascii_case(sender.trim()))
                && stop
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|written| written.trim().eq_ignore_ascii_case(kind))
        })
    });
    match (at, present) {
        (Some(_), true) | (None, false) => return Ok(None),
        (None, true) => {
            let mut stop = InlineTable::new();
            stop.insert("sender", Value::from(sender.trim()));
            stop.insert("kind", Value::from(kind));
            stops.push(Value::InlineTable(stop));
        }
        (Some(at), false) => {
            stops.remove(at);
        }
    }
    Ok(Some(render(text, doc)))
}

/// `[[focus.digests]]`, made when it is not there.
fn digests(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables> {
    let focus = doc
        .as_table_mut()
        .entry("focus")
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| shape("[focus] is not a table"))?;
    focus
        .entry("digests")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| shape("`focus.digests` is not a list of `[[focus.digests]]` rules"))
}

/// Where the rule called `name` is in the file, by its name as written.
fn position_of(rules: &ArrayOfTables, name: &str) -> Option<usize> {
    rules.iter().position(|rule| {
        rule.get("name")
            .and_then(Item::as_str)
            .is_some_and(|written| written.trim() == name.trim())
    })
}

/// `rule` as the table `[[focus.digests]]` holds.
fn rule_table(rule: &DigestRule) -> Result<Table> {
    let text = toml::to_string(rule).map_err(|err| ConfigError::Serialize(err.to_string()))?;
    Ok(document(&text)?.as_table().clone())
}

/// The rule the file holds called `name`, and where, if it holds one.
pub fn digest_rule(text: &str, name: &str) -> Result<Option<(usize, DigestRule)>> {
    let doc = document(text)?;
    let Some(rules) = doc
        .get("focus")
        .and_then(|focus| focus.get("digests"))
        .and_then(Item::as_array_of_tables)
    else {
        return Ok(None);
    };
    let Some(at) = position_of(rules, name) else {
        return Ok(None);
    };
    let table = rules.get(at).expect("the position is in the list");
    let rule: DigestRule =
        toml::from_str(&table.to_string()).map_err(|err| ConfigError::parse(None, &err))?;
    Ok(Some((at, rule)))
}

/// `[[focus.digests]]` with `rule` in it: a new rule at the end, or, with
/// `replacing`, the rule of that name rewritten where it stands -- or put
/// at `at` when there is none of that name, as undo puts back a rule it
/// took away. The file's order is the order rules are matched in, so a
/// rule keeps its place.
pub fn put_digest_rule(
    text: &str,
    replacing: Option<&str>,
    rule: &DigestRule,
    at: Option<usize>,
) -> Result<String> {
    let mut doc = document(text)?;
    let table = rule_table(rule)?;
    let rules = digests(&mut doc)?;
    let existing = replacing.and_then(|name| position_of(rules, name));
    match existing {
        Some(position) => {
            *rules
                .get_mut(position)
                .expect("the position is in the list") = table;
        }
        None => match at.filter(|at| *at < rules.len()) {
            // `ArrayOfTables` has no insert, so the list is rebuilt around
            // the rule; the other rules' own text comes across unchanged.
            Some(at) => {
                let mut rebuilt = ArrayOfTables::new();
                for (index, existing) in rules.iter().enumerate() {
                    if index == at {
                        rebuilt.push(table.clone());
                    }
                    rebuilt.push(existing.clone());
                }
                *rules = rebuilt;
            }
            None => rules.push(table),
        },
    }
    Ok(render(text, doc))
}

/// `[[focus.digests]]` without the rule called `name`, and the rule as it
/// stood and where: what removing it at `g d` takes away (FR-126), and what
/// undo would need to put it back. `None` when there is no such rule.
pub fn remove_digest_rule(text: &str, name: &str) -> Result<Option<(String, usize, DigestRule)>> {
    let Some((at, rule)) = digest_rule(text, name)? else {
        return Ok(None);
    };
    let mut doc = document(text)?;
    let rules = digests(&mut doc)?;
    rules.remove(at);
    if rules.is_empty()
        && let Some(focus) = doc.get_mut("focus").and_then(Item::as_table_mut)
    {
        focus.remove("digests");
    }
    Ok(Some((doc.to_string(), at, rule)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    const FILE: &str = "\
# My settings.
[ui]
density = \"compact\" # as I like it

[focus]
filtering = true

[[focus.digests]]
name = \"Newsletters\"
match = [\"from:news@ledger.example\", \"from:editor@ledger.example\"]
cadence = \"weekly\"
day = \"saturday\"
at = \"16:00\"
";

    #[test]
    fn a_restored_sender_is_pinned_and_nothing_else_moves() {
        let written = set_never(FILE, "ada@example.org", true)
            .expect("an edit")
            .expect("a change");
        assert!(
            written.starts_with("# My settings.\n[ui]\ndensity = \"compact\" # as I like it\n")
        );
        let config = Config::from_toml_str(&written).expect("it reads");
        assert_eq!(config.focus.filter.never, vec!["ada@example.org"]);
        assert_eq!(config.focus.digests.len(), 1, "the rule is untouched");

        assert_eq!(
            set_never(&written, "ADA@example.org", true).expect("an edit"),
            None,
            "pinned already, in any case"
        );
        let unpinned = set_never(&written, "ada@example.org", false)
            .expect("an edit")
            .expect("a change");
        assert!(
            Config::from_toml_str(&unpinned)
                .expect("it reads")
                .focus
                .filter
                .never
                .is_empty()
        );
    }

    #[test]
    fn reading_is_the_dialog_unless_the_file_says_pane() {
        use crate::Reading;
        assert_eq!(
            Config::from_toml_str("").expect("it reads").focus.reading,
            Reading::Dialog
        );
        let pane = Config::from_toml_str("[focus]\nreading = \"pane\"\n").expect("it reads");
        assert_eq!(pane.focus.reading, Reading::Pane);
    }

    #[test]
    fn reading_is_written_beside_filtering_and_nothing_else_moves() {
        use crate::Reading;
        let written = set_reading(FILE, Reading::Pane)
            .expect("an edit")
            .expect("a change");
        assert!(
            written.starts_with("# My settings.\n[ui]\ndensity = \"compact\" # as I like it\n")
        );
        assert!(written.contains("[focus]\nfiltering = true\nreading = \"pane\"\n"));
        let config = Config::from_toml_str(&written).expect("it reads");
        assert_eq!(config.focus.reading, Reading::Pane);
        assert_eq!(config.focus.digests.len(), 1, "the rule is untouched");
        assert_eq!(set_reading(&written, Reading::Pane).expect("an edit"), None);
        let back = set_reading(&written, Reading::Dialog)
            .expect("an edit")
            .expect("a change");
        assert_eq!(
            Config::from_toml_str(&back).expect("it reads").focus.reading,
            Reading::Dialog
        );
        // A file that says nothing already says the dialog.
        assert_eq!(set_reading("", Reading::Dialog).expect("an edit"), None);
        assert_eq!(
            set_reading("", Reading::Pane).expect("an edit").as_deref(),
            Some("[focus]\nreading = \"pane\"\n")
        );
    }

    #[test]
    fn a_file_with_no_focus_section_gains_only_the_filter_table() {
        let written = set_never("[ui]\ndensity = \"compact\"\n", "@example.net", true)
            .expect("an edit")
            .expect("a change");
        assert_eq!(
            written,
            "[ui]\ndensity = \"compact\"\n\n[focus.filter]\nnever = [\"@example.net\"]\n"
        );
    }

    #[test]
    fn a_stopped_marker_kind_is_written_and_taken_back() {
        let written = set_stop_marker(FILE, "news@ledger.example", "question", true)
            .expect("an edit")
            .expect("a change");
        let config = Config::from_toml_str(&written).expect("it reads");
        assert_eq!(
            config.focus.filter.stop_markers,
            vec![crate::focus::StopMarker {
                sender: "news@ledger.example".to_owned(),
                kind: "question".to_owned(),
            }]
        );
        assert_eq!(
            set_stop_marker(&written, "news@ledger.example", "question", true).expect("an edit"),
            None
        );
        assert_eq!(
            set_stop_marker(&written, "news@ledger.example", "todo", false).expect("an edit"),
            None,
            "another kind was never stopped"
        );
        let taken_back = set_stop_marker(&written, "news@ledger.example", "question", false)
            .expect("an edit")
            .expect("a change");
        assert!(
            Config::from_toml_str(&taken_back)
                .expect("it reads")
                .focus
                .filter
                .stop_markers
                .is_empty()
        );
    }

    fn weekly(name: &str, queries: &[&str]) -> DigestRule {
        DigestRule {
            name: name.to_owned(),
            queries: queries.iter().map(|query| (*query).to_owned()).collect(),
            cadence: "weekly".to_owned(),
            day: Some(toml::Value::String("sunday".to_owned())),
            at: "09:00".to_owned(),
            extras: Default::default(),
        }
    }

    #[test]
    fn a_rule_added_to_a_file_of_only_a_comment_keeps_the_comment_on_top() {
        let made = put_digest_rule(
            "# Mine.\n",
            None,
            &weekly("Receipts", &["from:shop@example.com"]),
            None,
        )
        .expect("an edit");
        assert!(made.starts_with("# Mine.\n"), "{made}");
    }

    #[test]
    fn a_rule_is_made_edited_in_its_place_and_removed() {
        let made = put_digest_rule(
            FILE,
            None,
            &weekly("Receipts", &["from:shop@example.com"]),
            None,
        )
        .expect("an edit");
        let names = |text: &str| -> Vec<String> {
            Config::from_toml_str(text)
                .expect("it reads")
                .focus
                .digests
                .into_iter()
                .map(|rule| rule.name)
                .collect()
        };
        assert_eq!(names(&made), ["Newsletters", "Receipts"]);
        assert!(made.contains("density = \"compact\" # as I like it"));

        let edited = put_digest_rule(
            &made,
            Some("Newsletters"),
            &weekly("Letters", &["from:news@ledger.example"]),
            None,
        )
        .expect("an edit");
        assert_eq!(
            names(&edited),
            ["Letters", "Receipts"],
            "edited where it stands"
        );

        let (removed, at, rule) = remove_digest_rule(&edited, "Letters")
            .expect("an edit")
            .expect("the rule was there");
        assert_eq!(names(&removed), ["Receipts"]);
        assert_eq!(at, 0);
        assert_eq!(rule.queries, ["from:news@ledger.example"]);

        let put_back =
            put_digest_rule(&removed, Some("Letters"), &rule, Some(at)).expect("an edit");
        assert_eq!(
            names(&put_back),
            ["Letters", "Receipts"],
            "back in its place"
        );
        assert_eq!(
            remove_digest_rule(&put_back, "Nobody").expect("a read"),
            None
        );
    }
}
