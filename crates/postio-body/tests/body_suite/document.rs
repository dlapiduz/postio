use postio_body::document::{
    Block, ContentId, Document, HeadingLevel, Href, Inline, ListItem, ListKind,
};
use postio_body::parse;

fn every_shape() -> Document {
    Document {
        blocks: vec![
            Block::Paragraph(vec![
                Inline::Text("plain, with ".to_owned()),
                Inline::Strong(vec![Inline::Text("bold".to_owned())]),
                Inline::Text(" and ".to_owned()),
                Inline::Emphasis(vec![Inline::Text("italic".to_owned())]),
                Inline::Break,
                Inline::Code("let x = 1;".to_owned()),
                Inline::Link {
                    href: Href::parse("https://example.com/a?b=c&d=e").unwrap(),
                    inlines: vec![Inline::Text("a link".to_owned())],
                },
                Inline::Image {
                    content_id: ContentId::parse("part1@example.com").unwrap(),
                    alt: "a picture".to_owned(),
                },
            ]),
            Block::Heading {
                level: HeadingLevel::One,
                inlines: vec![Inline::Text("one".to_owned())],
            },
            Block::Heading {
                level: HeadingLevel::Two,
                inlines: vec![Inline::Text("two".to_owned())],
            },
            Block::Heading {
                level: HeadingLevel::Three,
                inlines: vec![Inline::Text("three".to_owned())],
            },
            Block::List {
                kind: ListKind::Bullet,
                items: vec![
                    ListItem::new(vec![Block::Paragraph(vec![Inline::Text(
                        "first".to_owned(),
                    )])]),
                    ListItem::new(vec![
                        Block::Paragraph(vec![Inline::Text("second".to_owned())]),
                        Block::List {
                            kind: ListKind::NUMBERED,
                            items: vec![ListItem::new(vec![Block::Paragraph(vec![Inline::Text(
                                "nested".to_owned(),
                            )])])],
                        },
                    ]),
                ],
            },
            Block::Quote(vec![
                Block::Paragraph(vec![Inline::Text("they wrote this".to_owned())]),
                Block::Quote(vec![Block::Paragraph(vec![Inline::Text(
                    "and quoted this".to_owned(),
                )])]),
            ]),
            Block::Code {
                language: None,
                text: "  indented\n    further\n".to_owned(),
            },
            Block::Rule,
        ],
    }
}

#[test]
fn structure_survives_the_round_trip() {
    let document = every_shape();
    let round_tripped = parse(&document.to_html());
    assert_eq!(
        round_tripped, document,
        "document -> HTML -> document lost or changed structure"
    );
}

#[test]
fn the_serialiser_is_the_normal_form() {
    let html = every_shape().to_html();
    assert_eq!(parse(&html).to_html(), html, "to_html is not idempotent");
}

#[test]
fn a_script_has_no_representation() {
    let document = parse(
        "<p>before</p><script>alert('pwned')</script><p>after</p>\
         <p onclick=\"steal()\">handler</p>",
    );

    let html = document.to_html();
    assert!(!html.contains("script"), "{html}");
    assert!(!html.contains("alert"), "{html}");
    assert!(!html.contains("onclick"), "{html}");
    assert!(html.contains("before") && html.contains("after") && html.contains("handler"));
}

#[test]
fn a_remote_image_has_no_representation() {
    let document = parse(
        "<p><img src=\"https://tracker.example.com/pixel.gif\" width=\"1\">\
         <img src=\"http://tracker.example.net/p.png\">\
         <img src=\"data:image/gif;base64,R0lGOD\">\
         <img src=\"cid:real@example.com\" alt=\"kept\"></p>",
    );

    let images: Vec<_> = match &document.blocks[0] {
        Block::Paragraph(inlines) => inlines
            .iter()
            .filter(|i| matches!(i, Inline::Image { .. }))
            .collect(),
        other => panic!("expected a paragraph, got {other:?}"),
    };
    assert_eq!(images.len(), 1, "only the cid: image survives: {images:?}");

    let html = document.to_html();
    assert!(!html.contains("tracker.example"), "{html}");
    assert!(!html.contains("data:"), "{html}");
    assert!(html.contains("cid:real@example.com"), "{html}");
}

#[test]
fn only_three_schemes_construct_an_href() {
    for good in [
        "https://example.com",
        "http://example.com/x",
        "mailto:ada@example.com",
        "HTTPS://EXAMPLE.COM",
    ] {
        assert!(Href::parse(good).is_some(), "{good} should be a link");
    }
    for bad in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html;base64,PHNjcmlwdD4=",
        "vbscript:msgbox",
        "file:///etc/passwd",
        "/relative/path",
        "#anchor",
        "",
        "https:",
        "java\tscript:alert(1)",
        "java\nscript:alert(1)",
    ] {
        assert!(Href::parse(bad).is_none(), "{bad:?} must not be a link");
    }
}

#[test]
fn a_javascript_link_loses_the_href_and_keeps_the_words() {
    let document = parse("<p><a href=\"javascript:alert(1)\">click me</a></p>");
    let html = document.to_html();

    assert!(!html.contains("javascript"), "{html}");
    assert!(
        !html.contains("<a "),
        "a refused href must not emit a link: {html}"
    );
    assert!(
        html.contains("click me"),
        "the author's words are not the attack: {html}"
    );
}

#[test]
fn a_content_id_cannot_be_a_url() {
    for bad in [
        "https://evil.example/p.gif",
        "//evil.example",
        "a b",
        "",
        "<>",
        "with\tcontrol",
    ] {
        assert!(ContentId::parse(bad).is_none(), "{bad:?}");
    }
    assert_eq!(
        ContentId::parse("<part1@example.com>").unwrap().as_str(),
        "part1@example.com",
        "MIME writes the brackets; the id is what is inside them"
    );
}

#[test]
fn styling_does_not_survive() {
    let document = parse(
        "<p style=\"color:red\" class=\"x\"><font color=\"red\">red</font> \
         <span style=\"display:none\">hidden</span></p>",
    );
    let html = document.to_html();

    for banned in ["style", "class", "color", "font", "span"] {
        assert!(!html.contains(banned), "{banned} survived: {html}");
    }
    assert!(html.contains("red") && html.contains("hidden"));
}

#[test]
fn a_plain_text_document_is_just_paragraphs() {
    let document = Document::from_text("first line\nsame paragraph\n\nsecond paragraph");

    assert_eq!(
        document,
        Document {
            blocks: vec![
                Block::Paragraph(vec![
                    Inline::Text("first line".to_owned()),
                    Inline::Break,
                    Inline::Text("same paragraph".to_owned()),
                ]),
                Block::Paragraph(vec![Inline::Text("second paragraph".to_owned())]),
            ],
        }
    );
    assert_eq!(
        document.to_text(),
        "first line\nsame paragraph\n\nsecond paragraph"
    );
    assert_eq!(
        document.to_html(),
        "<p>first line<br>same paragraph</p><p>second paragraph</p>"
    );
}

#[test]
fn plain_text_survives_the_composer_exactly() {
    // A reply opens with blank lines to type into; `to_text(from_text(t)) == t` must not tidy them.
    for text in [
        "",
        "one line",
        "\n\n-- \nLena\n",
        "trailing newline\n",
        "two trailing\n\n",
        "\n\n\nleading blanks",
        "a\n\nb\n\nc",
        "  indented  \n\tand tabbed",
    ] {
        assert_eq!(
            Document::from_text(text).to_text(),
            *text,
            "from_text/to_text is not lossless for {text:?}"
        );
    }
}

#[test]
fn to_text_is_readable_rather_than_a_tag_soup() {
    let text = every_shape().to_text();

    assert!(text.contains("plain, with bold and italic"));
    assert!(
        text.contains("a link <https://example.com/a?b=c&d=e>"),
        "a link's address is the point of it in plain text: {text}"
    );
    assert!(text.contains("[image: a picture]"));
    assert!(text.contains("- first"), "{text}");
    assert!(text.contains("1. nested"), "{text}");
    assert!(
        text.contains("> they wrote this"),
        "a quote is quoted the way every mail client writes one: {text}"
    );
}

#[test]
fn to_search_text_keeps_the_words_and_drops_the_addresses() {
    // #327: the index must not be told where a link goes, or a tracker URL makes a message answer
    // queries it should not.
    let text = every_shape().to_search_text();

    assert!(text.contains("plain, with bold and italic"));
    assert!(
        text.contains("a link"),
        "the words a person reads are still there: {text}"
    );
    assert!(
        !text.contains("example.com"),
        "a link's address is in the indexable text: {text}"
    );
    assert!(text.contains("a picture"), "alt text is content: {text}");
    assert!(
        !text.contains("[image"),
        "the placeholder is not something the message says: {text}"
    );
    assert!(
        every_shape()
            .to_text()
            .contains("a link <https://example.com/a?b=c&d=e>")
    );
}

#[test]
fn text_escaping_survives_a_round_trip() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![Inline::Text(
            "5 < 6 && \"quoted\" & <b>not bold</b>".to_owned(),
        )])],
    };

    let html = document.to_html();
    assert!(!html.contains("<b>"), "text became markup: {html}");
    assert_eq!(parse(&html), document);
}

#[test]
fn parsing_never_panics_on_anything() {
    for hostile in [
        "",
        "<",
        "<<<<<<",
        "<p><p><p><p>",
        "</p></div></html>",
        "<a href=",
        "<!-- unterminated",
        "<![CDATA[x]]>",
        "&#x0;&#xFFFF;&notanentity;",
        "<table><tr><td>cell</td></tr></table>",
        "<p>\u{0}\u{feff}text</p>",
    ] {
        let _ = parse(hostile).to_html();
        let _ = parse(hostile).to_text();
    }
}

#[test]
fn a_plain_document_is_recognised_and_a_structured_one_is_not() {
    assert!(Document::from_text("just words\nand a line break").is_plain_text());
    assert!(Document::new().is_plain_text());

    for structured in [
        Block::Heading {
            level: HeadingLevel::One,
            inlines: vec![Inline::Text("h".to_owned())],
        },
        Block::Quote(vec![]),
        Block::Rule,
        Block::Code {
            language: None,
            text: "x".to_owned(),
        },
        Block::Paragraph(vec![Inline::Strong(vec![Inline::Text("b".to_owned())])]),
        Block::Paragraph(vec![Inline::Link {
            href: Href::parse("https://example.com").unwrap(),
            inlines: vec![],
        }]),
    ] {
        let document = Document {
            blocks: vec![structured.clone()],
        };
        assert!(
            !document.is_plain_text(),
            "{structured:?} is not plain text, so it needs an HTML alternative"
        );
    }
}

#[test]
fn link_hosts_finds_a_link_nested_in_a_list_and_a_quote() {
    let document = Document {
        blocks: vec![
            Block::Paragraph(vec![Inline::Link {
                href: Href::parse("https://click.tracker.example.org/r?c=1").unwrap(),
                inlines: vec![Inline::Text("shop now".to_owned())],
            }]),
            Block::List {
                kind: ListKind::Bullet,
                items: vec![ListItem::new(vec![Block::Paragraph(vec![Inline::Link {
                    href: Href::parse("https://cdn.example.org/img").unwrap(),
                    inlines: vec![],
                }])])],
            },
            Block::Quote(vec![Block::Paragraph(vec![Inline::Strong(vec![
                Inline::Link {
                    href: Href::parse("https://nested.example.org").unwrap(),
                    inlines: vec![],
                },
            ])])]),
        ],
    };
    let mut hosts = document.link_hosts();
    hosts.sort();
    assert_eq!(
        hosts,
        [
            "cdn.example.org",
            "click.tracker.example.org",
            "nested.example.org",
        ]
    );
}

#[test]
fn link_hosts_is_case_insensitive_and_deduplicates() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![
            Inline::Link {
                href: Href::parse("https://Tracker.Example.org/a").unwrap(),
                inlines: vec![],
            },
            Inline::Link {
                href: Href::parse("https://tracker.example.org/b").unwrap(),
                inlines: vec![],
            },
        ])],
    };
    assert_eq!(document.link_hosts(), ["tracker.example.org"]);
}

#[test]
fn link_hosts_skips_a_mailto_link_since_it_names_no_host_to_compare() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![Inline::Link {
            href: Href::parse("mailto:ada@example.com").unwrap(),
            inlines: vec![],
        }])],
    };
    assert!(document.link_hosts().is_empty());
}

#[test]
fn a_document_with_no_links_has_no_hosts() {
    assert!(Document::from_text("just words").link_hosts().is_empty());
    assert!(Document::new().link_hosts().is_empty());
}

#[test]
fn an_empty_paragraph_records_nothing_and_is_narrowed_away() {
    // WebKit nests the new `<ul>` inside the paragraph; recovery leaves empty `<p>`s, which would
    // ship as blank `<p></p>`.
    let document = parse("<p><ul><li>item one</li></ul></p>");
    assert_eq!(
        document.blocks,
        vec![Block::List {
            kind: ListKind::Bullet,
            items: vec![ListItem::new(vec![Block::Paragraph(vec![Inline::Text(
                "item one".into()
            )])])],
        }]
    );
    let spaced = parse("<p>a</p><p><br></p><p>b</p>");
    assert_eq!(spaced.blocks.len(), 3, "the blank line is content");
}

#[test]
fn the_editor_form_round_trips_an_inline_image_through_the_editing_scheme() {
    // The editing shell's CSP allows only `img-src postio-cid:`, so the editor form uses that
    // scheme and the wire form stays `cid:`.
    let mut document = Document::new();
    document.blocks.push(Block::Paragraph(vec![
        Inline::Text("see ".to_owned()),
        Inline::Image {
            content_id: ContentId::parse("photo-1@postio.invalid").unwrap(),
            alt: "the lamp".to_owned(),
        },
    ]));

    let editor_html = document.editor_html();
    assert!(
        editor_html.contains("src=\"postio-cid:photo-1%40postio.invalid\""),
        "{editor_html}"
    );
    assert!(
        !editor_html.contains("src=\"cid:"),
        "the editor form must not carry the wire scheme: {editor_html}"
    );
    assert_eq!(parse(&editor_html), document);

    assert!(
        document.to_html().contains("src=\"cid:"),
        "{}",
        document.to_html()
    );
}

#[test]
fn a_postio_cid_source_that_smuggles_a_url_is_still_refused() {
    let document = parse("<p><img src=\"postio-cid:%2F%2Fevil.example%2Fx\" alt=\"\"></p>");
    assert!(
        !document
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Paragraph(inlines)
                if inlines.iter().any(|inline| matches!(inline, Inline::Image { .. })))),
        "{document:?}"
    );
}

#[test]
fn a_paragraph_short_enough_to_fit_wraps_to_nothing() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![Inline::Text(
            "Looking now.".to_owned(),
        )])],
    };
    assert_eq!(document.to_flowed_text(), "Looking now.");
}

#[test]
fn a_paragraph_longer_than_the_width_gets_soft_broken() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![Inline::Text(
            "one two three four five six seven eight nine ten".to_owned(),
        )])],
    };
    let flowed = document.to_flowed_text_at(20);

    let lines: Vec<&str> = flowed.split('\n').collect();
    assert!(lines.len() > 1, "should have wrapped at all: {flowed:?}");
    for line in &lines[..lines.len() - 1] {
        assert!(
            line.ends_with(' '),
            "every line but the last should carry a soft break: {line:?}"
        );
        assert!(
            line.chars().count() <= 20,
            "{line:?} is longer than the width"
        );
    }
    assert_eq!(
        postio_body::flowed::unwrap(&flowed, ""),
        "one two three four five six seven eight nine ten",
        "unwrapping should reconstruct the sentence exactly"
    );
}

#[test]
fn a_hard_break_stays_a_fixed_line_even_when_it_is_the_only_one() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![
            Inline::Text("first line".to_owned()),
            Inline::Break,
            Inline::Text("second line".to_owned()),
        ])],
    };
    let flowed = document.to_flowed_text();

    assert_eq!(
        flowed, "first line\nsecond line",
        "the hard break must not gain a trailing soft-break space of its own"
    );
}

#[test]
fn a_quote_is_wrapped_with_the_marker_counted_against_the_width() {
    let document = Document {
        blocks: vec![Block::Quote(vec![Block::Paragraph(vec![Inline::Text(
            "one two three four five six seven eight".to_owned(),
        )])])],
    };
    let flowed = document.to_flowed_text_at(20);

    for line in flowed.split('\n') {
        assert!(
            line.starts_with("> "),
            "every quoted line should carry the marker: {line:?}"
        );
        assert!(
            line.chars().count() <= 20,
            "{line:?} (with its marker) is longer than the width"
        );
    }
}

#[test]
fn a_nested_quote_uses_adjacent_markers_with_one_space_before_content() {
    let document = Document {
        blocks: vec![Block::Quote(vec![Block::Quote(vec![Block::Paragraph(
            vec![Inline::Text("deeply nested".to_owned())],
        )])])],
    };
    assert_eq!(
        document.to_flowed_text(),
        ">> deeply nested",
        "RFC 3676 markers are adjacent -- `>>`, never `> >`"
    );
}

#[test]
fn wrapping_and_unwrapping_a_paragraph_reconstructs_the_original_text() {
    for text in [
        "a short line",
        "a considerably longer paragraph that should wrap across several \
         physical lines once it is flowed at a narrow width for the test",
        "one two three four five six seven eight nine ten eleven twelve",
    ] {
        let wrapped = postio_body::flowed::wrap(text, "", 30);
        assert_eq!(postio_body::flowed::unwrap(&wrapped, ""), text);
    }
}

#[test]
fn a_soft_wrapped_paragraph_unwraps_back_to_one_line_of_text() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![Inline::Text(
            "one two three four five six seven eight nine ten".to_owned(),
        )])],
    };
    let flowed = document.to_flowed_text_at(20);
    assert!(
        flowed.contains('\n'),
        "sanity: this should actually have wrapped: {flowed:?}"
    );

    assert_eq!(Document::from_flowed_text(&flowed), document);
}

#[test]
fn a_hard_break_survives_the_round_trip_as_two_inlines_not_one() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![
            Inline::Text("first line".to_owned()),
            Inline::Break,
            Inline::Text("second line".to_owned()),
        ])],
    };
    let flowed = document.to_flowed_text();

    assert_eq!(Document::from_flowed_text(&flowed), document);
}

#[test]
fn a_leading_hard_break_with_nothing_typed_before_it_still_counts() {
    // A paragraph opening with a break has a span that unwraps to no text; it must not read as "no
    // span yet".
    let document = Document {
        blocks: vec![Block::Paragraph(vec![
            Inline::Break,
            Inline::Text("second line".to_owned()),
        ])],
    };
    let flowed = document.to_flowed_text();

    assert_eq!(Document::from_flowed_text(&flowed), document);
}

#[test]
fn two_paragraphs_stay_two_paragraphs_after_the_round_trip() {
    let document = Document {
        blocks: vec![
            Block::Paragraph(vec![Inline::Text(
                "one two three four five six seven eight nine ten eleven twelve".to_owned(),
            )]),
            Block::Paragraph(vec![Inline::Text("a short second paragraph".to_owned())]),
        ],
    };
    let flowed = document.to_flowed_text_at(20);

    assert_eq!(Document::from_flowed_text(&flowed), document);
}

#[test]
fn a_flowed_reply_body_unwraps_to_the_sentence_the_sender_actually_wrote() {
    let flowed = "That order works for me, though I would rather take the sign-off \nquestion first while everyone is still in the room and awake, because \nthe layout argument tends to eat the whole hour once it starts.";

    let document = Document::from_flowed_text(flowed);

    assert_eq!(
        document,
        Document {
            blocks: vec![Block::Paragraph(vec![Inline::Text(
                "That order works for me, though I would rather take the sign-off \
                 question first while everyone is still in the room and awake, because \
                 the layout argument tends to eat the whole hour once it starts."
                    .to_owned()
            )])],
        },
        "a reply built from this must not show the sender's own sentence as \
         three typed line breaks"
    );
}

#[test]
fn a_tables_cells_come_apart_in_the_plain_text_rendering() {
    // #1482: cells were concatenated with nothing between them, so
    // `<td>Gate</td><td>Interlock</td>` narrowed to `GateInterlock`.
    let document = parse(
        "<table>\
           <tr><td>Gate</td><td>Interlock</td></tr>\
           <tr><td>North</td><td>Armed</td></tr>\
         </table>",
    );
    let text = document.to_text();

    assert!(
        !text.contains("GateInterlock"),
        "the cells in a row ran together: {text:?}"
    );
    assert!(
        !text.contains("InterlockNorth"),
        "one row ran into the next: {text:?}"
    );
    for cell in ["Gate", "Interlock", "North", "Armed"] {
        assert!(text.contains(cell), "{cell} was lost entirely: {text:?}");
    }
}

#[test]
fn a_table_cell_does_not_gain_a_space_that_was_not_there() {
    let one = parse("<table><tr><td>Gate</td></tr></table>").to_text();
    assert_eq!(one.trim(), "Gate", "{one:?}");

    let spaced = parse("<table><tr><td>Gate </td><td>Interlock</td></tr></table>").to_text();
    assert!(
        !spaced.contains("Gate  Interlock"),
        "a cell that already ended in a space got a second one: {spaced:?}"
    );
}

#[test]
fn a_space_between_two_loose_inlines_is_content_not_formatting() {
    // #1486: `walk_blocks` dropped whitespace-only nodes, wrong between inlines (`<b>x</b>
    // <i>y</i>`);
    // the composer's DOM has loose inlines in `<body>` after a formatting command.
    assert_eq!(parse("<b>x</b> <i>y</i>").to_text(), "x y");
    assert_eq!(parse("<b>x</b>\u{a0}<i>y</i>").to_text(), "x y");

    assert_eq!(parse("   <b>x</b>").to_text(), "x");
    assert_eq!(parse("<p>one</p>\n  <p>two</p>").to_text(), "one\n\ntwo");
    assert_eq!(parse("<b>x</b>  <i>y</i>").to_text(), "x y");
}

#[test]
fn the_folds_own_chrome_is_not_part_of_the_message() {
    let folded = "<p>Acknowledged.</p>\
<details class=\"postio-quote\">\
<summary contenteditable=\"false\">Quoted message</summary>\
<blockquote data-postio-quoted=\"1\"><p>Do not reset.</p></blockquote>\
</details>";
    let text = parse(folded).to_text();

    assert!(
        !text.contains("Quoted message"),
        "the summary became content, so a reply carries a line nobody wrote: {text:?}"
    );
    assert!(text.contains("Acknowledged."), "{text:?}");
    assert!(
        text.contains("> Do not reset."),
        "the quote was lost: {text:?}"
    );
}

#[test]
fn a_plain_reply_turned_rich_quotes_what_it_quoted() {
    // A reply's plain field ends in `>` lines; as paragraphs they reached the editor as literal
    // angle brackets.
    let document =
        Document::from_text_quoting("Yes, six.\n\nOn Monday, Bo wrote:\n> Six?\n> >> Or seven?\n");
    let (_, html) = postio_body::render(&document);
    assert_eq!(
        html,
        "<p>Yes, six.</p><p>On Monday, Bo wrote:</p>\
         <blockquote><p>Six?</p><blockquote><blockquote><p>Or seven?</p>\
         </blockquote></blockquote></blockquote><p></p>"
    );
}

#[test]
fn text_with_no_quote_is_what_from_text_makes() {
    let text = "Line one\nline two\n\nNext.";
    assert_eq!(Document::from_text_quoting(text), Document::from_text(text));
}

mod subset {
    use postio_body::document::{
        Alignment, Block, Document, HeadingLevel, Inline, Language, ListItem, ListKind, Table,
        Task, WebHref,
    };
    use postio_body::parse;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    fn para(s: &str) -> Block {
        Block::Paragraph(vec![text(s)])
    }

    fn row(blocks: Vec<Block>, html: &str) {
        let document = Document { blocks };
        assert_eq!(document.to_html(), html, "the normal form moved");
        assert_eq!(parse(html), document, "{html} did not read back");
        assert_eq!(parse(html).to_html(), html, "to_html(parse(h)) != h");
    }

    #[test]
    fn every_heading_level_is_its_own_tag() {
        for level in HeadingLevel::ALL {
            let digit = level.digit();
            row(
                vec![Block::Heading {
                    level,
                    inlines: vec![text("h")],
                }],
                &format!("<h{digit}>h</h{digit}>"),
            );
        }
    }

    #[test]
    fn strikethrough_is_del_and_reads_s_and_strike() {
        let struck = vec![Block::Paragraph(vec![Inline::Strikethrough(vec![text(
            "x",
        )])])];
        row(struck.clone(), "<p><del>x</del></p>");
        assert_eq!(parse("<p><s>x</s></p>").blocks, struck);
        assert_eq!(parse("<p><strike>x</strike></p>").blocks, struck);
    }

    #[test]
    fn an_ordered_list_keeps_its_start() {
        let list = |kind| {
            vec![Block::List {
                kind,
                items: vec![ListItem::new(vec![para("a")])],
            }]
        };
        row(list(ListKind::Bullet), "<ul><li><p>a</p></li></ul>");
        row(list(ListKind::NUMBERED), "<ol><li><p>a</p></li></ol>");
        row(
            list(ListKind::Ordered { start: 7 }),
            "<ol start=\"7\"><li><p>a</p></li></ol>",
        );
        row(
            list(ListKind::Ordered { start: 0 }),
            "<ol start=\"0\"><li><p>a</p></li></ol>",
        );
    }

    #[test]
    fn a_task_list_carries_its_boxes_as_glyphs() {
        row(
            vec![Block::List {
                kind: ListKind::Bullet,
                items: vec![
                    ListItem::task(Task::Open, vec![para("open task")]),
                    ListItem::task(Task::Done, vec![para("done task")]),
                ],
            }],
            "<ul data-postio-task=\"list\" style=\"list-style-type:none\">\
             <li data-postio-task=\"open\"><p>\u{2610} open task</p></li>\
             <li data-postio-task=\"done\"><p>\u{2611} done task</p></li></ul>",
        );
        row(
            vec![Block::List {
                kind: ListKind::Bullet,
                items: vec![
                    ListItem::task(Task::Open, vec![para("t")]),
                    ListItem::new(vec![para("plain")]),
                ],
            }],
            "<ul><li data-postio-task=\"open\"><p>\u{2610} t</p></li><li><p>plain</p></li></ul>",
        );
    }

    #[test]
    fn a_glyph_without_the_marker_is_only_text() {
        let document = parse("<ul><li><p>\u{2610} not a task</p></li></ul>");
        let Block::List { items, .. } = &document.blocks[0] else {
            panic!("{document:?}")
        };
        assert_eq!(items[0].task, None);
        assert_eq!(items[0].blocks, vec![para("\u{2610} not a task")]);
    }

    #[test]
    fn a_github_checkbox_list_reads_as_tasks() {
        let document = parse(
            "<ul><li><input type=\"checkbox\" disabled> open</li>\
             <li><input type=\"checkbox\" checked disabled> done</li></ul>",
        );
        let Block::List { items, .. } = &document.blocks[0] else {
            panic!("{document:?}")
        };
        assert_eq!(items[0], ListItem::task(Task::Open, vec![para("open")]));
        assert_eq!(items[1], ListItem::task(Task::Done, vec![para("done")]));
    }

    #[test]
    fn a_code_block_keeps_its_language() {
        row(
            vec![Block::Code {
                language: Language::parse("rust"),
                text: "fn main() {}\n".to_owned(),
            }],
            "<pre data-postio-lang=\"rust\">fn main() {}\n</pre>",
        );
        row(
            vec![Block::Code {
                language: None,
                text: "x".to_owned(),
            }],
            "<pre>x</pre>",
        );
        assert_eq!(
            parse("<pre data-postio-lang=\"a&quot; b\">x</pre>").blocks,
            vec![Block::Code {
                language: None,
                text: "x".to_owned()
            }]
        );
    }

    #[test]
    fn a_table_is_a_table_with_its_layout_attributes() {
        let table = Table::new(
            vec![Alignment::Left, Alignment::Right],
            vec![vec![text("Item")], vec![text("Qty")]],
            vec![vec![vec![text("Widget")], vec![text("3")]]],
        );
        row(
            vec![Block::Table(table)],
            "<table border=\"1\" cellpadding=\"6\" cellspacing=\"0\"><thead><tr>\
             <th align=\"left\">Item</th><th align=\"right\">Qty</th></tr></thead><tbody><tr>\
             <td align=\"left\">Widget</td><td align=\"right\">3</td></tr></tbody></table>",
        );
        let plain = Table::new(
            vec![Alignment::None, Alignment::Center],
            vec![vec![text("a")], vec![]],
            vec![],
        );
        row(
            vec![Block::Table(plain)],
            "<table border=\"1\" cellpadding=\"6\" cellspacing=\"0\"><thead><tr>\
             <th>a</th><th align=\"center\"></th></tr></thead><tbody></tbody></table>",
        );
    }

    #[test]
    fn a_pasted_table_without_a_header_reads_its_first_row_as_one() {
        let document = parse("<table><tr><td>a</td><td>b</td></tr><tr><td>1</td></tr></table>");
        assert_eq!(
            document.blocks,
            vec![Block::Table(Table::new(
                vec![],
                vec![vec![text("a")], vec![text("b")]],
                vec![vec![vec![text("1")], vec![]]],
            ))]
        );
    }

    #[test]
    fn a_spanning_table_narrows_to_paragraphs() {
        let document = parse(
            "<table><tr><td colspan=\"2\">Gate</td></tr><tr><td>a</td><td>b</td></tr></table>",
        );
        assert!(
            document
                .blocks
                .iter()
                .all(|block| matches!(block, Block::Paragraph(_))),
            "{document:?}"
        );
        assert_eq!(document.to_text(), "Gate\n\na b");
    }

    #[test]
    fn a_table_with_a_block_in_a_cell_narrows_to_paragraphs() {
        let document = parse("<table><tr><td><ul><li>x</li></ul></td></tr></table>");
        assert!(
            !document
                .blocks
                .iter()
                .any(|block| matches!(block, Block::Table(_))),
            "{document:?}"
        );
    }

    #[test]
    fn a_remote_image_is_a_marked_link_and_a_remote_img_still_has_no_shape() {
        let image = Inline::RemoteImage {
            href: WebHref::parse("https://example.com/x.png").unwrap(),
            alt: "x".to_owned(),
        };
        row(
            vec![Block::Paragraph(vec![image])],
            "<p><a href=\"https://example.com/x.png\" data-postio-remote-image=\"\">x</a></p>",
        );
        let pasted = parse("<p>a<img src=\"https://example.com/x.png\" alt=\"x\">b</p>");
        assert_eq!(
            pasted.to_html(),
            "<p>ab</p>",
            "the remote <img> narrowed away"
        );
        assert!(matches!(
            &parse("<p><a href=\"mailto:ada@example.com\" data-postio-remote-image=\"\">x</a></p>")
                .blocks[0],
            Block::Paragraph(inlines) if matches!(inlines[0], Inline::Link { .. })
        ));
    }
}

#[test]
fn the_composer_dialect_is_what_to_html_writes() {
    let document = every_shape();
    let html = document.to_html();
    assert_eq!(parse(&html), document, "the dialect round-trips");
    for foreign in [
        "<span",
        "<div",
        "<font",
        " style=",
        " class=",
        "<b>",
        "<i>",
        "<script",
        "<br/>",
        "contenteditable",
        "&nbsp;",
    ] {
        assert!(
            !html.contains(foreign),
            "the dialect holds {foreign}: {html}"
        );
    }
    let webkit = "<div><b>bold</b> and <i>italic</i></div><div><br></div><p>next</p>";
    assert_eq!(
        parse(&parse(webkit).to_html()),
        parse(webkit),
        "WebKit's own markup narrows once and stays narrowed"
    );
}
