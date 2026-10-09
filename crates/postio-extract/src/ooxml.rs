//! Office Open XML: a zip of XML parts, walked with `quick-xml`'s
//! streaming reader (research R5).
//!
//! Three small walkers, one per format, each a loop over events with
//! counters for depth rather than recursion, so a document nested two
//! hundred thousand elements deep costs a counter, not a stack frame:
//!
//! * **DOCX** — `w:p` paragraphs, numbered from 1 outside tables
//!   ([`Location::Paragraph`]); each top-level `w:tbl` row is one unit
//!   ([`Location::Table`]).
//! * **XLSX** — the workbook's sheets in its own order, by name; each
//!   `row` one unit, its cells' values joined, shared strings resolved
//!   ([`Location::Sheet`]).
//! * **PPTX** — the deck's slides in `p:sldIdLst` order; each slide one
//!   unit ([`Location::Slide`]).
//!
//! Parts are found the way the format says to find them, through the
//! package's relationships, with the conventional path as the fallback.
//! Every entry is read through [`Guarded`], so a part that inflates past
//! the entry limit or compresses too well to be honest stops the walk.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{BufReader, Read, Seek};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use zip::ZipArchive;

use crate::Location;
use crate::limits::{Budget, Guarded, Stop};

/// Which package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Docx,
    Xlsx,
    Pptx,
}

/// How many XML events pass between two looks at the clock.
const CLOCK_EVERY: u32 = 4_096;

pub(crate) fn extract<R: Read + Seek>(
    reader: R,
    kind: Kind,
    budget: &mut Budget<'_>,
) -> Result<(), Stop> {
    let mut archive = ZipArchive::new(reader).map_err(|_| Stop::Failed)?;
    let fallback = match kind {
        Kind::Docx => "word/document.xml",
        Kind::Xlsx => "xl/workbook.xml",
        Kind::Pptx => "ppt/presentation.xml",
    };
    let main = package_main(&mut archive, budget)?.unwrap_or_else(|| fallback.to_owned());
    match kind {
        Kind::Docx => docx(&mut archive, &main, budget),
        Kind::Xlsx => xlsx(&mut archive, &main, budget),
        Kind::Pptx => pptx(&mut archive, &main, budget),
    }
}

// ---------------------------------------------------------------------------
// Reading a part.
// ---------------------------------------------------------------------------

type Xml<'z, R> = Reader<BufReader<Guarded<zip::read::ZipFile<'z, R>>>>;

/// Open `name` and hand its XML reader to `walk`. `Ok(None)` when the
/// package has no such part. A limit the entry's reader tripped is
/// reported as that limit, not as the malformed XML it surfaces as.
fn with_part<R: Read + Seek, T>(
    archive: &mut ZipArchive<R>,
    name: &str,
    budget: &mut Budget<'_>,
    walk: impl FnOnce(&mut Xml<'_, R>, &mut Budget<'_>) -> Result<T, Stop>,
) -> Result<Option<T>, Stop> {
    let Some(index) = archive.index_for_name(name) else {
        return Ok(None);
    };
    let file = archive.by_index(index).map_err(|_| Stop::Failed)?;
    let compressed = file.compressed_size();
    let guarded = Guarded::new(file, compressed, budget.limits, budget.deadline);
    let mut reader = Reader::from_reader(BufReader::new(guarded));
    let walked = walk(&mut reader, budget);
    let tripped = reader.get_ref().get_ref().tripped;
    match (walked, tripped) {
        (Err(Stop::Failed), Some(limit)) => Err(Stop::Limit(limit)),
        (walked, _) => walked.map(Some),
    }
}

/// The next event, with a look at the clock every [`CLOCK_EVERY`]. A
/// malformed document is [`Stop::Failed`].
fn next<'b, R: std::io::BufRead>(
    reader: &mut Reader<R>,
    buffer: &'b mut Vec<u8>,
    budget: &Budget<'_>,
    count: &mut u32,
) -> Result<Event<'b>, Stop> {
    *count = count.wrapping_add(1);
    if (*count).is_multiple_of(CLOCK_EVERY) {
        budget.check_time()?;
    }
    buffer.clear();
    reader.read_event_into(buffer).map_err(|_| Stop::Failed)
}

/// The text an event carries: character data, CDATA, or a reference
/// (`&amp;`, `&#233;`) resolved. `None` for everything else.
fn text_of(event: &Event<'_>) -> Option<Cow<'static, str>> {
    match event {
        Event::Text(text) => text.decode().ok().map(|t| Cow::Owned(t.into_owned())),
        Event::CData(data) => data.decode().ok().map(|t| Cow::Owned(t.into_owned())),
        Event::GeneralRef(reference) => {
            if reference.is_char_ref() {
                reference
                    .resolve_char_ref()
                    .ok()
                    .flatten()
                    .map(|c| Cow::Owned(c.to_string()))
            } else {
                let name = reference.decode().ok()?;
                quick_xml::escape::resolve_predefined_entity(&name).map(Cow::Borrowed)
            }
        }
        _ => None,
    }
}

/// An attribute's value by its local name, unescaped.
fn attribute(element: &BytesStart<'_>, local: &[u8]) -> Option<String> {
    element.attributes().flatten().find_map(|attribute| {
        (attribute.key.local_name().as_ref() == local).then(|| {
            let raw = String::from_utf8_lossy(&attribute.value).into_owned();
            quick_xml::escape::unescape(&raw)
                .map(Cow::into_owned)
                .unwrap_or(raw)
        })
    })
}

/// A relationship target, as a path inside the package: relative to the
/// directory of the part whose relationships named it, or absolute from
/// the package root.
fn resolve(base_dir: &str, target: &str) -> String {
    let joined = if let Some(absolute) = target.strip_prefix('/') {
        absolute.to_owned()
    } else if base_dir.is_empty() {
        target.to_owned()
    } else {
        format!("{base_dir}/{target}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn directory_of(part: &str) -> &str {
    part.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// A part's relationships: id → (type, resolved target).
fn relationships<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    part: &str,
    budget: &mut Budget<'_>,
) -> Result<HashMap<String, (String, String)>, Stop> {
    let dir = directory_of(part);
    let file = part.rsplit_once('/').map_or(part, |(_, file)| file);
    let rels = if dir.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{dir}/_rels/{file}.rels")
    };
    let found = with_part(archive, &rels, budget, |reader, budget| {
        let mut out = HashMap::new();
        let (mut buffer, mut count) = (Vec::new(), 0);
        loop {
            match next(reader, &mut buffer, budget, &mut count)? {
                Event::Start(element) | Event::Empty(element)
                    if element.local_name().as_ref() == b"Relationship" =>
                {
                    if let (Some(id), Some(kind), Some(target)) = (
                        attribute(&element, b"Id"),
                        attribute(&element, b"Type"),
                        attribute(&element, b"Target"),
                    ) {
                        out.insert(id, (kind, resolve(dir, &target)));
                    }
                }
                Event::Eof => return Ok(out),
                _ => {}
            }
        }
    })?;
    Ok(found.unwrap_or_default())
}

/// The package's main part, from its root relationships.
fn package_main<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    budget: &mut Budget<'_>,
) -> Result<Option<String>, Stop> {
    // The root relationships are "_rels/.rels": the part named "" in the
    // root directory.
    let rels = relationships(archive, "", budget)?;
    Ok(rels
        .into_values()
        .find(|(kind, _)| kind.ends_with("/officeDocument"))
        .map(|(_, target)| target))
}

// ---------------------------------------------------------------------------
// DOCX.
// ---------------------------------------------------------------------------

fn docx<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    main: &str,
    budget: &mut Budget<'_>,
) -> Result<(), Stop> {
    let walked = with_part(archive, main, budget, |reader, budget| {
        let (mut buffer, mut count) = (Vec::new(), 0);
        let mut text = String::new();
        let mut row = String::new();
        let (mut paragraphs, mut tables, mut rows) = (0u32, 0u32, 0u32);
        let (mut paragraph_depth, mut table_depth, mut in_text) = (0u32, 0u32, 0u32);
        loop {
            let event = next(reader, &mut buffer, budget, &mut count)?;
            match &event {
                Event::Start(element) => match element.local_name().as_ref() {
                    b"p" => paragraph_depth += 1,
                    b"t" => in_text += 1,
                    b"tbl" => {
                        table_depth += 1;
                        if table_depth == 1 {
                            tables += 1;
                            rows = 0;
                        }
                    }
                    b"tr" if table_depth == 1 => row.clear(),
                    _ => {}
                },
                Event::Empty(element) => match element.local_name().as_ref() {
                    b"tab" if in_text == 0 => text.push('\t'),
                    b"br" | b"cr" => text.push(' '),
                    b"p" if table_depth == 0 && paragraph_depth == 0 => paragraphs += 1,
                    _ => {}
                },
                Event::End(element) => match element.local_name().as_ref() {
                    b"t" => in_text = in_text.saturating_sub(1),
                    b"p" => {
                        paragraph_depth = paragraph_depth.saturating_sub(1);
                        if paragraph_depth == 0 {
                            if table_depth == 0 {
                                paragraphs += 1;
                                budget.push(Location::Paragraph(paragraphs), &text)?;
                            } else {
                                row.push(' ');
                                row.push_str(&text);
                            }
                            text.clear();
                        } else {
                            text.push(' ');
                        }
                    }
                    b"tr" if table_depth == 1 => {
                        rows += 1;
                        budget.push(
                            Location::Table {
                                index: tables,
                                row: rows,
                            },
                            &row,
                        )?;
                        row.clear();
                    }
                    b"tbl" => table_depth = table_depth.saturating_sub(1),
                    _ => {}
                },
                Event::Eof => return Ok(()),
                other if in_text > 0 => {
                    if let Some(piece) = text_of(other) {
                        text.push_str(&piece);
                    }
                }
                _ => {}
            }
        }
    })?;
    walked.ok_or(Stop::Failed)
}

// ---------------------------------------------------------------------------
// XLSX.
// ---------------------------------------------------------------------------

fn xlsx<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    main: &str,
    budget: &mut Budget<'_>,
) -> Result<(), Stop> {
    let rels = relationships(archive, main, budget)?;

    // The sheets, in the workbook's order, by the relationship that names
    // each one's part.
    let sheets = with_part(archive, main, budget, |reader, budget| {
        let (mut buffer, mut count) = (Vec::new(), 0);
        let mut sheets = Vec::new();
        loop {
            match next(reader, &mut buffer, budget, &mut count)? {
                Event::Start(element) | Event::Empty(element)
                    if element.local_name().as_ref() == b"sheet" =>
                {
                    if let Some(name) = attribute(&element, b"name") {
                        sheets.push((name, attribute(&element, b"id")));
                    }
                }
                Event::Eof => return Ok(sheets),
                _ => {}
            }
        }
    })?
    .ok_or(Stop::Failed)?;

    let dir = directory_of(main).to_owned();
    let strings_part = rels
        .values()
        .find(|(kind, _)| kind.ends_with("/sharedStrings"))
        .map_or_else(
            || resolve(&dir, "sharedStrings.xml"),
            |(_, target)| target.clone(),
        );
    let strings = shared_strings(archive, &strings_part, budget)?;

    for (index, (name, id)) in sheets.iter().enumerate() {
        let part = id.as_ref().and_then(|id| rels.get(id)).map_or_else(
            || resolve(&dir, &format!("worksheets/sheet{}.xml", index + 1)),
            |(_, target)| target.clone(),
        );
        // A sheet the workbook lists and the package lacks is skipped,
        // not the whole file: the other sheets are still worth reading.
        with_part(archive, &part, budget, |reader, budget| {
            sheet(reader, name, &strings, budget)
        })?;
    }
    Ok(())
}

/// The shared string table: each `si`'s text, phonetic runs left out.
fn shared_strings<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    part: &str,
    budget: &mut Budget<'_>,
) -> Result<Vec<String>, Stop> {
    let found = with_part(archive, part, budget, |reader, budget| {
        let (mut buffer, mut count) = (Vec::new(), 0);
        let mut strings = Vec::new();
        let mut current = String::new();
        let (mut in_item, mut in_text, mut in_phonetic) = (false, 0u32, 0u32);
        loop {
            let event = next(reader, &mut buffer, budget, &mut count)?;
            match &event {
                Event::Start(element) => match element.local_name().as_ref() {
                    b"si" => {
                        in_item = true;
                        current.clear();
                    }
                    b"t" => in_text += 1,
                    b"rPh" => in_phonetic += 1,
                    _ => {}
                },
                Event::Empty(element) if element.local_name().as_ref() == b"si" => {
                    strings.push(String::new());
                }
                Event::End(element) => match element.local_name().as_ref() {
                    b"si" => {
                        in_item = false;
                        strings.push(std::mem::take(&mut current));
                    }
                    b"t" => in_text = in_text.saturating_sub(1),
                    b"rPh" => in_phonetic = in_phonetic.saturating_sub(1),
                    _ => {}
                },
                Event::Eof => return Ok(strings),
                other if in_item && in_text > 0 && in_phonetic == 0 => {
                    if let Some(piece) = text_of(other) {
                        current.push_str(&piece);
                    }
                }
                _ => {}
            }
        }
    })?;
    Ok(found.unwrap_or_default())
}

/// One worksheet, a row at a time.
fn sheet<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    name: &str,
    strings: &[String],
    budget: &mut Budget<'_>,
) -> Result<(), Stop> {
    let (mut buffer, mut count) = (Vec::new(), 0);
    let mut row_number = 0u32;
    let mut row = String::new();
    let mut cell = String::new();
    let mut cell_type: Option<String> = None;
    let mut in_value = 0u32;
    loop {
        let event = next(reader, &mut buffer, budget, &mut count)?;
        match &event {
            Event::Start(element) => match element.local_name().as_ref() {
                b"row" => {
                    row_number = attribute(element, b"r")
                        .and_then(|r| r.parse().ok())
                        .unwrap_or(row_number + 1);
                    row.clear();
                }
                b"c" => {
                    cell_type = attribute(element, b"t");
                    cell.clear();
                }
                b"v" | b"t" => in_value += 1,
                _ => {}
            },
            Event::Empty(element) if element.local_name().as_ref() == b"row" => {
                row_number = attribute(element, b"r")
                    .and_then(|r| r.parse().ok())
                    .unwrap_or(row_number + 1);
            }
            Event::End(element) => match element.local_name().as_ref() {
                b"v" | b"t" => in_value = in_value.saturating_sub(1),
                b"c" => {
                    let value = if cell_type.as_deref() == Some("s") {
                        cell.trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| strings.get(index))
                            .map_or("", String::as_str)
                    } else {
                        cell.as_str()
                    };
                    if !value.trim().is_empty() {
                        if !row.is_empty() {
                            row.push(' ');
                        }
                        row.push_str(value);
                    }
                }
                b"row" => {
                    budget.push(
                        Location::Sheet {
                            name: name.to_owned(),
                            row: row_number,
                        },
                        &row,
                    )?;
                }
                _ => {}
            },
            Event::Eof => return Ok(()),
            other if in_value > 0 => {
                if let Some(piece) = text_of(other) {
                    cell.push_str(&piece);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// PPTX.
// ---------------------------------------------------------------------------

fn pptx<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    main: &str,
    budget: &mut Budget<'_>,
) -> Result<(), Stop> {
    let rels = relationships(archive, main, budget)?;
    let order =
        with_part(archive, main, budget, |reader, budget| {
            let (mut buffer, mut count) = (Vec::new(), 0);
            let mut ids = Vec::new();
            loop {
                match next(reader, &mut buffer, budget, &mut count)? {
                    Event::Start(element) | Event::Empty(element)
                        if element.local_name().as_ref() == b"sldId" =>
                    {
                        // `id` is the slide's number; `r:id`, the prefixed
                        // one, is the relationship that names its part.
                        if let Some(relation) = element.attributes().flatten().find(|a| {
                            a.key.prefix().is_some() && a.key.local_name().as_ref() == b"id"
                        }) {
                            ids.push(String::from_utf8_lossy(&relation.value).into_owned());
                        }
                    }
                    Event::Eof => return Ok(ids),
                    _ => {}
                }
            }
        })?
        .ok_or(Stop::Failed)?;

    for (index, id) in order.iter().enumerate() {
        let Some((_, part)) = rels.get(id) else {
            continue;
        };
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let part = part.clone();
        with_part(archive, &part, budget, |reader, budget| {
            let (mut buffer, mut count) = (Vec::new(), 0);
            let mut text = String::new();
            let mut in_text = 0u32;
            loop {
                let event = next(reader, &mut buffer, budget, &mut count)?;
                match &event {
                    Event::Start(element) if element.local_name().as_ref() == b"t" => in_text += 1,
                    Event::End(element) => match element.local_name().as_ref() {
                        b"t" => in_text = in_text.saturating_sub(1),
                        b"p" => text.push(' '),
                        _ => {}
                    },
                    Event::Empty(element) if element.local_name().as_ref() == b"br" => {
                        text.push(' ')
                    }
                    Event::Eof => break,
                    other if in_text > 0 => {
                        if let Some(piece) = text_of(other) {
                            text.push_str(&piece);
                        }
                    }
                    _ => {}
                }
            }
            budget
                .push(Location::Slide(number), &text)
                .map_err(Stop::from)
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn a_target_resolves_against_its_parts_directory() {
        assert_eq!(
            resolve("xl", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            resolve("xl", "/xl/sharedStrings.xml"),
            "xl/sharedStrings.xml"
        );
        assert_eq!(resolve("ppt/slides", "../media/a.png"), "ppt/media/a.png");
        assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
    }
}
