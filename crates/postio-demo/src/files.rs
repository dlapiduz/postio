//! The search seed's attachment files, built in memory (spec 010 T002).
//!
//! Five formats, each a few hundred bytes of minimal valid structure, each
//! saying "Atlas budget" where an extractor will look: the PDF's page text,
//! the spreadsheet's shared strings, the document's paragraphs, the deck's
//! slides, the note's lines. No dependency builds them: an OOXML package is
//! a zip, and a stored-only zip is a few lines of header bytes; a PDF is
//! text with an offset table.

/// One file the seed attaches.
pub struct SearchFile {
    /// The name a message shows it under.
    pub name: &'static str,
    /// Its MIME type.
    pub mime: &'static str,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

/// Every file the search seed stores, in a fixed order.
pub fn search_files() -> Vec<SearchFile> {
    vec![
        SearchFile {
            name: "Atlas-Q3-budget.xlsx",
            mime: XLSX,
            bytes: workbook(
                "Q3",
                &[
                    ("Atlas budget, Q3 final", 0),
                    ("Platform", 412_000),
                    ("Contractors", 268_500),
                    ("Tooling", 96_250),
                    ("Travel", 41_000),
                    ("Total Atlas budget Q3", 817_750),
                ],
            ),
        },
        SearchFile {
            name: "Atlas-budget-template.xlsx",
            mime: XLSX,
            bytes: workbook(
                "Summary",
                &[
                    ("Atlas budget template v2", 0),
                    ("Platform", 1_640_000),
                    ("Contractors", 1_070_000),
                    ("Tooling", 385_000),
                    ("Travel", 164_000),
                    ("Training", 72_000),
                    ("Licences", 118_000),
                    ("Facilities", 210_000),
                    ("Research", 95_000),
                    ("Events", 48_000),
                    ("Reserve", 120_000),
                    ("Other", 31_000),
                    ("Adjustments", -22_000),
                    ("Total Atlas budget FY26", 4_031_000),
                ],
            ),
        },
        SearchFile {
            name: "Atlas-Sep-actuals.pdf",
            mime: "application/pdf",
            bytes: pdf(&[
                "Atlas budget actuals, September",
                "Spend to date against Atlas budget: 71%",
            ]),
        },
        SearchFile {
            name: "Atlas-budget-memo.docx",
            mime: DOCX,
            bytes: document(&[
                "Memo: the Atlas budget for Q4",
                "The Atlas budget holds the platform roles and the contractors.",
                "Harbor keeps its own line.",
            ]),
        },
        SearchFile {
            name: "Atlas-budget-review.pptx",
            mime: PPTX,
            bytes: deck(&["Atlas budget review", "Where the Atlas budget went in Q3"]),
        },
        SearchFile {
            name: "atlas-budget-notes.txt",
            mime: "text/plain",
            bytes: b"Notes from the Atlas budget sync\n\
                     - platform roles move the Atlas budget up by about 9%\n\
                     - contractors stay flat\n\
                     - Harbor tooling is not part of the Atlas budget\n"
                .to_vec(),
        },
    ]
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn content_types(overrides: &[(&str, &str)]) -> String {
    let mut text = format!(
        "{XML}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>"
    );
    for (part, kind) in overrides {
        text.push_str(&format!(
            "<Override PartName=\"{part}\" ContentType=\"{kind}\"/>"
        ));
    }
    text.push_str("</Types>");
    text
}

fn relationships(items: &[(&str, &str, &str)]) -> String {
    let mut text = format!(
        "{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">"
    );
    for (id, kind, target) in items {
        text.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>"
        ));
    }
    text.push_str("</Relationships>");
    text
}

/// A one-sheet workbook: column A holds the label, column B the number
/// (blank when it is zero), and every label is a shared string, as a
/// spreadsheet program writes them.
fn workbook(sheet: &str, rows: &[(&str, i64)]) -> Vec<u8> {
    let mut strings = String::new();
    let mut cells = String::new();
    for (index, (label, number)) in rows.iter().enumerate() {
        let row = index + 1;
        strings.push_str(&format!("<si><t>{}</t></si>", escape(label)));
        cells.push_str(&format!(
            "<row r=\"{row}\"><c r=\"A{row}\" t=\"s\"><v>{index}</v></c>"
        ));
        if *number != 0 {
            cells.push_str(&format!("<c r=\"B{row}\"><v>{number}</v></c>"));
        }
        cells.push_str("</row>");
    }
    let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    zip(&[
        (
            "[Content_Types].xml",
            content_types(&[
                (
                    "/xl/workbook.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
                ),
                (
                    "/xl/worksheets/sheet1.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
                ),
                (
                    "/xl/sharedStrings.xml",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml",
                ),
            ]),
        ),
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        (
            "xl/workbook.xml",
            format!(
                "{XML}<workbook xmlns=\"{main}\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
                 <sheets><sheet name=\"{}\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
                escape(sheet)
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            relationships(&[
                ("rId1", "worksheet", "worksheets/sheet1.xml"),
                ("rId2", "sharedStrings", "sharedStrings.xml"),
            ]),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!("{XML}<worksheet xmlns=\"{main}\"><sheetData>{cells}</sheetData></worksheet>"),
        ),
        (
            "xl/sharedStrings.xml",
            format!(
                "{XML}<sst xmlns=\"{main}\" count=\"{n}\" uniqueCount=\"{n}\">{strings}</sst>",
                n = rows.len()
            ),
        ),
    ])
}

/// A word-processing document: one paragraph per line.
fn document(paragraphs: &[&str]) -> Vec<u8> {
    let body: String = paragraphs
        .iter()
        .map(|line| format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", escape(line)))
        .collect();
    zip(&[
        (
            "[Content_Types].xml",
            content_types(&[(
                "/word/document.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            )]),
        ),
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "word/document.xml")]),
        ),
        (
            "word/document.xml",
            format!(
                "{XML}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                 <w:body>{body}</w:body></w:document>"
            ),
        ),
    ])
}

/// A presentation: one slide per line, its text in one shape.
fn deck(slides: &[&str]) -> Vec<u8> {
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut overrides: Vec<(String, &str)> = vec![(
        "/ppt/presentation.xml".to_owned(),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
    )];
    let mut ids = String::new();
    let mut links: Vec<(String, &str, String)> = Vec::new();
    for (index, text) in slides.iter().enumerate() {
        let number = index + 1;
        overrides.push((
            format!("/ppt/slides/slide{number}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        ));
        ids.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{number}\"/>",
            255 + number
        ));
        links.push((
            format!("rId{number}"),
            "slide",
            format!("slides/slide{number}.xml"),
        ));
        parts.push((
            format!("ppt/slides/slide{number}.xml"),
            format!(
                "{XML}<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
                 xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
                 <p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>",
                escape(text)
            ),
        ));
    }
    let override_refs: Vec<(&str, &str)> = overrides
        .iter()
        .map(|(part, kind)| (part.as_str(), *kind))
        .collect();
    let link_refs: Vec<(&str, &str, &str)> = links
        .iter()
        .map(|(id, kind, target)| (id.as_str(), *kind, target.as_str()))
        .collect();
    let mut entries: Vec<(&str, String)> = vec![
        ("[Content_Types].xml", content_types(&override_refs)),
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "ppt/presentation.xml")]),
        ),
        (
            "ppt/presentation.xml",
            format!(
                "{XML}<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" \
                 xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
                 <p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"
            ),
        ),
        ("ppt/_rels/presentation.xml.rels", relationships(&link_refs)),
    ];
    for (name, text) in &parts {
        entries.push((name.as_str(), text.clone()));
    }
    zip(&entries)
}

/// A PDF with one page per line of text, in Helvetica.
fn pdf(pages: &[&str]) -> Vec<u8> {
    let count = pages.len();
    // 1 catalog, 2 pages, 3 font, then a page and its stream per page.
    let mut objects: Vec<String> = vec![
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
        let stream = format!(
            "BT /F1 14 Tf 72 720 Td ({}) Tj ET",
            text.replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)")
        );
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * index
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A zip archive of stored (uncompressed) entries.
fn zip(entries: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, text) in entries {
        let data = text.as_bytes();
        let crc = crc32(data);
        let offset = out.len() as u32;
        let size = data.len() as u32;
        let length = name.len() as u16;
        // Local header: signature, version 20, flags 0, method 0 (stored),
        // DOS time and date, crc, sizes, name length, no extra field.
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&[0, 0, 0x21, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // Its central directory record.
        directory.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        directory.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0]);
        directory.extend_from_slice(&[0, 0, 0x21, 0]);
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&length.to_le_bytes());
        directory.extend_from_slice(&[0; 12]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    let size = directory.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out
}

/// CRC-32 (IEEE), bit by bit: the files are small and built once.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn an_archive_ends_with_a_directory_that_counts_its_entries() {
        let bytes = zip(&[("a.txt", "one".to_owned()), ("b/c.txt", "two".to_owned())]);
        let end = bytes.len() - 22;
        assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
        assert_eq!(&bytes[end + 10..end + 12], &[2, 0]);
    }
}
