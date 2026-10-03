//! The strip under the top bar: where the list is, and how much is in it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::app::App;
use crate::theme::{Role, Theme};
use crate::view::fit;

/// Draw the strip into `area`, one row, on the surface background.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    frame
        .buffer_mut()
        .set_style(area, theme.style(Role::Surface));
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            fit(app.place_name().as_str(), usize::from(area.width)),
            theme
                .style(Role::Text)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(" ▾", theme.style(Role::Dim)),
    ];
    if let Some(key) = app.hint(postio_core::CommandId::GoToFolders) {
        spans.push(Span::styled(format!(" {key}"), theme.style(Role::Accent)));
    }
    let count = postio_ui::focus_row::strip_counts(app.total(), 0);
    spans.push(Span::styled(format!("   {count}"), theme.style(Role::Dim)));
    frame.render_widget(Line::from(spans), area);
}
