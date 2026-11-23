use postio_core::CommandId;
use postio_ui::terminal::SafeText;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::App;
use crate::surface::Part;
use crate::theme::{Role, Theme};
use crate::view::bottom::cap;
use crate::view::boxed::{self, Button};
use crate::view::hit::{Hits, Target};
use crate::view::wrap::wrap;

pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, hits: &mut Hits) {
    let Some(question) = app.confirm() else {
        return;
    };
    let width = 76.min(area.width);
    let body = SafeText::new(&question.body);
    let lines = wrap(
        &Line::styled(body.as_str().to_owned(), theme.style(Role::Text)),
        usize::from(width).saturating_sub(4).max(1),
    );
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(5);
    let heading = SafeText::new(&question.heading);
    let inside = boxed::draw(frame, area, width, height, heading.as_str(), theme, hits);
    for (offset, line) in lines.into_iter().enumerate() {
        let y = inside.y + 1 + u16::try_from(offset).unwrap_or(0);
        if y >= inside.y + inside.height {
            break;
        }
        frame.render_widget(
            line,
            Rect::new(inside.x + 1, y, inside.width.saturating_sub(2), 1),
        );
    }
    let key = |command| app.hint(command).map(|key| cap(&key)).unwrap_or_default();
    boxed::buttons(
        frame,
        inside,
        inside.y + inside.height.saturating_sub(1),
        &[
            Button {
                words: "Cancel".to_owned(),
                key: key(CommandId::Back),
                target: Target::Surface(Part::ConfirmCancel, 0),
                primary: false,
            },
            Button {
                words: question.confirm.clone(),
                key: key(CommandId::OpenMessage),
                target: Target::Surface(Part::ConfirmYes, 0),
                primary: true,
            },
        ],
        theme,
        hits,
    );
}
