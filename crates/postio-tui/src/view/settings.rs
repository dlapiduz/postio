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

/// The widest a connection's "who and where" is set.
const CONNECTION: usize = 44;

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
            let (mark, role) = match (here, settings.in_list()) {
                (true, false) => ("› ", Role::Surface),
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
        let cursor = settings.row(accounts.len());
        for (index, account) in accounts.iter().enumerate() {
            let here = settings.in_list() && index == cursor;
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
                    theme.style(if here { Role::Surface } else { Role::Text }),
                ),
                Span::styled(format!("  {state}"), theme.style(Role::Dim)),
            ]));
        }
        pane.push(Line::default());
        match settings
            .signatures_of()
            .and_then(|id| accounts.iter().find(|account| account.id == id))
        {
            Some(account) => signatures(&mut pane, account, settings, columns, theme),
            None => pane.push(Line::styled(
                fit(
                    "Tab to the accounts · Enter on or off · d remove · c sign in again · \
                     r rebuild index · m default · M map a folder · s signatures",
                    columns,
                ),
                theme.style(Role::Dim),
            )),
        }
    } else if section == Section::Privacy {
        privacy(&mut pane, app, settings.in_list(), columns, theme);
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

/// An account's signatures, under the accounts: each name with the first
/// line of its text, and the keys that work on them.
fn signatures(
    pane: &mut Vec<Line>,
    account: &postio_model::Account,
    settings: &crate::settings::Settings,
    columns: usize,
    theme: &Theme,
) {
    let address = SafeText::new(&account.address.address);
    pane.push(Line::styled(
        fit(&format!("SIGNATURES · {address}"), columns),
        theme.style(Role::Dim).add_modifier(Modifier::BOLD),
    ));
    if account.signatures.is_empty() {
        pane.push(Line::styled("  None yet.", theme.style(Role::Dim)));
    }
    let cursor = settings.signature(account.signatures.len());
    for (index, signature) in account.signatures.iter().enumerate() {
        let here = index == cursor;
        let name = SafeText::new(&signature.name);
        let first = SafeText::new(signature.text.lines().next().unwrap_or_default());
        pane.push(Line::from(vec![
            Span::styled(if here { "› " } else { "  " }, theme.style(Role::Accent)),
            Span::styled(
                format!("{:<20}", fit(name.as_str(), 20)),
                theme.style(if here { Role::Surface } else { Role::Text }),
            ),
            Span::styled(
                fit(first.as_str(), columns.saturating_sub(24)),
                theme.style(Role::Dim),
            ),
        ]));
    }
    pane.push(Line::default());
    pane.push(Line::styled(
        fit(
            "Enter writes it in $EDITOR · n new · r rename · d delete · Esc back",
            columns,
        ),
        theme.style(Role::Dim),
    ));
}

/// The privacy pane, as the desktop's: who may load remote images, the
/// lists left, the read receipts asked for, and every connection made. All
/// of it is read back from the store, and none of it is in the file.
fn privacy(pane: &mut Vec<Line>, app: &App, in_list: bool, columns: usize, theme: &Theme) {
    use postio_ui::privacy;
    let kicker = |text: &'static str| {
        Line::styled(text, theme.style(Role::Dim).add_modifier(Modifier::BOLD))
    };
    let empty = |text: &'static str| {
        Line::styled(fit(&format!("  {text}"), columns), theme.style(Role::Dim))
    };
    let log = app.privacy();

    pane.push(kicker(privacy::ALLOWED));
    let senders: Vec<&str> = app.allowlist().senders().collect();
    if senders.is_empty() {
        pane.push(empty(privacy::NO_ALLOWED));
    }
    let cursor = app
        .settings()
        .map_or(0, |settings| settings.row(senders.len()));
    for (index, sender) in senders.iter().enumerate() {
        let here = in_list && index == cursor;
        // An address the person allowed, but it came from a message.
        let sender = SafeText::new(sender);
        pane.push(Line::from(vec![
            Span::styled(if here { "› " } else { "  " }, theme.style(Role::Accent)),
            Span::styled(
                fit(sender.as_str(), columns.saturating_sub(2)),
                theme.style(if here { Role::Surface } else { Role::Text }),
            ),
        ]));
    }

    pane.push(Line::default());
    pane.push(kicker(privacy::LISTS_LEFT));
    let left = log
        .map(|log| log.log.activations.as_slice())
        .unwrap_or_default();
    if left.is_empty() {
        pane.push(empty(privacy::NO_LISTS_LEFT));
    }
    for activation in left {
        let when = activation
            .activated_at
            .format(privacy::LEFT_WHEN)
            .to_string();
        let list = SafeText::new(&activation.list_identifier);
        pane.push(Line::from(vec![
            Span::styled(format!("  {when}  "), theme.style(Role::Dim)),
            Span::styled(
                fit(list.as_str(), columns.saturating_sub(when.len() + 4)),
                theme.style(Role::Text),
            ),
        ]));
    }

    pane.push(Line::default());
    pane.push(kicker(privacy::READ_RECEIPTS));
    pane.push(Line::styled(
        fit(
            &format!(
                "  {}",
                privacy::read_receipts(log.map_or(0, |log| log.log.read_receipts))
            ),
            columns,
        ),
        theme.style(Role::Text),
    ));

    pane.push(Line::default());
    pane.push(Line::styled(
        fit(
            "Tab to the senders · d asks for remote images again",
            columns,
        ),
        theme.style(Role::Dim),
    ));

    pane.push(Line::default());
    pane.push(kicker(privacy::CONNECTIONS));
    let connections = log
        .map(|log| log.connections.as_slice())
        .unwrap_or_default();
    if connections.is_empty() {
        pane.push(empty(privacy::NO_CONNECTIONS));
    }
    for connection in connections {
        let when = connection
            .at
            .with_timezone(&chrono::Local)
            .format(privacy::CONNECTION_WHEN)
            .to_string();
        let outcome = connection.outcome.as_str();
        // The host is what a server or an autoconfig answer named.
        let what = SafeText::new(&privacy::connection(connection)).to_string();
        // A column wide enough for a host, not the whole pane: the outcome
        // belongs beside what it is about.
        let room = columns
            .saturating_sub(when.len() + outcome.len() + 6)
            .min(CONNECTION);
        let what = fit(&what, room);
        let pad = room.saturating_sub(unicode_width::UnicodeWidthStr::width(what.as_str()));
        pane.push(Line::from(vec![
            Span::styled(format!("  {when}  "), theme.style(Role::Dim)),
            Span::styled(what, theme.style(Role::Text)),
            Span::raw(" ".repeat(pad + 2)),
            Span::styled(
                outcome,
                theme.style(match connection.outcome {
                    postio_model::egress::EgressOutcome::Connected => Role::Dim,
                    postio_model::egress::EgressOutcome::Failed => Role::Warning,
                }),
            ),
        ]));
    }
}
