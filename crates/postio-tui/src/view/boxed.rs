//! A small rounded box over the window, for questions and forms.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::theme::{Role, Theme};
use crate::view::hit::{Hits, Target};

/// Centred in `area`, no larger than it, with `title` in its top edge. Answers the inside.
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

pub struct Button {
    pub words: String,
    pub key: String,
    pub target: Target,
    pub primary: bool,
}

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
