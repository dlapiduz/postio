//! Spec 014's golden document (`contracts/wire.md`): every construct the
//! composer authors, in one document, for every suite that asserts what a
//! formatted message is -- the wire, the reader, the send, the draft.
//!
//! Test-only (`test-golden`), so nothing ships in a normal build.

use crate::document::{
    Alignment, Block, Document, HeadingLevel, Href, Inline, Language, ListItem, ListKind, Table,
    Task,
};

/// The Markdown that reads into [`golden`], as the contract writes it.
pub const GOLDEN_MARKDOWN: &str = "\
# Plan

Some **bold**, *italic*, ~~struck~~ and `code`, and a [link](https://example.com/x).

## Steps

1. first
2. second
   - nested
- [ ] open task
- [x] done task

7. seventh

> quoted
>
> > nested quote

```rust
fn main() {}
```

---

| Item | Qty |
|:-----|----:|
| Widget | 3 |

line one\\
line two
";

fn text(s: &str) -> Inline {
    Inline::Text(s.to_owned())
}

fn para(s: &str) -> Block {
    Block::Paragraph(vec![text(s)])
}

fn item(s: &str) -> ListItem {
    ListItem::new(vec![para(s)])
}

/// `contracts/wire.md`'s document.
pub fn golden() -> Document {
    Document {
        blocks: vec![
            Block::Heading {
                level: HeadingLevel::One,
                inlines: vec![text("Plan")],
            },
            Block::Paragraph(vec![
                text("Some "),
                Inline::Strong(vec![text("bold")]),
                text(", "),
                Inline::Emphasis(vec![text("italic")]),
                text(", "),
                Inline::Strikethrough(vec![text("struck")]),
                text(" and "),
                Inline::Code("code".to_owned()),
                text(", and a "),
                Inline::Link {
                    href: Href::parse("https://example.com/x").unwrap(),
                    inlines: vec![text("link")],
                },
                text("."),
            ]),
            Block::Heading {
                level: HeadingLevel::Two,
                inlines: vec![text("Steps")],
            },
            Block::List {
                kind: ListKind::NUMBERED,
                items: vec![
                    item("first"),
                    ListItem::new(vec![
                        para("second"),
                        Block::List {
                            kind: ListKind::Bullet,
                            items: vec![item("nested")],
                        },
                    ]),
                ],
            },
            Block::List {
                kind: ListKind::Bullet,
                items: vec![
                    ListItem::task(Task::Open, vec![para("open task")]),
                    ListItem::task(Task::Done, vec![para("done task")]),
                ],
            },
            Block::List {
                kind: ListKind::Ordered { start: 7 },
                items: vec![item("seventh")],
            },
            Block::Quote(vec![
                para("quoted"),
                Block::Quote(vec![para("nested quote")]),
            ]),
            Block::Code {
                language: Language::parse("rust"),
                text: "fn main() {}\n".to_owned(),
            },
            Block::Rule,
            Block::Table(Table::new(
                vec![Alignment::Left, Alignment::Right],
                vec![vec![text("Item")], vec![text("Qty")]],
                vec![vec![vec![text("Widget")], vec![text("3")]]],
            )),
            Block::Paragraph(vec![text("line one"), Inline::Break, text("line two")]),
        ],
    }
}
