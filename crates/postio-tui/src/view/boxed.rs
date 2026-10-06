//! A small rounded box over the window, for the questions and the forms
//! that are not the open message: the sweep's question, the rule dialog and
//! the capture frame.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::theme::{Role, Theme};
use crate::view::hit::{Hits, Target};

/// A box `width` by `height` cells, centred in `area` and no larger than
/// it, drawn over whatever is there, with `title` in its top edge. Clicks on
/// it land on nothing underneath. Answers what is inside the border.
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
