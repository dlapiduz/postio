//! The terminal composer's HTML is the desktop composer's (SC-006).
//!
//! Both build a `postio_body::Document` and send `render` of it, so the
//! question is whether a document written in Markdown is one the desktop
//! composer would itself hold. The desktop composer loads a draft's HTML with
//! `parse`; so for every construct Markdown and the document share, the HTML
//! the terminal sends has to parse back to the very document it came from --
//! otherwise the same draft would change shape the moment the desktop opened
//! it, and the two composers would disagree about what was sent.

use postio_body::markdown::to_document;
use postio_body::{parse, render};

const CONSTRUCTS: &[&str] = &[
    "Plain words.",
    "Some **bold** words.",
    "Some *italic* words.",
    "Some `code`.",
    "A [link](https://example.com/a?b=c) here.",
    "A [mail link](mailto:ada@example.com).",
    "# A heading",
    "## A second",
    "### A third",
    "- one\n- two",
    "1. first\n2. second",
    "- outer\n  - inner",
    "> a quote",
    "```\nlet x = 1;\n```",
    "---",
    "first line\\\nsecond line",
    "**bold with *italic* inside**",
    "~~struck~~",
    "- [ ] open\n- [x] done",
    "7. seventh",
    "```rust\nfn x() {}\n```",
    "| a | b |\n|:--|--:|\n| 1 | 2 |",
    "![chart](https://example.com/c.png)",
    "#### Four\n\n##### Five\n\n###### Six",
    "see www.example.com",
];

#[test]
fn the_golden_markdown_reads_into_the_golden_document() {
    // spec 014 contracts/wire.md: the document is written there as the
    // Markdown that reads into it.
    let golden = postio_body::golden::golden();
    assert_eq!(to_document(postio_body::golden::GOLDEN_MARKDOWN), golden);
    assert_eq!(
        to_document(&postio_body::markdown::from_document(&golden)),
        golden,
        "the golden document does not survive being written as Markdown"
    );
}

#[test]
fn the_html_the_terminal_sends_is_html_the_desktop_would_hold() {
    for markdown in CONSTRUCTS {
        let document = to_document(markdown);
        let html = render(&document).1;
        assert_eq!(
            parse(&html),
            document,
            "{markdown:?} rendered {html:?}, which the desktop composer reads as something else"
        );
    }
}

#[test]
fn a_sentence_typed_in_either_composer_sends_the_same_html() {
    use postio_body::{Block, Document, Href, Inline};
    // What the desktop composer holds after typing the sentence and pressing
    // its bold and link buttons.
    let desktop = Document {
        blocks: vec![Block::Paragraph(vec![
            Inline::Strong(vec![Inline::Text("bold".into())]),
            Inline::Text(" and a ".into()),
            Inline::Link {
                href: Href::parse("https://example.com").unwrap(),
                inlines: vec![Inline::Text("link".into())],
            },
        ])],
    };
    let terminal = to_document("**bold** and a [link](https://example.com)");
    assert_eq!(render(&terminal).1, render(&desktop).1);
}

#[test]
fn a_heading_made_in_either_composer_is_the_same_first_level_heading() {
    use postio_body::{Block, HeadingLevel};
    // #1485: the desktop's `heading_1` is WebKit's `formatBlock` to `<h1>`,
    // and the terminal's is `# ` -- the same command, so the same message.
    // Level one on both sides: a `# ` that sent `<h2>` would be a second
    // dialect of the one verb.
    let desktop = parse("<h1>Agenda</h1><p>First, the gate.</p>");
    let terminal = to_document("# Agenda\n\nFirst, the gate.");
    assert!(
        matches!(
            desktop.blocks.first(),
            Some(Block::Heading {
                level: HeadingLevel::One,
                ..
            })
        ),
        "the desktop's heading is not a first-level one: {desktop:?}"
    );
    assert_eq!(terminal, desktop);
    assert_eq!(render(&terminal).1, render(&desktop).1);
}

// ---------------------------------------------------------------------------
// Spec 014 FR-024: the Markdown source view loses nothing
// ---------------------------------------------------------------------------

mod round_trip {
    //! `to_document(from_document(d)) == d` for every normalised document
    //! without a carried quote (`contracts/source-view.md`). The documents
    //! come from a seeded generator over every shape, weighted towards the
    //! awkward ones the contract names, so a failure is reproducible from
    //! its seed and no dependency is added for it.

    use postio_body::document::{
        Alignment, Block, ContentId, Document, HeadingLevel, Href, Inline, Language, ListItem,
        ListKind, Table, Task, WebHref,
    };
    use postio_body::markdown::{from_document, normalize, to_document};

    /// xorshift64*: small, seeded, and the same on every machine.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn chance(&mut self, percent: u64) -> bool {
            self.next() % 100 < percent
        }
        fn pick<'a, T>(&mut self, from: &'a [T]) -> &'a T {
            &from[self.below(from.len())]
        }
    }

    /// Words, every ASCII punctuation character, and the edges that need a
    /// spelling: a leading or trailing space, a digit run and a dot.
    const WORDS: &[&str] = &["plan", "tide", "Gate", "x", "42", "é", "日本"];
    const PUNCTUATION: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

    fn text(rng: &mut Rng) -> String {
        let mut out = String::new();
        for index in 0..1 + rng.below(4) {
            if index > 0 || rng.chance(15) {
                out.push(' ');
            }
            if rng.chance(30) {
                let at = rng.below(PUNCTUATION.len());
                out.push_str(&PUNCTUATION[at..=at]);
            }
            out.push_str(rng.pick(WORDS));
            if rng.chance(25) {
                let at = rng.below(PUNCTUATION.len());
                out.push_str(&PUNCTUATION[at..=at]);
            }
        }
        if rng.chance(10) {
            out.push(' ');
        }
        if rng.chance(5) {
            out = rng
                .pick(&[
                    "1. not a list",
                    "- not a list",
                    "# not a heading",
                    "===",
                    "www.example.com",
                    "ada@example.com",
                    "    indented",
                    "&amp;",
                    "<b>",
                ])
                .to_string();
        }
        out
    }

    /// A word for the inside of a mark: a mark's edges are its words.
    fn word(rng: &mut Rng) -> String {
        let mut out = rng.pick(WORDS).to_string();
        if rng.chance(30) {
            out.push(' ');
            out.push_str(rng.pick(WORDS));
        }
        out
    }

    fn inline(rng: &mut Rng, depth: usize) -> Inline {
        let nested = |rng: &mut Rng| {
            if depth < 2 && rng.chance(30) {
                vec![inline(rng, depth + 1)]
            } else {
                vec![Inline::Text(word(rng))]
            }
        };
        match rng.below(12) {
            0 => Inline::Strong(nested(rng)),
            1 => Inline::Emphasis(nested(rng)),
            2 => Inline::Strikethrough(nested(rng)),
            3 => Inline::Code(
                rng.pick(&["code", "a `tick`", "``", "` x `", " lead", "a\\b", "x|y"])
                    .to_string(),
            ),
            4 if depth == 0 => Inline::Link {
                href: Href::parse(rng.pick(&[
                    "https://example.com/x",
                    "https://example.com/a?b=c&d=e",
                    "mailto:ada@example.com",
                    "https://example.com/(paren)",
                ]))
                .unwrap(),
                inlines: vec![Inline::Text(word(rng))],
            },
            5 => Inline::Image {
                content_id: ContentId::parse("chart.1@example.com").unwrap(),
                alt: rng.pick(&["chart", "", "a [b]"]).to_string(),
            },
            6 => Inline::RemoteImage {
                href: WebHref::parse("https://example.com/x.png").unwrap(),
                alt: rng.pick(&["x", "", "a *b*"]).to_string(),
            },
            7 => Inline::Break,
            _ => Inline::Text(text(rng)),
        }
    }

    fn inlines(rng: &mut Rng) -> Vec<Inline> {
        (0..rng.below(5)).map(|_| inline(rng, 0)).collect()
    }

    fn paragraph(rng: &mut Rng) -> Block {
        Block::Paragraph(inlines(rng))
    }

    fn cell(rng: &mut Rng) -> Vec<Inline> {
        (0..rng.below(3))
            .map(|_| match rng.below(5) {
                0 => Inline::Break,
                1 => Inline::Strong(vec![Inline::Text(word(rng))]),
                _ => Inline::Text(text(rng)),
            })
            .collect()
    }

    fn block(rng: &mut Rng, depth: usize) -> Block {
        let deep = depth < 2;
        match rng.below(if deep { 9 } else { 6 }) {
            0 => Block::Heading {
                level: *rng.pick(&HeadingLevel::ALL),
                inlines: inlines(rng),
            },
            1 => Block::Code {
                language: if rng.chance(50) {
                    Language::parse(rng.pick(&["rust", "c++", "objective-c"]))
                } else {
                    None
                },
                text: rng
                    .pick(&[
                        "fn x() {}\n",
                        "",
                        "a\n\n  b\n",
                        "```\nfenced\n```\n",
                        "> not a quote\n",
                    ])
                    .to_string(),
            },
            2 => Block::Rule,
            3 => {
                let width = 1 + rng.below(3);
                let alignments = (0..width)
                    .map(|_| {
                        *rng.pick(&[
                            Alignment::None,
                            Alignment::Left,
                            Alignment::Center,
                            Alignment::Right,
                        ])
                    })
                    .collect();
                let header = (0..width).map(|_| cell(rng)).collect();
                let rows = (0..rng.below(3))
                    .map(|_| (0..width).map(|_| cell(rng)).collect())
                    .collect();
                Block::Table(Table::new(alignments, header, rows))
            }
            6 => Block::Quote(
                (0..1 + rng.below(3))
                    .map(|_| block(rng, depth + 1))
                    .collect(),
            ),
            7 | 8 => {
                let kind = if rng.chance(50) {
                    ListKind::Bullet
                } else {
                    ListKind::Ordered {
                        start: *rng.pick(&[0, 1, 7, 12]),
                    }
                };
                let items = (0..1 + rng.below(3))
                    .map(|_| {
                        let task = rng.chance(30).then(|| *rng.pick(&[Task::Open, Task::Done]));
                        let mut blocks = vec![if task.is_some() {
                            Block::Paragraph(vec![Inline::Text(word(rng))])
                        } else {
                            paragraph(rng)
                        }];
                        if rng.chance(40) {
                            blocks.push(block(rng, depth + 1));
                        }
                        ListItem { task, blocks }
                    })
                    .collect();
                Block::List { kind, items }
            }
            _ => paragraph(rng),
        }
    }

    fn document(rng: &mut Rng) -> Document {
        Document {
            blocks: (0..1 + rng.below(5)).map(|_| block(rng, 0)).collect(),
        }
    }

    #[test]
    fn every_normalised_document_survives_the_markdown_view() {
        let mut failures = Vec::new();
        for seed in 1..=2_000u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let document = normalize(document(&mut rng));
            assert_eq!(
                normalize(document.clone()),
                document,
                "normalize is not idempotent, seed {seed}"
            );
            let written = from_document(&document);
            let read = to_document(&written);
            if read != document {
                failures.push(format!(
                    "seed {seed}: wrote {written:?}\n  from {document:?}\n  read {read:?}"
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of 2000 documents changed in the Markdown view; the first:\n{}",
            failures.len(),
            failures
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// The contract's golden spellings (`contracts/source-view.md`).
    #[test]
    fn the_golden_spellings_are_written_as_the_contract_says() {
        let para = |inlines| Document {
            blocks: vec![Block::Paragraph(inlines)],
        };
        let text = |s: &str| Inline::Text(s.to_owned());
        for (document, written) in [
            (para(vec![text("a"), Inline::Break]), "a<br>"),
            (para(vec![Inline::Break, text("a")]), "<br>a"),
            (para(vec![]), "<br>"),
            (para(vec![text(" a ")]), "&#32;a&#32;"),
            (
                para(vec![Inline::RemoteImage {
                    href: WebHref::parse("https://example.com/x.png").unwrap(),
                    alt: "x".to_owned(),
                }]),
                "![x](https://example.com/x.png)",
            ),
            (
                Document {
                    blocks: vec![Block::Heading {
                        level: HeadingLevel::Two,
                        inlines: vec![text("a"), Inline::Break, text("b")],
                    }],
                },
                "## a<br>b",
            ),
        ] {
            assert_eq!(from_document(&document).trim_end(), written, "{document:?}");
            assert_eq!(to_document(written), document, "{written:?}");
        }
        let list = |s: &str| Block::List {
            kind: ListKind::Bullet,
            items: vec![ListItem::new(vec![Block::Paragraph(vec![text(s)])])],
        };
        let two = Document {
            blocks: vec![list("a"), list("b")],
        };
        assert_eq!(from_document(&two).trim_end(), "- a\n\n<!-- -->\n\n- b");
        assert_eq!(to_document("- a\n\n<!-- -->\n\n- b"), two);
    }
}
