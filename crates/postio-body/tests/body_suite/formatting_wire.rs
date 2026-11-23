//! What a formatted message is on the wire: spec 014's
//! `contracts/wire.md`, held byte for byte.
//!
//! One golden document carries every construct the composer authors. Its
//! HTML part is pinned here (T011), and its plain-text part beside it
//! (T019). Both are what a recipient's client is handed, so a change to
//! either is a change to what people receive, and has to be made here on
//! purpose.

use postio_body::document::Document;
use postio_body::{harden, parse};

use postio_body::golden::golden;

/// The HTML part, one block per line here for reading; the wire has no
/// whitespace between blocks.
const GOLDEN_HTML: &[&str] = &[
    "<h1>Plan</h1>",
    "<p>Some <strong>bold</strong>, <em>italic</em>, <del>struck</del> and <code>code</code>, and a <a href=\"https://example.com/x\">link</a>.</p>",
    "<h2>Steps</h2>",
    "<ol><li><p>first</p></li><li><p>second</p><ul><li><p>nested</p></li></ul></li></ol>",
    "<ul data-postio-task=\"list\" style=\"list-style-type:none\"><li data-postio-task=\"open\"><p>\u{2610} open task</p></li><li data-postio-task=\"done\"><p>\u{2611} done task</p></li></ul>",
    "<ol start=\"7\"><li><p>seventh</p></li></ol>",
    "<blockquote><p>quoted</p><blockquote><p>nested quote</p></blockquote></blockquote>",
    "<pre data-postio-lang=\"rust\">fn main() {}\n</pre>",
    "<hr>",
    "<table border=\"1\" cellpadding=\"6\" cellspacing=\"0\"><thead><tr><th align=\"left\">Item</th><th align=\"right\">Qty</th></tr></thead><tbody><tr><td align=\"left\">Widget</td><td align=\"right\">3</td></tr></tbody></table>",
    "<p>line one<br>line two</p>",
];

#[test]
fn the_html_part_is_pinned_byte_for_byte() {
    let (_, html) = postio_body::render(&golden());
    assert_eq!(html, GOLDEN_HTML.concat());
}

#[test]
fn the_backstop_changes_nothing_in_the_golden_document() {
    let html = golden().to_html();
    assert_eq!(harden(&html), html);
}

#[test]
fn the_golden_html_reads_back_as_the_golden_document() {
    assert_eq!(parse(&GOLDEN_HTML.concat()), golden());
}

/// The plain-text part, `format=flowed`, as `contracts/wire.md` writes it,
/// with the one paragraph longer than 72 columns soft-broken as RFC 3676
/// says: the trailing space is the soft break a reader joins.
const GOLDEN_TEXT: &str = "\
Plan
====

Some bold, italic, ~~struck~~ and code, and a link \n<https://example.com/x>.

Steps
-----

1. first
2. second
    - nested

- [ ] open task
- [x] done task

7. seventh

> quoted
>
>> nested quote

```rust
fn main() {}
```

----

| Item   | Qty |
|:-------|----:|
| Widget |   3 |

line one
line two";

#[test]
fn the_plain_text_part_is_pinned_byte_for_byte() {
    let (text, _) = postio_body::render(&golden());
    assert_eq!(text, GOLDEN_TEXT);
}

#[test]
fn every_code_line_and_table_row_is_a_fixed_line() {
    // RFC 3676: a line that ends in a space is soft, and a reader joins it
    // to the next. Nothing in a fence or a table may be joined.
    let (text, _) = postio_body::render(&golden());
    let fixed = |line: &str| line.starts_with('|') || line.starts_with("fn ") || line == "```";
    for line in text.lines().filter(|line| fixed(line)) {
        assert!(
            !line.ends_with(' '),
            "{line:?} would flow into the next line"
        );
    }
}

#[test]
fn a_plain_document_is_still_plain() {
    // FR-018: plain text goes out as text/plain alone. The shape is
    // `is_plain_text`'s to decide, and the golden document is not plain.
    assert!(!golden().is_plain_text());
    assert!(Document::from_text("Hello Ada,\n\nThanks.").is_plain_text());
}
