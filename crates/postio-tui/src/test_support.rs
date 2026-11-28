//! One harness for driving the terminal app without a terminal, shared by the app's tests, the
//! view's and the `shot` example (through the `test-support` feature).

pub mod sample;

use crate::focus::Held;
use chrono::{DateTime, Local, TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use postio_model::mailbox::{Mailbox, MailboxRole};
use postio_model::{
    Account, AccountId, EmailAddress, ListScope, MailboxId, MessageBody, MessageId,
};
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

pub fn now() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap()
}

pub fn app(size: (u16, u16)) -> App {
    app_with_keys(size, &Default::default())
}

pub fn app_with_keys(size: (u16, u16), bindings: &postio_config::KeyBindings) -> App {
    App::new(
        size,
        Keys::new(&postio_core::Keymap::for_terminal(bindings)).0,
    )
}

pub fn account() -> Account {
    let mut account = Account::new("ada", EmailAddress::new(None::<String>, "ada@example.com"));
    account.id = AccountId::new(1);
    account.enabled = true;
    account.incoming.host = "imap.example.com".into();
    account
}

pub fn folder(id: i64, name: &str, role: MailboxRole, unread: u32) -> Mailbox {
    let mut folder = Mailbox::new(AccountId::new(1), name, None);
    folder.id = MailboxId::new(id);
    folder.role = role;
    folder.selectable = true;
    folder.counts.unread = unread;
    folder
}

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

pub fn saved_search(key: &str, name: &str, query: &str) -> Saved {
    Saved {
        key: key.into(),
        name: name.into(),
        query: query.into(),
    }
}

pub fn seed_places(app: &mut App, contents: Places) {
    update(app, Input::Places(contents));
}

pub fn row(position: u32) -> Row {
    row_from(
        i64::from(position) + 1,
        "Ada",
        &format!("Message {position}"),
        "",
        Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
    )
}

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

/// Local time, so the day a row sits under does not depend on the machine's zone.
pub fn local(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Local
        .with_ymd_and_hms(2026, 9, day, hour, minute, 0)
        .unwrap()
        .with_timezone(&Utc)
}

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
            to: Vec::new(),
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
        muted: None,
    }
}

pub fn unread(
    mut summary: postio_model::listing::ThreadSummary,
) -> postio_model::listing::ThreadSummary {
    summary.unread_count = 1;
    summary.representative.seen = false;
    summary
}

pub fn marked(
    mut summary: postio_model::listing::ThreadSummary,
    marker: postio_model::listing::MarkerSummary,
) -> postio_model::listing::ThreadSummary {
    summary.marker = Some(marker);
    summary
}

pub fn label(id: i64, name: &str) -> postio_model::Label {
    let mut label = postio_model::Label::new(AccountId::new(1), name);
    label.id = postio_model::ids::LabelId::new(id);
    label
}

pub fn show_focus(app: &mut App, rows: Vec<postio_ui::focus_list::FocusRow>) {
    show_scope(app, postio_model::FocusScope::Inbox, rows);
}

pub fn show_scope(
    app: &mut App,
    scope: postio_model::FocusScope,
    rows: Vec<postio_ui::focus_list::FocusRow>,
) {
    let rows: Vec<Held> = rows.into_iter().map(Held::of).collect();
    app.open_for_tests(ListScope::Focus(scope), rows.len() as u32);
    app.serve_for_tests(|position| rows[position as usize].clone());
}

pub fn drawing_counts() -> postio_client::protocol::FocusCounts {
    postio_client::protocol::FocusCounts {
        conversations: 312,
        unread: 41,
        has_action: 7,
        filtered_today: 186,
    }
}

pub fn places_with_features() -> Places {
    Places {
        features: crate::places::Features {
            filtering: true,
            digest_rules: 4,
            ..Default::default()
        },
        ..places()
    }
}

pub fn opens(effects: &[Effect], scope: ListScope) -> bool {
    effects.iter().any(|effect| {
        matches!(effect, Effect::Focus { request: postio_focus::Request::OpenScope { scope: asked, .. }, .. } if *asked == scope)
    })
}

pub fn opens_any(effects: &[Effect]) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Focus {
                request: postio_focus::Request::OpenScope { .. },
                ..
            }
        )
    })
}

pub fn asked(effects: &[Effect]) -> Vec<(postio_focus::Ticket, postio_focus::Request)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Focus { ticket, request } => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect()
}

pub fn sent(effects: &[Effect]) -> Vec<postio_core::Command> {
    asked(effects)
        .into_iter()
        .filter_map(|(_, request)| match request {
            postio_focus::Request::Send { command, .. } => Some(command),
            _ => None,
        })
        .collect()
}

pub fn labels_arrive(
    app: &mut App,
    opened: &[Effect],
    labels: Vec<postio_model::Label>,
    counts: Vec<(postio_model::LabelId, u32)>,
    applied: &[postio_model::LabelId],
) {
    for (ticket, request) in asked(opened) {
        let postio_focus::Request::Labels { threads, stamp, .. } = request else {
            continue;
        };
        let carried = labels
            .iter()
            .filter(|label| applied.contains(&label.id))
            .flat_map(|label| threads.iter().map(|thread| (*thread, label.clone())))
            .collect();
        let read = postio_focus::LabelsRead {
            account: AccountId::new(1),
            labels: labels.clone(),
            counts: counts.clone(),
            carried,
        };
        let answer = postio_focus::Reply::Labels {
            stamp,
            answer: Ok(read),
        };
        update(app, Input::FocusReply(ticket, answer));
    }
}

pub fn folders_arrive(
    app: &mut App,
    opened: &[Effect],
    folders: Vec<Mailbox>,
    recent: Vec<MailboxId>,
) {
    for (ticket, request) in asked(opened) {
        let postio_focus::Request::Folders { stamp } = request else {
            continue;
        };
        let read = postio_focus::FoldersRead {
            folders: folders.clone(),
            recent: recent.clone(),
        };
        let answer = postio_focus::Reply::Folders {
            stamp,
            answer: Ok(read),
        };
        update(app, Input::FocusReply(ticket, answer));
    }
}

pub fn places_arrive(
    app: &mut App,
    opened: &[Effect],
    folders: &[Mailbox],
    labels: &[(postio_model::Label, u32)],
    outbox: &[(AccountId, u32)],
) {
    use postio_ui::places as rules;
    for (ticket, request) in asked(opened) {
        if request != postio_focus::Request::Places {
            continue;
        }
        let mut read = postio_focus::PlacesRead::default();
        for folder in folders {
            read.folders.push((folder.id, rules::place_name(folder)));
            read.places.push(rules::mailbox_place(folder));
            read.entries.push(rules::mailbox_entry(folder));
        }
        for (account, waiting) in outbox.iter().filter(|(_, waiting)| *waiting > 0) {
            read.entries.push(rules::outbox_entry(*account, *waiting));
        }
        for (label, held) in labels {
            read.entries.push(rules::label_entry(label, Some(*held)));
            read.places.push(rules::label_place(label));
        }
        for role in [MailboxRole::Snoozed, MailboxRole::Flagged] {
            read.entries.push(rules::view_entry(role, None));
        }
        update(
            app,
            Input::FocusReply(ticket, postio_focus::Reply::Places(Ok(read))),
        );
    }
}

pub fn results_arrive(app: &mut App, opened: &[Effect], subjects: &[&str]) {
    use postio_focus::{Reply, Request};
    use postio_search::results::{
        ConversationHit, ConversationKey, ConversationResults, Match, Source,
    };
    let found = ConversationResults {
        hits: subjects
            .iter()
            .zip(1_i64..)
            .map(|(subject, n)| ConversationHit {
                key: ConversationKey::Thread(postio_model::ThreadId::new(2000 + n)),
                best: MessageId::new(1000 + n),
                best_number: 1,
                mailbox_id: MailboxId::new(1),
                subject: Some((*subject).to_owned()),
                from: Some(EmailAddress::new(Some("Ada Moreno"), "ada@example.com")),
                newest_match: Utc.with_ymd_and_hms(2026, 9, 20, 9, 0, 0).unwrap(),
                messages: 1,
                unread: false,
                has_attachments: false,
                labels: Vec::new(),
                score: 1.0,
                reasons: Vec::new(),
                matched_messages: 1,
                matches: vec![Match {
                    source: Source::Body,
                    passage: None,
                    when: None,
                }],
            })
            .collect(),
        total: subjects.len() as u64,
        capped: false,
        messages_searched: 100,
        corpus_complete: true,
        contents_complete: true,
        facets: Default::default(),
        files: 0,
        people: 0,
        names: Default::default(),
        elapsed: std::time::Duration::from_millis(3),
    };
    for (ticket, request) in asked(opened) {
        let reply = match request {
            Request::ResultsPage {
                order,
                offset,
                stamp,
                ..
            } => Reply::ResultsPage {
                stamp,
                order,
                offset,
                answer: Ok(Box::new(found.clone())),
            },
            Request::Facets { stamp, .. } => Reply::Facets {
                stamp,
                answer: Ok(Box::new(found.clone())),
            },
            Request::ResultsPassages { stamp, .. } => Reply::ResultsPassages {
                stamp,
                answer: Ok(Vec::new()),
            },
            _ => continue,
        };
        update(app, Input::FocusReply(ticket, reply));
    }
}

pub fn open_scope(app: &mut App, scope: ListScope, total: u32) {
    app.open_for_tests(scope, total);
}

pub fn open_list(app: &mut App, total: u32) -> Vec<Effect> {
    app.open_for_tests(ListScope::Mailbox(MailboxId::new(1)), total);
    Vec::new()
}

pub fn serve(app: &mut App, effects: Vec<Effect>) -> usize {
    serve_with(app, effects, row)
}

pub fn serve_with(app: &mut App, _effects: Vec<Effect>, row_at: impl Fn(u32) -> Row) -> usize {
    app.serve_for_tests(|position| held_of(row_at(position)))
}

pub fn held_of(row: Row) -> Held {
    let source = postio_ui::focus_list::FocusRow::conversation(conversation(
        row.id.get(),
        row.from.as_str(),
        row.subject.as_str(),
        row.preview.as_str(),
        row.when,
    ));
    Held { row, source }
}

pub fn show_rows(app: &mut App, rows: &[Row]) {
    let effects = open_list(app, rows.len() as u32);
    serve_with(app, effects, |position| rows[position as usize].clone());
}

pub fn filtered_row(
    id: i64,
    source: &str,
    reason: &str,
    day: u32,
    hour: u32,
    minute: u32,
) -> postio_client::protocol::FilteredRow {
    let at = local(day, hour, minute);
    let mut message = conversation(id, source, &format!("Filtered {id}"), "", at).representative;
    message.thread = None;
    postio_client::protocol::FilteredRow {
        message,
        reason: reason.to_owned(),
        source: Some(source.to_owned()),
        at,
    }
}

/// Returns every effect the app returned on the way, the first `effects` included.
pub fn serve_focus(
    app: &mut App,
    effects: Vec<Effect>,
    mut reply: impl FnMut(&postio_focus::Request) -> Option<postio_focus::Reply>,
) -> Vec<Effect> {
    let mut seen = Vec::new();
    let mut pending: Vec<Effect> = effects.into_iter().rev().collect();
    while let Some(effect) = pending.pop() {
        if let Effect::Focus { ticket, request } = &effect
            && let Some(answer) = reply(request)
        {
            pending.extend(
                update(app, Input::FocusReply(*ticket, answer))
                    .into_iter()
                    .rev(),
            );
        }
        seen.push(effect);
    }
    seen
}

pub fn serve_filtered(
    app: &mut App,
    effects: Vec<Effect>,
    tabs: &[(&str, u32)],
    rows: &[postio_client::protocol::FilteredRow],
) -> Vec<Effect> {
    use postio_focus::{Reply, Request};
    serve_focus(app, effects, |request| match request {
        Request::FilteredTabs => Some(Reply::FilteredTabs(Ok(tabs
            .iter()
            .map(|(reason, count)| ((*reason).to_owned(), *count))
            .collect()))),
        Request::Filtered {
            reason,
            offset,
            stamp,
        } => Some(Reply::Filtered {
            stamp: *stamp,
            offset: *offset,
            answer: Ok(rows
                .iter()
                .filter(|row| reason.as_ref().is_none_or(|reason| *reason == row.reason))
                .skip(*offset as usize)
                .take(postio_ui::filtered::PAGE as usize)
                .cloned()
                .collect()),
        }),
        _ => None,
    })
}

pub fn serve_digest(
    app: &mut App,
    effects: Vec<Effect>,
    messages: Vec<postio_model::listing::MessageSummary>,
    summary: Option<postio_model::summary::DigestSummary>,
) -> Vec<Effect> {
    use postio_focus::{Reply, Request};
    serve_focus(app, effects, |request| match request {
        Request::DigestRead { stamp, .. } => Some(Reply::DigestRead {
            stamp: *stamp,
            rows: Ok(messages.clone()),
            summary: summary.clone(),
        }),
        _ => None,
    })
}

pub fn serve_preview(
    app: &mut App,
    effects: Vec<Effect>,
    preview: postio_client::protocol::DigestPreview,
) -> Vec<Effect> {
    use postio_focus::{Reply, Request};
    serve_focus(app, effects, |request| match request {
        Request::DigestPreview { stamp, .. } => Some(Reply::DigestPreview {
            stamp: *stamp,
            answer: Ok(preview.clone()),
        }),
        _ => None,
    })
}

pub fn serve_vault(
    app: &mut App,
    effects: Vec<Effect>,
    picture: postio_client::protocol::VaultPicture,
) -> Vec<Effect> {
    use postio_focus::{Reply, Request};
    serve_focus(app, effects, |request| match request {
        Request::Vault { stamp, .. } => Some(Reply::Vault {
            stamp: *stamp,
            answer: Ok(picture.clone()),
        }),
        _ => None,
    })
}

pub fn open_message(app: &mut App, message: MessageId, to: Vec<EmailAddress>, body: MessageBody) {
    enter_thread(app, &[message.get()]);
    let mut row = header(app, message);
    row.to = to;
    update(
        app,
        Input::Addressed {
            message,
            row,
            warning: None,
        },
    );
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

pub fn header(app: &App, message: MessageId) -> Box<postio_model::Message> {
    let row = app.row(message);
    let at = row.map_or_else(|| local(23, 9, 0), |row| row.when);
    let mut header = postio_model::Message::new(AccountId::new(1), MailboxId::new(1), at);
    header.id = message;
    if let Some(row) = row {
        let address = row.address.clone().unwrap_or_default();
        header.from = vec![EmailAddress::new(Some(row.from.as_str()), address)];
    }
    Box::new(header)
}

pub fn enter_thread(app: &mut App, ids: &[i64]) -> Vec<Effect> {
    use postio_model::thread_view::{Line, ThreadMember, ThreadView};
    let asked = update(app, key(KeyCode::Enter, KeyModifiers::NONE));
    let mut effects = Vec::new();
    for effect in asked {
        let Effect::Focus {
            ticket,
            request: postio_focus::Request::ReadThread(thread),
        } = effect
        else {
            effects.push(effect);
            continue;
        };
        let members = ids
            .iter()
            .map(|id| ThreadMember {
                id: MessageId::new(*id),
                from: Some(EmailAddress::new(Some("Lena Park"), "lena@example.com")),
                own: false,
                at: local(23, 11, 22),
                seen: true,
                has_attachments: false,
                line: Line::Preview(String::new()),
                answers: None,
                marker: false,
            })
            .collect();
        let view = ThreadView {
            thread,
            subject: None,
            members,
            draft: None,
            muted: None,
            labels: Vec::new(),
        };
        let reply = postio_focus::Reply::ThreadRead {
            thread,
            view: Ok(Some(view)),
        };
        effects.extend(update(app, Input::FocusReply(ticket, reply)));
    }
    effects
}

pub fn reader_text(app: &App) -> String {
    assert!(app.reading().is_some(), "a message is open");
    crate::view::open::document(app, &plain_theme(), 72, now())
        .lines
        .iter()
        .map(|line| line.line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn key(code: KeyCode, modifiers: KeyModifiers) -> Input {
    Input::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

pub fn press(c: char) -> Input {
    key(KeyCode::Char(c), KeyModifiers::NONE)
}

pub fn ctrl(c: char) -> Input {
    key(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub fn alt(c: char) -> Input {
    key(KeyCode::Char(c), KeyModifiers::ALT)
}

pub fn type_text(app: &mut App, text: &str) -> Vec<Effect> {
    text.chars().flat_map(|c| update(app, press(c))).collect()
}

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

/// A character is that key; a name in angle brackets is a named key (`<Enter>`, `<F3>`), with
/// `C-`, `A-` or `S-` for ctrl, alt, shift (`<C-z>`).
pub fn keys_of(spec: &str) -> Vec<Input> {
    let mut out = Vec::new();
    let mut rest = spec;
    while let Some(first) = rest.chars().next() {
        if first == '<'
            && let Some(end) = rest.find('>')
        {
            let mut name = &rest[1..end];
            let mut modifiers = KeyModifiers::NONE;
            while let Some((prefix, after)) = name
                .split_once('-')
                .filter(|(prefix, _)| matches!(*prefix, "C" | "A" | "S") && !name.is_empty())
            {
                modifiers |= match prefix {
                    "C" => KeyModifiers::CONTROL,
                    "A" => KeyModifiers::ALT,
                    _ => KeyModifiers::SHIFT,
                };
                name = after;
            }
            let code = match name {
                "Enter" => KeyCode::Enter,
                "Esc" => KeyCode::Esc,
                "Tab" => KeyCode::Tab,
                "Del" => KeyCode::Delete,
                "Up" => KeyCode::Up,
                "Down" => KeyCode::Down,
                "Left" => KeyCode::Left,
                "Right" => KeyCode::Right,
                "F3" => KeyCode::F(3),
                "F5" => KeyCode::F(5),
                "F8" => KeyCode::F(8),
                _ if name.chars().count() == 1 => {
                    KeyCode::Char(name.chars().next().expect("a character"))
                }
                _ => panic!("no key called <{name}>"),
            };
            out.push(key(code, modifiers));
            rest = &rest[end + 1..];
        } else {
            out.push(press(first));
            rest = &rest[first.len_utf8()..];
        }
    }
    out
}

pub fn press_keys(app: &mut App, spec: &str) -> Vec<Effect> {
    keys_of(spec)
        .into_iter()
        .flat_map(|input| update(app, input))
        .collect()
}

pub fn mouse_at(
    app: &mut App,
    (width, height): (u16, u16),
    (x, y): (u16, u16),
    kind: crossterm::event::MouseEventKind,
    modifiers: KeyModifiers,
) -> Vec<Effect> {
    let hits = hits_of(width, height, app);
    mouse_on(app, &hits, (x, y), kind, modifiers)
}

pub fn mouse_on(
    app: &mut App,
    hits: &hit::Hits,
    (x, y): (u16, u16),
    kind: crossterm::event::MouseEventKind,
    modifiers: KeyModifiers,
) -> Vec<Effect> {
    let event = crossterm::event::MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers,
    };
    match crate::run::pointer(&event, hits) {
        Some(pointer) => update(app, Input::Pointer(pointer)),
        None => Vec::new(),
    }
}

pub fn click_at(app: &mut App, size: (u16, u16), at: (u16, u16)) -> Vec<Effect> {
    use crossterm::event::{MouseButton, MouseEventKind};
    mouse_at(
        app,
        size,
        at,
        MouseEventKind::Down(MouseButton::Left),
        KeyModifiers::NONE,
    )
}

pub fn wheel_at(app: &mut App, size: (u16, u16), at: (u16, u16), down: bool) -> Vec<Effect> {
    use crossterm::event::MouseEventKind;
    let kind = if down {
        MouseEventKind::ScrollDown
    } else {
        MouseEventKind::ScrollUp
    };
    mouse_at(app, size, at, kind, KeyModifiers::NONE)
}

pub fn plain_theme() -> Theme {
    Theme::new(Colour::None, Background::Unknown, &Default::default()).0
}

pub fn draw_into(width: u16, height: u16, app: &App) -> (ratatui::buffer::Buffer, hit::Hits) {
    let theme = plain_theme();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut hits = hit::Hits::default();
    terminal
        .draw(|frame| hits = draw(frame, app, &theme, now()))
        .unwrap();
    (terminal.backend().buffer().clone(), hits)
}

pub fn buffer(width: u16, height: u16, app: &App) -> ratatui::buffer::Buffer {
    draw_into(width, height, app).0
}

pub fn hits_of(width: u16, height: u16, app: &App) -> hit::Hits {
    draw_into(width, height, app).1
}

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

pub fn digest_row(
    delivery: i64,
    rule: &str,
    count: u32,
    senders: usize,
) -> postio_ui::focus_list::FocusRow {
    postio_ui::focus_list::FocusRow::Digest(postio_ui::focus_list::Digest {
        delivery: postio_model::ids::DeliveryId::new(delivery),
        rule: rule.to_owned(),
        cadence: Some(postio_model::listing::Cadence::Weekly),
        count,
        senders: (0..senders)
            .map(|at| {
                EmailAddress::new(
                    Some(format!("Sender {at}")),
                    format!("sender{at}@example.com"),
                )
            })
            .collect(),
        summary_line: None,
        at: local(23, 11, 0),
    })
}

pub fn with_model() -> postio_config::FocusConfig {
    postio_config::Config::from_toml_str(
        "[focus.model]\nendpoint = \"http://127.0.0.1:11434/v1\"\nmodel = \"a-small-model\"\n",
    )
    .expect("a config")
    .focus
}

pub fn newsletters_rule() -> postio_config::DigestRule {
    postio_config::Config::from_toml_str(
        "[[focus.digests]]\nname = \"Newsletters\"\nmatch = [\"from:sender0@example.com\"]\n\
         cadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"\n",
    )
    .expect("a config")
    .focus
    .digests
    .remove(0)
}

pub fn held(
    id: i64,
    from: &str,
    subject: &str,
    preview: &str,
) -> postio_model::listing::MessageSummary {
    conversation(id, from, subject, preview, local(22, 9, id as u32 % 50)).representative
}

pub fn summary_of(cites: &[(&str, &str, i64)]) -> postio_model::summary::DigestSummary {
    use postio_model::summary::{DigestSummary, SummaryReference, SummaryStatement};
    DigestSummary {
        statements: cites
            .iter()
            .enumerate()
            .map(|(at, (topic, text, message))| SummaryStatement {
                topic: (*topic).to_owned(),
                text: (*text).to_owned(),
                reference: SummaryReference {
                    number: at as u32 + 1,
                    message: MessageId::new(*message),
                    excerpt: format!("passage {}", at + 1),
                },
            })
            .collect(),
        messages: 14,
        senders: 6,
    }
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
