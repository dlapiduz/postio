use std::collections::BTreeMap;

use postio_config::KeyBindings;
use postio_config::paths::Platform;
use postio_core::config::Keymap;
use postio_core::{Availability, CommandId, Context, Frontend, Scope, registry};
use postio_model::AccountId;

const CALENDAR: &[Context] = &[Context::Calendar, Context::CalendarWindow];

struct Row {
    id: &'static str,
    key: &'static str,
    contexts: &'static [Context],
    terminal: bool,
}

const fn row(id: &'static str, key: &'static str, contexts: &'static [Context]) -> Row {
    Row {
        id,
        key,
        contexts,
        terminal: true,
    }
}

const fn windowed(id: &'static str, key: &'static str, contexts: &'static [Context]) -> Row {
    Row {
        id,
        key,
        contexts,
        terminal: false,
    }
}

const CONTRACT: &[Row] = &[
    row(
        "go_to_calendar",
        "g e",
        &[Context::List, Context::Reader, Context::Sidebar],
    ),
    row("calendar_agenda", "1", CALENDAR),
    row("calendar_previous", "[", CALENDAR),
    row("calendar_next", "]", CALENDAR),
    row("calendar_today", "t", CALENDAR),
    row("next_message", "j", CALENDAR),
    row("prev_message", "k", CALENDAR),
    row("first_message", "g g", CALENDAR),
    row("last_message", "G", CALENDAR),
    row("open_message", "Return", CALENDAR),
    row("search", "/", CALENDAR),
    windowed("pop_out", "mod+shift+o", &[Context::Calendar]),
    windowed("pop_in", "mod+shift+o", &[Context::CalendarWindow]),
    row("new_event", "c", CALENDAR),
    row("edit_event", "mod+e", CALENDAR),
    row("delete", "#", CALENDAR),
    row("move_event", "v", CALENDAR),
    row("duplicate_event", "mod+shift+d", CALENDAR),
    row("accept_invite", "y", CALENDAR),
    row("decline_invite", "Y", CALENDAR),
    row(
        "tentative_invite",
        "alt+y",
        &[
            Context::List,
            Context::Conversation,
            Context::Reader,
            Context::Calendar,
            Context::CalendarWindow,
        ],
    ),
    row("save_event", "mod+s", CALENDAR),
    row("save_and_send_event", "mod+Return", CALENDAR),
    windowed("email_attendees", "alt+e", CALENDAR),
    windowed(
        "close_window",
        "mod+w",
        &[Context::List, Context::Composer, Context::CalendarWindow],
    ),
    row("quit", "mod+q", &[Context::List, Context::CalendarWindow]),
];

fn open_in(app: Frontend) -> Availability {
    Availability {
        frontend: app,
        ..Availability::open(Scope::Account(AccountId::new(1)))
    }
}

#[test]
fn the_registry_holds_the_calendar_contracts_keys() {
    let mut problems = Vec::new();
    for row in CONTRACT {
        let Ok(id) = row.id.parse::<CommandId>() else {
            problems.push(format!("`{}` is not a command", row.id));
            continue;
        };
        let spec = registry::get(id);
        if spec.default_binding != row.key {
            problems.push(format!(
                "`{}` is on `{}`, and the contract puts it on `{}`",
                row.id, spec.default_binding, row.key
            ));
        }
        for context in row.contexts {
            if !spec.available_in(*context) {
                problems.push(format!("`{}` is not reachable in {context}", row.id));
            }
        }
        if !spec.requires.met_by(open_in(Frontend::Focus)) {
            problems.push(format!("`{}` is not offered by the desktop app", row.id));
        }
        if spec.requires.met_by(open_in(Frontend::Terminal)) != row.terminal {
            problems.push(format!(
                "`{}` is {} in the terminal, and the contract says otherwise",
                row.id,
                if row.terminal {
                    "not offered"
                } else {
                    "offered"
                }
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_reused_verb_is_titled_for_the_calendar_there() {
    let open = "open_message".parse::<CommandId>().expect("open_message");
    assert_eq!(registry::title_in(open, Context::Calendar), "Open event");
    assert_eq!(
        registry::title_in(open, Context::List),
        registry::get(open).title
    );
    let delete = "delete".parse::<CommandId>().expect("delete");
    assert_eq!(
        registry::title_in(delete, Context::Calendar),
        "Delete event"
    );
    let new = "new_event".parse::<CommandId>().expect("new_event");
    assert_eq!(registry::title_in(new, Context::Calendar), "New event");
}

#[test]
fn no_key_means_two_commands_in_a_calendar_context() {
    for platform in [Platform::Freedesktop, Platform::Apple] {
        let keymap = Keymap::resolve_on(&KeyBindings::default(), platform);
        for context in CALENDAR {
            let mut seen: BTreeMap<String, CommandId> = BTreeMap::new();
            for action in registry::reachable_in(*context, open_in(Frontend::Focus)) {
                let Some(id) = action.id.builtin() else {
                    continue;
                };
                for binding in keymap.bindings(id) {
                    if let Some(other) = seen.insert(binding.clone(), id) {
                        assert_eq!(
                            other, id,
                            "{platform:?}, {context}: `{binding}` is both `{other}` and `{id}`"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn one_chord_pops_calendar_out_and_back_in() {
    let out = registry::lookup_binding_on(Context::Calendar, "ctrl+shift+o", Platform::Freedesktop);
    let back = registry::lookup_binding_on(
        Context::CalendarWindow,
        "ctrl+shift+o",
        Platform::Freedesktop,
    );
    assert_eq!(out.map(|spec| spec.id.as_str()), Some("pop_out"));
    assert_eq!(back.map(|spec| spec.id.as_str()), Some("pop_in"));
}

#[test]
fn mod_w_closes_the_window_and_mod_q_alone_quits_everywhere() {
    let quit = registry::get(CommandId::Quit);
    assert_eq!(quit.default_binding, "mod+q");
    assert!(
        quit.alternate_bindings.is_empty(),
        "quit keeps {:?}",
        quit.alternate_bindings
    );
    let close = "close_window".parse::<CommandId>().expect("close_window");
    for context in Context::ALL {
        assert!(
            registry::get(close).available_in(*context),
            "mod+w is not close_window in {context}"
        );
        for platform in [Platform::Freedesktop, Platform::Apple] {
            let key = postio_config::keys::expand_mod("mod+w", platform);
            assert_eq!(
                registry::lookup_binding_on(*context, &key, platform).map(|spec| spec.id),
                Some(close),
                "{platform:?}, {context}"
            );
            assert!(registry::binding_offered_on(
                close.into(),
                "mod+w",
                platform
            ));
        }
    }
}

#[test]
fn the_terminal_has_no_second_window() {
    for id in ["pop_out", "pop_in"] {
        let id = id.parse::<CommandId>().expect("a calendar command");
        assert!(
            !registry::get(id)
                .requires
                .met_by(open_in(Frontend::Terminal))
        );
    }
}
