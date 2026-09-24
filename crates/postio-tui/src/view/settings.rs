//! The settings, drawn: the desktop's navigation, and the section under it
//! (US7).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::app::App;
use crate::theme::{Role, Theme};
use crate::view::fit;
use postio_ui::settings::{Group, Section};
use postio_ui::terminal::SafeText;

/// The navigation's width.
const NAV: u16 = 26;

/// Draw the settings over `area`.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(settings) = app.settings() else {
        return;
    };
    let nav_width = NAV.min(area.width / 2);
    let mut nav: Vec<Line> = vec![Line::styled(
        "Settings",
        theme.style(Role::Text).add_modifier(Modifier::BOLD),
    )];
    for group in Group::ALL {
        nav.push(Line::default());
        nav.push(Line::styled(
            group.label(),
            theme.style(Role::Dim).add_modifier(Modifier::BOLD),
        ));
        for section in settings
            .sections()
            .iter()
            .filter(|section| section.group() == group)
        {
            let here = *section == settings.current();
            let (mark, role) = match (here, settings.in_accounts()) {
                (true, false) => ("› ", Role::Selection),
                (true, true) => ("› ", Role::Accent),
                _ => ("  ", Role::Text),
            };
            nav.push(Line::styled(
                fit(
                    &format!("{mark}{}", section.label()),
                    usize::from(nav_width),
                ),
                theme.style(role),
            ));
        }
    }
    for (offset, line) in nav.into_iter().enumerate() {
        let row = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        if row >= area.y + area.height {
            break;
        }
        frame.render_widget(line, Rect::new(area.x, row, nav_width, 1));
    }

    let x = area.x + nav_width + 2;
    let width = area.width.saturating_sub(nav_width + 2);
    let columns = usize::from(width);
    let section = settings.current();
    let mut pane: Vec<Line> = vec![
        Line::styled(
            section.label(),
            theme.style(Role::Text).add_modifier(Modifier::BOLD),
        ),
        Line::styled(fit(section.description(), columns), theme.style(Role::Dim)),
        Line::default(),
    ];
    if section == Section::Accounts {
        let accounts = app.accounts();
        let cursor = settings.account(accounts.len());
        for (index, account) in accounts.iter().enumerate() {
            let here = settings.in_accounts() && index == cursor;
            let state = match (account.enabled, account.is_default) {
                (false, _) => "disabled",
                (true, true) => "default",
                (true, false) => "",
            };
            // An account's name is the person's own, but it is still text
            // from outside the program.
            let name = SafeText::new(&account.display_name);
            let address = SafeText::new(&account.address.address);
            pane.push(Line::from(vec![
                Span::styled(if here { "› " } else { "  " }, theme.style(Role::Accent)),
                Span::styled(
                    fit(
                        &format!("{:<28} {}", address.as_str(), name.as_str()),
                        columns.saturating_sub(14),
                    ),
                    theme.style(if here { Role::Selection } else { Role::Text }),
                ),
                Span::styled(format!("  {state}"), theme.style(Role::Dim)),
            ]));
        }
        pane.push(Line::default());
        pane.push(Line::styled(
            fit(
                "Tab to the accounts · Enter on or off · d remove · c sign in again · \
                 r rebuild index · m default · M map a folder",
                columns,
            ),
            theme.style(Role::Dim),
        ));
    } else {
        let table = section.table().unwrap_or("the whole file");
        pane.push(Line::styled(
            fit(
                &format!("Lives in config.toml, {table}. Enter edits it there, in $EDITOR."),
                columns,
            ),
            theme.style(Role::Text),
        ));
    }
    for (offset, line) in pane.into_iter().enumerate() {
        let row = area.y + u16::try_from(offset).unwrap_or(u16::MAX);
        if row >= area.y + area.height {
            break;
        }
        frame.render_widget(line, Rect::new(x, row, width, 1));
    }
}
