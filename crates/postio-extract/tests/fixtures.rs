//! Extraction over files built here, from invented text (spec 010 T120).
//!
//! Every fixture is generated in memory rather than committed: an OOXML
//! package is a zip of a few XML parts, a PDF is text with an offset table,
//! and the hostile set is the honest set bent — encrypted, cut short,
//! inflated, nested, made to point at itself. Nothing here is real mail
//! and no name in it is a person's.
//!
//! The promises: each format's units carry the location a person would
//! name ("Sheet ‘Summary’, row 14", "Page 2"), and each hostile file ends
//! in its recorded outcome within `Limits::max_time`, with no panic
//! reaching the caller.

use std::io::{Cursor, Write as _};
use std::time::{Duration, Instant};

use postio_extract::{Extracted, Limit, Limits, Location, Outcome, Skip, extract};

const PDF: &str = "application/pdf";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

// ---------------------------------------------------------------------------
// The generator.
// ---------------------------------------------------------------------------

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A zip of `entries`, deflated, as an office program writes one.
fn zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        writer.start_file(*name, options).expect("an entry");
        writer.write_all(bytes).expect("its bytes");
    }
    writer.finish().expect("the archive").into_inner()
}

const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn relationships(items: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut text = format!("{XML}<Relationships xmlns=\"{RELS}\">");
    for (id, kind, target) in items {
        text.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{OFFICE}/{kind}\" Target=\"{target}\"/>"
        ));
    }
    text.push_str("</Relationships>");
    text.into_bytes()
}

/// A workbook of named sheets, each a list of rows of cells. Text cells
/// are shared strings, numbers are numbers, as a spreadsheet program
/// writes them; sheet files are numbered in reverse so the walker has to
/// follow the workbook's relationships rather than guess from names.
fn xlsx(sheets: &[(&str, Vec<Vec<String>>)]) -> Vec<u8> {
    let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let mut strings: Vec<String> = Vec::new();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut listed = String::new();
    let mut links = Vec::new();
    for (index, (name, rows)) in sheets.iter().enumerate() {
        let file = sheets.len() - index;
        let mut data = String::new();
        for (r, row) in rows.iter().enumerate() {
            let number = r + 1;
            data.push_str(&format!("<row r=\"{number}\">"));
            for (c, cell) in row.iter().enumerate() {
                let column = (b'A' + c as u8) as char;
                if cell.parse::<f64>().is_ok() {
                    data.push_str(&format!("<c r=\"{column}{number}\"><v>{cell}</v></c>"));
                } else if !cell.is_empty() {
                    strings.push(cell.clone());
                    data.push_str(&format!(
                        "<c r=\"{column}{number}\" t=\"s\"><v>{}</v></c>",
                        strings.len() - 1
                    ));
                }
            }
            data.push_str("</row>");
        }
        entries.push((
            format!("xl/worksheets/sheet{file}.xml"),
            format!("{XML}<worksheet xmlns=\"{main}\"><sheetData>{data}</sheetData></worksheet>")
                .into_bytes(),
        ));
        listed.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
            escape(name),
            index + 1,
            index + 1
        ));
        links.push((
            format!("rId{}", index + 1),
            "worksheet",
            format!("worksheets/sheet{file}.xml"),
        ));
    }
    links.push((
        format!("rId{}", sheets.len() + 1),
        "sharedStrings",
        "sharedStrings.xml".to_owned(),
    ));
    let shared: String = strings
        .iter()
        .map(|text| format!("<si><t>{}</t></si>", escape(text)))
        .collect();
    let link_refs: Vec<(&str, &str, &str)> = links
        .iter()
        .map(|(id, kind, target)| (id.as_str(), *kind, target.as_str()))
        .collect();
    let mut all: Vec<(&str, Vec<u8>)> = vec![
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        (
            "xl/workbook.xml",
            format!(
                "{XML}<workbook xmlns=\"{main}\" xmlns:r=\"{OFFICE}\"><sheets>{listed}</sheets></workbook>"
            )
            .into_bytes(),
        ),
        ("xl/_rels/workbook.xml.rels", relationships(&link_refs)),
        (
            "xl/sharedStrings.xml",
            format!("{XML}<sst xmlns=\"{main}\">{shared}</sst>").into_bytes(),
        ),
    ];
    for (name, bytes) in &entries {
        all.push((name.as_str(), bytes.clone()));
    }
    zip(&all)
}

/// A document from its raw `w:body` XML.
fn docx_body(body: &str) -> Vec<u8> {
    zip(&[
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "word/document.xml")]),
        ),
        (
            "word/document.xml",
            format!(
                "{XML}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                 <w:body>{body}</w:body></w:document>"
            )
            .into_bytes(),
        ),
    ])
}

fn paragraph(text: &str) -> String {
    format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", escape(text))
}

/// A presentation, one slide per entry, each slide's text in two runs
/// of one shape. Slide files are numbered in reverse of the deck's
/// order, so the walker has to follow `p:sldIdLst`.
fn pptx(slides: &[&str]) -> Vec<u8> {
    let mut ids = String::new();
    let mut links = Vec::new();
    let mut parts = Vec::new();
    for (index, text) in slides.iter().enumerate() {
        let file = slides.len() - index;
        ids.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + index,
            index + 1
        ));
        links.push((
            format!("rId{}", index + 1),
            format!("slides/slide{file}.xml"),
        ));
        let (first, rest) = text.split_once(' ').unwrap_or((text, ""));
        parts.push((
            format!("ppt/slides/slide{file}.xml"),
            format!(
                "{XML}<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
                 xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
                 <p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{} </a:t></a:r><a:r><a:t>{}</a:t></a:r></a:p>\
                 </p:txBody></p:sp></p:spTree></p:cSld></p:sld>",
                escape(first),
                escape(rest)
            )
            .into_bytes(),
        ));
    }
    let link_refs: Vec<(&str, &str, &str)> = links
        .iter()
        .map(|(id, target)| (id.as_str(), "slide", target.as_str()))
        .collect();
    let mut all: Vec<(&str, Vec<u8>)> = vec![
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "ppt/presentation.xml")]),
        ),
        (
            "ppt/presentation.xml",
            format!(
                "{XML}<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" \
                 xmlns:r=\"{OFFICE}\"><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"
            )
            .into_bytes(),
        ),
        ("ppt/_rels/presentation.xml.rels", relationships(&link_refs)),
    ];
    for (name, bytes) in &parts {
        all.push((name.as_str(), bytes.clone()));
    }
    zip(&all)
}

/// A PDF of numbered objects, each written as given, with the offset
/// table and trailer a reader needs. `objects[0]` is object 1.
fn pdf_objects(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let table = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /ID [<0123456789abcdef0123456789abcdef> <0123456789abcdef0123456789abcdef>] >>\nstartxref\n{table}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn stream(content: &str) -> String {
    format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

/// A PDF with one page per entry, in Helvetica.
fn pdf(pages: &[&str]) -> Vec<u8> {
    let count = pages.len();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {count} >>",
            (0..count)
                .map(|index| format!("{} 0 R", 4 + 2 * index))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    for (index, text) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * index
        ));
        objects.push(stream(&format!("BT /F1 14 Tf 72 720 Td ({text}) Tj ET")));
    }
    pdf_objects(&objects)
}

fn three_pages() -> Vec<u8> {
    pdf(&[
        "Harbor survey, first soundings",
        "Spend to date against the Atlas budget: 71%",
        "Next review in November",
    ])
}

/// The three pages, encrypted with a user password nobody will give.
fn encrypted_pdf() -> Vec<u8> {
    use pdf_extract::{Document, EncryptionState, EncryptionVersion, Permissions};
    let mut document = Document::load_mem(&three_pages()).expect("the plain PDF loads");
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &document,
        owner_password: "owner-only",
        user_password: "not-given",
        key_length: 128,
        permissions: Permissions::all(),
    })
    .expect("an encryption state");
    document.encrypt(&state).expect("encrypted");
    let mut out = Vec::new();
    document.save_to(&mut out).expect("saved");
    out
}

/// A page whose one form XObject draws itself: a reader with no guard
/// recurses until the stack runs out, which no `catch_unwind` can catch.
fn self_drawing_pdf() -> Vec<u8> {
    pdf_objects(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /XObject << /X1 5 0 R >> >> /Contents 4 0 R >>"
            .to_owned(),
        stream("/X1 Do"),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] \
             /Resources << /XObject << /X1 5 0 R >> >> /Length {} >>\nstream\n/X1 Do\nendstream",
            "/X1 Do".len()
        ),
    ])
}

/// A page whose parent is itself, with no MediaBox to stop the climb.
fn own_parent_pdf() -> Vec<u8> {
    pdf_objects(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /Parent 2 0 R >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_owned(),
        stream("BT ET"),
    ])
}

fn utf16_text() -> Vec<u8> {
    let text = "Notes from the Atlas sync\r\n\r\nthe café keeps its own line\r\n";
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn summary_rows() -> Vec<Vec<String>> {
    let mut rows = vec![vec!["Atlas budget template v2".to_owned()]];
    for (label, amount) in [
        ("Platform", "1640000"),
        ("Contractors", "1070000"),
        ("Tooling", "385000"),
        ("Travel", "164000"),
        ("Training", "72000"),
        ("Licences", "118000"),
        ("Facilities", "210000"),
        ("Research", "95000"),
        ("Events", "48000"),
        ("Reserve", "120000"),
        ("Other", "31000"),
        ("Adjustments", "-22000"),
    ] {
        rows.push(vec![label.to_owned(), amount.to_owned()]);
    }
    rows.push(vec![
        "Total Atlas budget FY26".to_owned(),
        "4031000".to_owned(),
    ]);
    rows
}

fn workbook() -> Vec<u8> {
    xlsx(&[
        ("Summary", summary_rows()),
        (
            "Q3",
            vec![
                vec!["Harbor Q3".to_owned()],
                vec![String::new()],
                vec!["Total".to_owned(), "1240000".to_owned()],
            ],
        ),
    ])
}

// ---------------------------------------------------------------------------
// The honest formats.
// ---------------------------------------------------------------------------

fn run(bytes: &[u8], mime: &str, name: &str) -> Extracted {
    extract(bytes, mime, Some(name), &Limits::default())
}

fn located(extracted: &Extracted, word: &str) -> Vec<Location> {
    extracted
        .units
        .iter()
        .filter(|unit| unit.text.to_lowercase().contains(word))
        .map(|unit| unit.location.clone())
        .collect()
}

#[test]
fn a_pdf_is_read_a_page_at_a_time() {
    let extracted = run(&three_pages(), PDF, "survey.pdf");
    assert_eq!(extracted.outcome, Outcome::Complete);
    assert_eq!(extracted.units.len(), 3, "{:?}", extracted.units);
    assert_eq!(located(&extracted, "atlas"), vec![Location::Page(2)]);
    assert_eq!(
        extracted.units[1].text,
        "Spend to date against the Atlas budget: 71%"
    );
}

#[test]
fn a_workbook_is_read_a_row_at_a_time_under_its_sheet_names() {
    let extracted = run(&workbook(), XLSX, "budget.xlsx");
    assert_eq!(extracted.outcome, Outcome::Complete);
    assert_eq!(
        located(&extracted, "fy26"),
        vec![Location::Sheet {
            name: "Summary".to_owned(),
            row: 14
        }]
    );
    let atlas = located(&extracted, "atlas");
    assert!(atlas.contains(&Location::Sheet {
        name: "Summary".to_owned(),
        row: 14
    }));
    assert_eq!(
        located(&extracted, "1240000"),
        vec![Location::Sheet {
            name: "Q3".to_owned(),
            row: 3
        }],
        "numbers are text too, and the blank row 2 still counts"
    );
    let row = extracted
        .units
        .iter()
        .find(|unit| unit.text.contains("FY26"))
        .expect("row 14");
    assert_eq!(row.text, "Total Atlas budget FY26 4031000");
}

#[test]
fn a_document_is_read_a_paragraph_at_a_time_and_its_tables_a_row_at_a_time() {
    let body = format!(
        "{}{}<w:tbl><w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr></w:tbl>{}",
        paragraph("Memo: the Atlas budget for Q4"),
        "<w:p><w:r><w:t xml:space=\"preserve\">Harbor keeps </w:t></w:r><w:r><w:t>its own line.</w:t></w:r></w:p>",
        paragraph("Team"),
        paragraph("Amount"),
        paragraph("Platform"),
        paragraph("Atlas 412000"),
        paragraph("Signed, the planning group"),
    );
    let extracted = run(&docx_body(&body), DOCX, "memo.docx");
    assert_eq!(extracted.outcome, Outcome::Complete);
    let locations: Vec<Location> = extracted.units.iter().map(|u| u.location.clone()).collect();
    assert_eq!(
        locations,
        vec![
            Location::Paragraph(1),
            Location::Paragraph(2),
            Location::Table { index: 1, row: 1 },
            Location::Table { index: 1, row: 2 },
            Location::Paragraph(3),
        ]
    );
    assert_eq!(extracted.units[1].text, "Harbor keeps its own line.");
    assert_eq!(extracted.units[3].text, "Platform Atlas 412000");
}

#[test]
fn a_deck_is_read_a_slide_at_a_time_in_the_decks_order() {
    let extracted = run(
        &pptx(&["Harbor review", "Where the Atlas budget went", "Questions"]),
        PPTX,
        "review.pptx",
    );
    assert_eq!(extracted.outcome, Outcome::Complete);
    assert_eq!(located(&extracted, "atlas"), vec![Location::Slide(2)]);
    assert_eq!(extracted.units[1].text, "Where the Atlas budget went");
}

#[test]
fn utf16_text_is_read_a_line_at_a_time_by_its_byte_order_mark() {
    let extracted = run(&utf16_text(), "text/plain", "notes.txt");
    assert_eq!(extracted.outcome, Outcome::Complete);
    assert_eq!(located(&extracted, "atlas"), vec![Location::Line(1)]);
    assert_eq!(located(&extracted, "café"), vec![Location::Line(3)]);
}

#[test]
fn text_in_a_declared_charset_is_decoded_by_it() {
    // "café" in Latin-1: one byte for the é.
    let extracted = run(
        b"the caf\xe9 keeps its own line\n",
        "text/plain; charset=iso-8859-1",
        "notes.txt",
    );
    assert_eq!(extracted.units[0].text, "the café keeps its own line");
}

#[test]
fn a_generic_type_is_read_by_the_files_name() {
    let extracted = run(&workbook(), "application/octet-stream", "budget.xlsx");
    assert_eq!(extracted.outcome, Outcome::Complete);
    assert!(!located(&extracted, "fy26").is_empty());
}

#[test]
fn an_image_is_not_read_and_says_so() {
    let extracted = run(b"\x89PNG\r\n\x1a\n", "image/png", "photo.png");
    assert_eq!(extracted.outcome, Outcome::Skipped(Skip::Unsupported));
    assert!(extracted.units.is_empty());
}

#[test]
fn a_file_with_no_words_is_empty() {
    let extracted = run(b"\n \n\t\n", "text/plain", "blank.txt");
    assert_eq!(extracted.outcome, Outcome::Skipped(Skip::Empty));
}

#[test]
fn a_file_over_the_input_limit_is_not_opened() {
    let limits = Limits {
        max_input: 16,
        ..Limits::default()
    };
    let extracted = extract(&three_pages(), PDF, Some("survey.pdf"), &limits);
    assert_eq!(extracted.outcome, Outcome::Skipped(Skip::TooLarge));
}

// ---------------------------------------------------------------------------
// The hostile set: each ends in its outcome, inside the time limit.
// ---------------------------------------------------------------------------

/// Extract under the default limits and assert it ended in `expected`
/// within `max_time` (plus a scheduling grace on a loaded machine).
fn hostile(bytes: &[u8], mime: &str, name: &str, expected: Outcome) -> Extracted {
    let limits = Limits::default();
    let started = Instant::now();
    let extracted = extract(bytes, mime, Some(name), &limits);
    let elapsed = started.elapsed();
    assert_eq!(extracted.outcome, expected, "{name}");
    assert!(
        elapsed <= limits.max_time + Duration::from_millis(500),
        "{name} took {elapsed:?}, past its limit of {:?}",
        limits.max_time
    );
    extracted
}

#[test]
fn an_encrypted_pdf_is_skipped_as_encrypted() {
    let extracted = hostile(
        &encrypted_pdf(),
        PDF,
        "locked.pdf",
        Outcome::Skipped(Skip::Encrypted),
    );
    assert!(extracted.units.is_empty());
}

#[test]
fn a_truncated_pdf_fails_without_a_panic() {
    let whole = three_pages();
    let cut = &whole[..whole.len() / 2];
    hostile(cut, PDF, "cut.pdf", Outcome::Failed);
}

#[test]
fn garbage_named_pdf_fails() {
    hostile(
        b"%PDF-1.7\n\x00\x01\x02 not a document",
        PDF,
        "junk.pdf",
        Outcome::Failed,
    );
}

#[test]
fn a_form_that_draws_itself_fails_rather_than_overflowing_the_stack() {
    hostile(&self_drawing_pdf(), PDF, "loop.pdf", Outcome::Failed);
}

#[test]
fn a_page_that_is_its_own_ancestor_fails_rather_than_overflowing_the_stack() {
    hostile(&own_parent_pdf(), PDF, "parent.pdf", Outcome::Failed);
}

#[test]
fn a_zip_bomb_stops_at_its_ratio() {
    // 200 MB of one byte deflates to a few hundred kilobytes.
    let mut document = format!(
        "{XML}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>"
    )
    .into_bytes();
    document.resize(200 * 1024 * 1024, b' ');
    let bomb = zip(&[
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "word/document.xml")]),
        ),
        ("word/document.xml", document),
    ]);
    assert!(
        bomb.len() < 2 * 1024 * 1024,
        "the bomb is small: {}",
        bomb.len()
    );
    hostile(&bomb, DOCX, "bomb.docx", Outcome::Truncated(Limit::Ratio));
}

#[test]
fn a_sheet_of_two_hundred_thousand_rows_stops_at_the_unit_limit() {
    let rows: Vec<Vec<String>> = (0..200_000)
        .map(|n| vec![format!("line {n}"), n.to_string()])
        .collect();
    let workbook = xlsx(&[("Ledger", rows)]);
    let extracted = hostile(
        &workbook,
        XLSX,
        "ledger.xlsx",
        Outcome::Truncated(Limit::Units),
    );
    assert_eq!(extracted.units.len(), Limits::default().max_units);
    assert_eq!(
        extracted.units[0].location,
        Location::Sheet {
            name: "Ledger".to_owned(),
            row: 1
        }
    );
}

#[test]
fn deeply_nested_xml_is_walked_without_recursion() {
    // Deep enough to overflow any recursive walker, small enough (under
    // a megabyte inflated) that the ratio limit does not judge it.
    let depth = 20_000;
    let body = format!(
        "<w:p>{}<w:r><w:t>atlas at the bottom</w:t></w:r>{}</w:p>",
        "<w:sdt><w:sdtContent>".repeat(depth),
        "</w:sdtContent></w:sdt>".repeat(depth)
    );
    let extracted = hostile(&docx_body(&body), DOCX, "deep.docx", Outcome::Complete);
    assert_eq!(located(&extracted, "atlas"), vec![Location::Paragraph(1)]);
}

#[test]
fn a_package_missing_its_main_part_fails() {
    let empty = zip(&[("_rels/.rels", relationships(&[]))]);
    hostile(&empty, DOCX, "hollow.docx", Outcome::Failed);
}

#[test]
fn a_zip_that_is_not_a_zip_fails() {
    hostile(
        b"PK\x03\x04 and then nothing",
        XLSX,
        "broken.xlsx",
        Outcome::Failed,
    );
}
