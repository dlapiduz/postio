//! One harness for driving the terminal app without a terminal.
//!
//! Every test that needs an [`App`] with mail in it -- the app's own, the
//! view's, the `shot` example's -- builds it here, so a change to how a list
//! is opened, paged or read is made once. The functions are the ways a person
//! gets to a screen: the app at a size, the places it lists, a list opened
//! and its pages served, a message opened and read, keys and clicks, and the
//! screen that results as text.
//!
//! Compiled for this crate's tests and, through the `test-support` feature,
//! for `examples/shot.rs`. Every name and address is fictional and on a
//! reserved domain.

use chrono::{DateTime, Local, TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use postio_model::mailbox::{Mailbox, MailboxRole};
use postio_model::{
    Account, AccountId, EmailAddress, ListScope, MailboxId, MessageBody, MessageId,
};
use postio_ui::paging::{Fetch, Page};
use postio_ui::terminal::SafeText;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::app::{App, Effect, Input, Pointer, update};
use crate::caps::{Background, Colour};
use crate::input::Keys;
use crate::places::{Places, Saved};
use crate::row::Row;
use crate::theme::Theme;
use crate::view::{draw, hit};

/// The instant every drawn screen thinks it is.
pub fn now() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap()
}

// -- The app ---------------------------------------------------------------

/// An app of `size` under the default keymap.
pub fn app(size: (u16, u16)) -> App {
    app_with_keys(size, &Default::default())
}

/// An app of `size` under the default keymap with `bindings` on top.
pub fn app_with_keys(size: (u16, u16), bindings: &postio_config::KeyBindings) -> App {
    App::new(size, Keys::new(&postio_core::Keymap::resolve(bindings)).0)
}

// -- Places ----------------------------------------------------------------

/// The one account the fixtures have.
pub fn account() -> Account {
    let mut account = Account::new("ada", EmailAddress::new(None::<String>, "ada@example.com"));
    account.id = AccountId::new(1);
    account.enabled = true;
    account
}

/// A selectable folder of [`account`].
pub fn folder(id: i64, name: &str, role: MailboxRole, unread: u32) -> Mailbox {
    let mut folder = Mailbox::new(AccountId::new(1), name, None);
    folder.id = MailboxId::new(id);
    folder.role = role;
    folder.selectable = true;
    folder.counts.unread = unread;
    folder
}

/// An Inbox (id 1) and an Archive (id 2), as the places a small mailbox has.
pub fn places() -> Places {
    Places {
        accounts: vec![account()],
        folders: vec![
            folder(1, "INBOX", MailboxRole::Inbox, 0),
            folder(2, "Archive", MailboxRole::Archive, 0),
        ],
        counts: Vec::new(),
        saved: Vec::new(),
        features: Default::default(),
    }
}

/// A saved search, as `[filters]` pins one.
pub fn saved_search(key: &str, name: &str, query: &str) -> Saved {
    Saved {
        key: key.into(),
        name: name.into(),
        query: query.into(),
    }
}

/// Tell `app` about `contents`.
pub fn seed_places(app: &mut App, contents: Places) {
    update(app, Input::Places(contents));
}

// -- Lists -----------------------------------------------------------------

/// One synthetic row, `position` places down the Inbox: ids start at 1.
pub fn row(position: u32) -> Row {
    row_from(
        i64::from(position) + 1,
        "Ada",
        &format!("Message {position}"),
        "",
        Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
    )
}

/// A row from `from`, with an address made of their first name.
pub fn row_from(id: i64, from: &str, subject: &str, preview: &str, when: DateTime<Utc>) -> Row {
    Row {
        id: MessageId::new(id),
        thread: None,
        is_thread: false,
        from: SafeText::new(from),
        address: Some(format!(
            "{}@example.com",
            from.split(' ').next().unwrap_or(from).to_lowercase()
        )),
        subject: SafeText::new(subject),
        preview: SafeText::new(preview),
        when,
        unread: false,
        attachment: false,
        count: 1,
        kind: crate::row::Kind::Message,
        marker: None,
        labels: Vec::new(),
        send_state: None,
    }
}

/// A moment on `day` of September 2026 at `hour:minute` by the clock of the
/// machine the test runs on, so the day a row sits under does not depend on
/// where that is.
pub fn local(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Local
        .with_ymd_and_hms(2026, 9, day, hour, minute, 0)
        .unwrap()
        .with_timezone(&Utc)
}

/// A conversation of the Inbox: message and thread both numbered `id`.
pub fn conversation(
    id: i64,
    from: &str,
    subject: &str,
    preview: &str,
    when: DateTime<Utc>,
) -> postio_model::listing::ThreadSummary {
    use postio_model::ThreadId;
    use postio_model::listing::{MessageSummary, ThreadSummary};
    let sender = EmailAddress::new(
        Some(from.to_owned()),
        format!(
            "{}@example.com",
            from.split(' ').next().unwrap_or(from).to_lowercase()
        ),
    );
    ThreadSummary {
        id: Some(ThreadId::new(id)),
        representative: MessageSummary {
            id: MessageId::new(id),
            thread: Some(ThreadId::new(id)),
            from: Some(sender.clone()),
            subject: Some(subject.to_owned()),
            preview: Some(preview.to_owned()),
            received_at: when,
            seen: true,
            flagged: false,
            answered: false,
            send_state: None,
            send_at: None,
            has_attachments: false,
            thread_count: 1,
        },
        subject: Some(subject.to_owned()),
        participants: vec![sender],
        message_count: 1,
        unread_count: 0,
        flagged: false,
        has_attachments: false,
        last_at: when,
        marker: None,
        copies: Vec::new(),
    }
}

/// `conversation`, unread.
pub fn unread(
    mut summary: postio_model::listing::ThreadSummary,
) -> postio_model::listing::ThreadSummary {
    summary.unread_count = 1;
    summary.representative.seen = false;
    summary
}

/// `conversation`, with `marker`.
pub fn marked(
    mut summary: postio_model::listing::ThreadSummary,
    marker: postio_model::listing::MarkerSummary,
) -> postio_model::listing::ThreadSummary {
    summary.marker = Some(marker);
    summary
}

/// A label of the account, with no colour of its own.
pub fn label(id: i64, name: &str) -> postio_model::Label {
    let mut label = postio_model::Label::new(AccountId::new(1), name);
    label.id = postio_model::ids::LabelId::new(id);
    label
}

/// The Focus inbox opened over exactly `rows`, and its pages served from
/// them: `Row::from` each, as a page arriving does.
pub fn show_focus(app: &mut App, rows: Vec<postio_ui::focus_list::FocusRow>) {
    show_scope(app, postio_model::FocusScope::Inbox, rows);
}

/// Focus's `scope` opened over exactly `rows`, and its pages served.
pub fn show_scope(
    app: &mut App,
    scope: postio_model::FocusScope,
    rows: Vec<postio_ui::focus_list::FocusRow>,
) {
    let rows: Vec<Row> = rows.into_iter().map(Row::from).collect();
    let effects = update(
        app,
        Input::Opened {
            scope: ListScope::Focus(scope),
            total: rows.len() as u32,
        },
    );
    serve_with(app, effects, |position| rows[position as usize].clone());
}

/// What the strip counts in the inbox of the drawing: 312 conversations, 41
/// of them unread, 7 with a marker, 186 filtered today.
pub fn drawing_counts() -> postio_client::protocol::FocusCounts {
    postio_client::protocol::FocusCounts {
        conversations: 312,
        unread: 41,
        has_action: 7,
        filtered_today: 186,
    }
}

/// Places for an account with filtering on and four digest rules.
pub fn places_with_features() -> Places {
    Places {
        features: crate::places::Features {
            filtering: true,
            digest_rules: 4,
        },
        ..places()
    }
}

/// The host says the Inbox (id 1) has `total` rows.
pub fn open_list(app: &mut App, total: u32) -> Vec<Effect> {
    update(
        app,
        Input::Opened {
            scope: ListScope::Mailbox(MailboxId::new(1)),
            total,
        },
    )
}

/// Answer every fetch among `effects` with [`row`]s for its range; return how
/// many there were.
pub fn serve(app: &mut App, effects: Vec<Effect>) -> usize {
    serve_with(app, effects, row)
}

/// Answer every fetch among `effects` with `row_at(position)` for each
/// position in its range that the list has; return how many there were.
pub fn serve_with(app: &mut App, effects: Vec<Effect>, row_at: impl Fn(u32) -> Row) -> usize {
    let mut fetched = 0;
    let mut pending = effects;
    while let Some(effect) = pending.pop() {
        if let Effect::Fetch {
            generation,
            page,
            fetch: Fetch::Scope(request),
            ..
        } = effect
        {
            fetched += 1;
            let total = app.total();
            let rows = (request.offset..request.offset + request.limit)
                .filter(|position| *position < total)
                .map(&row_at)
                .collect();
            pending.extend(update(
                app,
                Input::Page {
                    generation,
                    page,
                    rows: Ok(Page { total, rows }),
                },
            ));
        }
    }
    fetched
}

/// Open the Inbox with exactly `rows`, and serve its pages from them.
pub fn show_rows(app: &mut App, rows: &[Row]) {
    let effects = open_list(app, rows.len() as u32);
    serve_with(app, effects, |position| rows[position as usize].clone());
}

// -- Reading ---------------------------------------------------------------

/// The message `message` open and read: the cursor rests on it, its
/// recipients are known and its body has arrived.
pub fn open_message(app: &mut App, message: MessageId, to: Vec<EmailAddress>, body: MessageBody) {
    app.open_reading(message);
    update(app, Input::Addressed { message, to });
    update(
        app,
        Input::Body {
            message,
            answer: Ok(postio_client::protocol::Body::Ready {
                body,
                encoding_problems: false,
            }),
        },
    );
}

/// What the reader shows, as plain text.
pub fn reader_text(app: &App) -> String {
    app.reading()
        .expect("a message is open")
        .layout(Local::now())
        .0
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// -- Keys and the pointer --------------------------------------------------

/// `code` with `modifiers`, pressed.
pub fn key(code: KeyCode, modifiers: KeyModifiers) -> Input {
    Input::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

/// The character `c`, pressed.
pub fn press(c: char) -> Input {
    key(KeyCode::Char(c), KeyModifiers::NONE)
}

/// `ctrl+c`.
pub fn ctrl(c: char) -> Input {
    key(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// `alt+c`.
pub fn alt(c: char) -> Input {
    key(KeyCode::Char(c), KeyModifiers::ALT)
}

/// Type `text` a character at a time; everything the app asked for on the way.
pub fn type_text(app: &mut App, text: &str) -> Vec<Effect> {
    text.chars().flat_map(|c| update(app, press(c))).collect()
}

/// A click on `target`.
pub fn click(target: hit::Target, ctrl: bool, shift: bool) -> Input {
    Input::Pointer(Pointer::Click {
        hit: hit::Hit {
            target,
            column: 0,
            row: 0,
        },
        ctrl,
        shift,
    })
}

/// The wheel turned over `target`.
pub fn wheel(target: hit::Target, down: bool) -> Input {
    Input::Pointer(Pointer::Wheel {
        hit: hit::Hit {
            target,
            column: 0,
            row: 0,
        },
        down,
    })
}

// -- The screen ------------------------------------------------------------

/// The plain theme: no colour, so a screen compares as text.
pub fn plain_theme() -> Theme {
    Theme::new(Colour::None, Background::Unknown, &Default::default()).0
}

/// `app` drawn into a `width` x `height` buffer under the plain theme, and the
/// click targets the drawing left.
pub fn draw_into(width: u16, height: u16, app: &App) -> (ratatui::buffer::Buffer, hit::Hits) {
    let theme = plain_theme();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut hits = hit::Hits::default();
    terminal
        .draw(|frame| hits = draw(frame, app, &theme, now()))
        .unwrap();
    (terminal.backend().buffer().clone(), hits)
}

/// The cells `app` draws at `width` x `height`.
pub fn buffer(width: u16, height: u16, app: &App) -> ratatui::buffer::Buffer {
    draw_into(width, height, app).0
}

/// The click targets `app` draws at `width` x `height`.
pub fn hits_of(width: u16, height: u16, app: &App) -> hit::Hits {
    draw_into(width, height, app).1
}

/// The screen `app` draws at `width` x `height`, a line to a row.
pub fn screen(width: u16, height: u16, app: &App) -> String {
    let buffer = buffer(width, height, app);
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_opened_and_served_is_what_the_screen_shows() {
        let mut app = app((120, 30));
        let effects = open_list(&mut app, 3);
        assert_eq!(serve(&mut app, effects), 1, "one page covers three rows");
        let drawn = screen(120, 30, &app);
        for position in 0..3 {
            assert!(drawn.contains(&format!("Message {position}")), "{drawn}");
        }
    }

    #[test]
    fn a_message_opened_here_is_read_by_the_reader() {
        let mut app = app((120, 30));
        show_rows(&mut app, &[row(0)]);
        open_message(
            &mut app,
            MessageId::new(1),
            vec![EmailAddress::new(None::<String>, "grace@example.net")],
            MessageBody {
                text: Some("Looking now.".into()),
                html: None,
            },
        );
        assert!(reader_text(&app).contains("Looking now."));
    }
}
