//! A small rounded box over the window, for the questions and the forms
//! that are not the open message: the sweep's question, the rule dialog and
//! the capture frame.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::theme::{Role, Theme};
use crate::view::hit::{Hits, Target};

/// A box `width` by `height` cells, centred in `area` and no larger than
/// it, drawn over whatever is there, with `title` in its top edge. A click
/// anywhere on the screen lands on nothing underneath. Answers what is inside
/// the border.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    width: u16,
    height: u16,
    title: &str,
    theme: &Theme,
    hits: &mut Hits,
) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let outer = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 3,
        width,
        height,
    );
    // Everything under a box, the top bar too, takes no click: the box has
    // the keyboard as it has the pointer.
    hits.add(frame.area(), Target::Overlay);
    frame.render_widget(Clear, outer);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.style(Role::Dim));
    if !title.is_empty() {
        block = block.title(format!(" {title} "));
    }
    let inside = block.inner(outer);
    frame.render_widget(block, outer);
    hits.add(outer, Target::Overlay);
    inside
}

/// One button of a box: its words, the key that runs it, what a click on it
/// is, and whether it is the one that does the thing.
pub struct Button {
    /// Its words.
    pub words: String,
    /// The key that runs it, as it is drawn; empty for none.
    pub key: String,
    /// What a click on it stands for.
    pub target: Target,
    /// Whether it is the box's own act, drawn bold.
    pub primary: bool,
}

/// `buttons` on the row `y`, side by side and centred in `inside`, each
/// `[ words key ]` and each a click.
pub fn buttons(
    frame: &mut Frame,
    inside: Rect,
    y: u16,
    buttons: &[Button],
    theme: &Theme,
    hits: &mut Hits,
) {
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Span};
    use unicode_width::UnicodeWidthStr;
    let widths: Vec<usize> = buttons
        .iter()
        .map(|button| {
            button.words.width()
                + 4
                + if button.key.is_empty() {
                    0
                } else {
                    1 + button.key.width()
                }
        })
        .collect();
    let total: usize = widths.iter().sum::<usize>() + 2 * buttons.len().saturating_sub(1);
    let mut x =
        inside.x + u16::try_from(usize::from(inside.width).saturating_sub(total) / 2).unwrap_or(0);
    for (button, width) in buttons.iter().zip(widths) {
        let style = if button.primary {
            theme.style(Role::Text).add_modifier(Modifier::BOLD)
        } else {
            theme.style(Role::Text)
        };
        let mut spans = vec![Span::styled(format!("[ {}", button.words), style)];
        if !button.key.is_empty() {
            spans.push(Span::styled(
                format!(" {}", button.key),
                theme.style(Role::Accent),
            ));
        }
        spans.push(Span::styled(" ]", style));
        let w = u16::try_from(width).unwrap_or(0);
        frame.render_widget(Line::from(spans), Rect::new(x, y, w, 1));
        hits.add(Rect::new(x, y, w, 1), button.target);
        x += w + 2;
    }
}
