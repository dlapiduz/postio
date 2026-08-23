//! Mapping a dotted config key back to where it sits in the file.
//!
//! The validity line in the settings panel is one line long: `line 12 · "ctrl+"
//! names a modifier but no key`. To say *line 12* the validation pass needs the
//! position of every key in the document, which typed deserialization throws
//! away. [`SourceMap`] keeps it, built from the spanned parse tree that the
//! `toml` crate exposes through `DeTable`.

use std::collections::BTreeMap;

use toml::de::{DeTable, DeValue};

/// Byte offsets of the start of every line, for offset -> line/column.
#[derive(Debug)]
pub(crate) struct LineIndex {
    starts: Vec<usize>,
    text_len: usize,
}

impl LineIndex {
    pub(crate) fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i + 1),
        );
        Self {
            starts,
            text_len: text.len(),
        }
    }

    /// One-based line and column of a byte offset. The column counts
    /// characters, so a key after a multi-byte character still points at it.
    pub(crate) fn at(&self, text: &str, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text_len);
        let line = match self.starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index - 1,
        };
        let start = self.starts[line];
        let column = text
            .get(start..offset)
            .map_or(0, |slice| slice.chars().count());
        (line + 1, column + 1)
    }
}

/// Where each key of a TOML document is written.
#[derive(Debug)]
pub(crate) struct SourceMap {
    /// Dotted path -> (offset of the key, offset of its value).
    entries: BTreeMap<String, (usize, usize)>,
    lines: LineIndex,
    text: String,
}

impl SourceMap {
    /// Parse `text` for positions only.
    ///
    /// Fails only when the document is not valid TOML, in which case the
    /// returned error carries the span of the syntax error.
    pub(crate) fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let table = DeTable::parse(text)?;
        let mut entries = BTreeMap::new();
        walk(table.get_ref(), "", &mut entries);
        Ok(Self {
            entries,
            lines: LineIndex::new(text),
            text: text.to_string(),
        })
    }

    /// One-based line and column of a byte offset.
    pub(crate) fn at(&self, offset: usize) -> (usize, usize) {
        self.lines.at(&self.text, offset)
    }

    /// Every dotted path in the document, in lexicographic order.
    pub(crate) fn paths(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Byte offset of a key, if the file sets it.
    pub(crate) fn key_offset(&self, path: &str) -> Option<usize> {
        self.entries.get(path).map(|(key, _)| *key)
    }

    /// Position of a key, falling back to its nearest written ancestor and
    /// finally to the top of the file.
    ///
    /// The fallback is what lets "this account has no email" point at the
    /// `[accounts.personal]` header: the key the user needs to add is not in the
    /// file yet, but the table it belongs to is.
    pub(crate) fn locate_key(&self, path: &str) -> (usize, usize) {
        self.locate(path, |(key, _)| *key)
    }

    /// Position of a key's *value* — what a "this value is wrong" error wants.
    pub(crate) fn locate_value(&self, path: &str) -> (usize, usize) {
        self.locate(path, |(_, value)| *value)
    }

    fn locate(&self, path: &str, pick: fn(&(usize, usize)) -> usize) -> (usize, usize) {
        let mut candidate = path;
        loop {
            if let Some(entry) = self.entries.get(candidate) {
                return self.at(pick(entry));
            }
            match candidate.rfind('.') {
                Some(dot) => candidate = &candidate[..dot],
                None => return (1, 1),
            }
        }
    }
}

fn walk(table: &DeTable<'_>, prefix: &str, out: &mut BTreeMap<String, (usize, usize)>) {
    for (key, value) in table.iter() {
        let path = if prefix.is_empty() {
            key.get_ref().to_string()
        } else {
            format!("{prefix}.{}", key.get_ref())
        };
        out.insert(path.clone(), (key.span().start, value.span().start));
        match value.get_ref() {
            DeValue::Table(nested) => walk(nested, &path, out),
            DeValue::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    let path = format!("{path}[{index}]");
                    out.insert(path.clone(), (item.span().start, item.span().start));
                    if let DeValue::Table(nested) = item.get_ref() {
                        walk(nested, &path, out);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "[ui]\ndensity = \"compact\"\n\n[accounts.personal.imap]\nhost = \"h\"\n";

    #[test]
    fn keys_and_values_get_separate_positions() {
        let map = SourceMap::parse(TEXT).unwrap();
        assert_eq!(map.locate_key("ui.density"), (2, 1));
        assert_eq!(map.locate_value("ui.density"), (2, 11));
    }

    #[test]
    fn a_nested_header_is_reachable_by_its_dotted_path() {
        let map = SourceMap::parse(TEXT).unwrap();
        assert_eq!(map.locate_key("accounts.personal.imap.host"), (5, 1));
        assert_eq!(map.locate_key("accounts.personal.imap").0, 4);
    }

    #[test]
    fn a_missing_key_falls_back_to_its_table() {
        let map = SourceMap::parse(TEXT).unwrap();
        assert_eq!(map.locate_key("accounts.personal.imap.port").0, 4);
        assert_eq!(map.locate_key("nothing.like.this"), (1, 1));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let map = SourceMap::parse("[ui]\ntheme = \"dark\" # café\nx = 1\n").unwrap();
        assert_eq!(map.locate_key("ui.x"), (3, 1));
    }

    #[test]
    fn a_syntax_error_carries_a_span() {
        let err = SourceMap::parse("[ui\nx = 1\n").unwrap_err();
        assert!(err.span().is_some());
    }
}
