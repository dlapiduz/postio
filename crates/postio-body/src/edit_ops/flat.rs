use std::rc::Rc;

use crate::document::{Block, ContentId, Href, Inline, WebHref};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MarkSet {
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: Option<Href>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Atom {
    Char(char),
    Break,
    Image { content_id: ContentId, alt: String },
    Remote { href: WebHref, alt: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cell {
    pub atom: Atom,
    pub marks: Rc<MarkSet>,
}

pub(crate) fn flatten(inlines: &[Inline]) -> Vec<Cell> {
    let mut out = Vec::new();
    walk(inlines, &Rc::new(MarkSet::default()), &mut out);
    out
}

fn walk(inlines: &[Inline], marks: &Rc<MarkSet>, out: &mut Vec<Cell>) {
    let with = |change: &dyn Fn(&mut MarkSet)| {
        let mut next = MarkSet::clone(marks);
        change(&mut next);
        Rc::new(next)
    };
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.extend(text.chars().map(|c| Cell {
                atom: Atom::Char(c),
                marks: marks.clone(),
            })),
            Inline::Code(text) => {
                let code = with(&|m| m.code = true);
                out.extend(text.chars().map(|c| Cell {
                    atom: Atom::Char(c),
                    marks: code.clone(),
                }));
            }
            Inline::Strong(inner) => walk(inner, &with(&|m| m.strong = true), out),
            Inline::Emphasis(inner) => walk(inner, &with(&|m| m.emphasis = true), out),
            Inline::Strikethrough(inner) => walk(inner, &with(&|m| m.strikethrough = true), out),
            Inline::Link { href, inlines } => {
                walk(inlines, &with(&|m| m.link = Some(href.clone())), out)
            }
            Inline::Image { content_id, alt } => out.push(Cell {
                atom: Atom::Image {
                    content_id: content_id.clone(),
                    alt: alt.clone(),
                },
                marks: marks.clone(),
            }),
            Inline::RemoteImage { href, alt } => out.push(Cell {
                atom: Atom::Remote {
                    href: href.clone(),
                    alt: alt.clone(),
                },
                marks: marks.clone(),
            }),
            Inline::Break => out.push(Cell {
                atom: Atom::Break,
                marks: marks.clone(),
            }),
        }
    }
}

#[derive(Clone, Copy)]
enum Level {
    Link,
    Strong,
    Emphasis,
    Strikethrough,
    Leaf,
}

pub(crate) fn build(cells: &[Cell]) -> Vec<Inline> {
    let mut out = Vec::new();
    nest(cells, Level::Link, &mut out);
    out
}

fn nest(cells: &[Cell], level: Level, out: &mut Vec<Inline>) {
    let mut rest = cells;
    while let Some(first) = rest.first() {
        let run = rest
            .iter()
            .take_while(|cell| same_at(level, &cell.marks, &first.marks))
            .count();
        let (group, tail) = rest.split_at(run);
        rest = tail;
        let marks = &first.marks;
        match level {
            Level::Link => match &marks.link {
                Some(href) => {
                    let mut inner = Vec::new();
                    nest(group, Level::Strong, &mut inner);
                    out.push(Inline::Link {
                        href: href.clone(),
                        inlines: inner,
                    });
                }
                None => nest(group, Level::Strong, out),
            },
            Level::Strong => wrap(group, marks.strong, Level::Emphasis, Inline::Strong, out),
            Level::Emphasis => wrap(
                group,
                marks.emphasis,
                Level::Strikethrough,
                Inline::Emphasis,
                out,
            ),
            Level::Strikethrough => wrap(
                group,
                marks.strikethrough,
                Level::Leaf,
                Inline::Strikethrough,
                out,
            ),
            Level::Leaf => leaves(group, marks.code, out),
        }
    }
}

fn wrap(
    group: &[Cell],
    on: bool,
    next: Level,
    make: fn(Vec<Inline>) -> Inline,
    out: &mut Vec<Inline>,
) {
    if on {
        let mut inner = Vec::new();
        nest(group, next, &mut inner);
        out.push(make(inner));
    } else {
        nest(group, next, out);
    }
}

fn leaves(group: &[Cell], code: bool, out: &mut Vec<Inline>) {
    for cell in group {
        match &cell.atom {
            Atom::Char(c) => match (out.last_mut(), code) {
                (Some(Inline::Code(text)), true) | (Some(Inline::Text(text)), false) => {
                    text.push(*c)
                }
                _ if code => out.push(Inline::Code(c.to_string())),
                _ => out.push(Inline::Text(c.to_string())),
            },
            Atom::Break => out.push(Inline::Break),
            Atom::Image { content_id, alt } => out.push(Inline::Image {
                content_id: content_id.clone(),
                alt: alt.clone(),
            }),
            Atom::Remote { href, alt } => out.push(Inline::RemoteImage {
                href: href.clone(),
                alt: alt.clone(),
            }),
        }
    }
}

fn same_at(level: Level, a: &MarkSet, b: &MarkSet) -> bool {
    match level {
        Level::Link => a.link == b.link,
        Level::Strong => a.strong == b.strong,
        Level::Emphasis => a.emphasis == b.emphasis,
        Level::Strikethrough => a.strikethrough == b.strikethrough,
        Level::Leaf => a.code == b.code,
    }
}

pub(crate) fn of_block(block: &Block) -> Option<Vec<Cell>> {
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => Some(flatten(inlines)),
        Block::Code { text, .. } => {
            let plain = Rc::new(MarkSet::default());
            Some(
                text.chars()
                    .map(|c| Cell {
                        atom: if c == '\n' {
                            Atom::Break
                        } else {
                            Atom::Char(c)
                        },
                        marks: plain.clone(),
                    })
                    .collect(),
            )
        }
        _ => None,
    }
}

pub(crate) fn set_block(block: &mut Block, cells: &[Cell]) {
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => *inlines = build(cells),
        Block::Code { text, .. } => *text = plain_text(cells),
        _ => {}
    }
}

pub(crate) fn plain_text(cells: &[Cell]) -> String {
    cells
        .iter()
        .filter_map(|cell| match cell.atom {
            Atom::Char(c) => Some(c),
            Atom::Break => Some('\n'),
            Atom::Image { .. } | Atom::Remote { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    #[test]
    fn a_block_in_normal_form_comes_back_unchanged() {
        let href = Href::parse("https://example.com/").unwrap();
        let inlines = vec![
            text("a "),
            Inline::Link {
                href,
                inlines: vec![
                    Inline::Strong(vec![text("b"), Inline::Emphasis(vec![text("c")])]),
                    text("d"),
                ],
            },
            Inline::Code("e()".to_owned()),
            Inline::Break,
            Inline::Strikethrough(vec![text("f")]),
        ];
        assert_eq!(build(&flatten(&inlines)), inlines);
    }

    #[test]
    fn rebuilding_merges_neighbours_and_drops_empty_wrappers() {
        let inlines = vec![
            text("a"),
            text("b"),
            Inline::Strong(vec![]),
            Inline::Emphasis(vec![text("c")]),
            Inline::Emphasis(vec![text("d")]),
        ];
        assert_eq!(
            build(&flatten(&inlines)),
            vec![text("ab"), Inline::Emphasis(vec![text("cd")])]
        );
    }
}
