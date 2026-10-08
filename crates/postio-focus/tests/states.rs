//! What Focus says about its own state, from the controller's side
//! (specs/009-focus-macos T095; screens 16 to 19).
//!
//! What GTK's window did in `hear_sync`, `note_account`, `show_state` and
//! `show_empty_or_list`, asserted here so both frontends get it: the first
//! word about an account is news, the banner and the sync label say what
//! `postio_ui::focus_state` says for where every account stands, a refused
//! password offers to update it, and an inbox with no rows says why, with
//! only the shortcuts that lead somewhere.

use chrono::{DateTime, Local, TimeZone};
use postio_client::protocol::FocusCounts;
use postio_config::FocusConfig;
use postio_config::paths::Platform;
use postio_core::{CommandId, ConnectionState, Event, FailureReason};
use postio_focus::{
    AccountsRead, BannerButton, BannerView, Effect, FocusController, Input, Intent, NoRows, Opened,
    PageAnswer, Policy, Reply, Request, Ticket,
};
use postio_model::{AccountId, FocusScope, ListScope, MailboxId};
use postio_ui::focus_state::{self, AccountFacts, Banner, EmptyInbox, EmptyPlace, InboxSaying};

const INBOX: ListScope = ListScope::Focus(FocusScope::Inbox);
const ADA: AccountId = AccountId::new(1);

/// Nine thirty on a Tuesday, where every clock in these tests is stopped.
fn at() -> DateTime<Local> {
    Local
        .with_ymd_and_hms(2026, 9, 29, 9, 30, 0)
        .single()
        .expect("a time")
}

fn focus() -> FocusController {
    let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
    focus.handle(Input::Clock(Some(at())));
    focus
}

fn ada() -> AccountFacts {
    AccountFacts {
        id: ADA,
        server: "imap.example.com".to_owned(),
        address: "ada@example.com".to_owned(),
        name: "Ada".to_owned(),
    }
}

fn connection(state: ConnectionState) -> Input {
    Input::Event(Event::ConnectionChanged {
        account: ADA,
        state,
    })
}

fn progress(done: u32, total: u32) -> Input {
    Input::Event(Event::SyncProgress {
        account: ADA,
        done,
        total,
    })
}

fn banners(effects: &[Effect]) -> Vec<Option<BannerView>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(Intent::Banner(banner)) => Some(banner.clone()),
            _ => None,
        })
        .collect()
}

fn labels(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(Intent::SyncLabel(label)) => Some(label.text.clone()),
            _ => None,
        })
        .collect()
}

fn empties(effects: &[Effect]) -> Vec<Option<EmptyInbox>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(Intent::Empty(page)) => Some(page.clone()),
            _ => None,
        })
        .collect()
}

fn asks(effects: &[Effect]) -> Vec<(Ticket, Request)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(ticket, request) => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect()
}

/// The key `command` has under the controller's keymap, as a banner's
/// button names it.
fn key(focus: &FocusController, command: CommandId) -> Option<String> {
    postio_ui::hints::key(focus.keymap(), command)
}

/// The strip's counts arrive, `conversations` in the inbox.
fn counted(focus: &mut FocusController, conversations: u32, filtered_today: u32) -> Vec<Effect> {
    let Effect::Ask(ticket, Request::FocusCounts) = focus.refresh_counts() else {
        panic!("refreshing asks for the counts");
    };
    focus.handle(Input::Reply(
        ticket,
        Reply::FocusCounts(Ok(FocusCounts {
            conversations,
            filtered_today,
            ..FocusCounts::default()
        })),
    ))
}

#[test]
fn offline_says_so_in_the_banner_and_the_label() {
    let mut focus = focus();
    // A tracker starts out Offline, so a first report of Offline changes
    // nothing in it; it is still news, the first word about the account.
    let effects = focus.handle(connection(ConnectionState::Offline));
    assert_eq!(
        banners(&effects),
        vec![Some(BannerView {
            heading: Banner::Offline.heading(),
            sentence: Banner::Offline.sentence(),
            button: Some(BannerButton {
                label: "Retry now".to_owned(),
                command: CommandId::Refresh,
                key: key(&focus, CommandId::Refresh),
            }),
            progress: None,
            error: false,
            account: None,
        })]
    );
    assert_eq!(labels(&effects), vec!["Offline".to_owned()]);
}

#[test]
fn a_first_sync_shows_how_far_it_has_come() {
    let mut focus = focus();
    let _ = focus.handle(connection(ConnectionState::Online));
    let effects = focus.handle(progress(1200, 8400));
    let first = Banner::FirstSync {
        done: 1200,
        total: 8400,
    };
    assert_eq!(
        banners(&effects),
        vec![Some(BannerView {
            heading: first.heading(),
            sentence: first.sentence(),
            button: None,
            progress: Some((1200, 8400)),
            error: false,
            account: None,
        })]
    );
    assert_eq!(labels(&effects), vec![focus_state::syncing(1200, 8400)]);
}

#[test]
fn a_finished_pass_says_when_mail_last_synced() {
    let mut focus = focus();
    let _ = focus.handle(connection(ConnectionState::Online));
    let _ = focus.handle(progress(10, 8400));
    let effects = focus.handle(progress(8400, 8400));
    assert_eq!(banners(&effects), vec![None], "the first sync is over");
    assert_eq!(
        labels(&effects),
        vec!["Synced 09:30".to_owned()],
        "the controller's clock is when it finished"
    );
}

#[test]
fn backfill_is_mail_still_arriving() {
    let mut focus = focus();
    let _ = focus.handle(connection(ConnectionState::Online));
    let _ = focus.handle(progress(10, 10));
    let effects = focus.handle(Input::Event(Event::BackfillProgress {
        account: ADA,
        done: 20,
        total: 50,
        footprint: None,
    }));
    assert_eq!(labels(&effects), vec![focus_state::syncing(20, 50)]);
    assert!(
        banners(&effects).is_empty(),
        "the banner did not change, so it is not drawn again"
    );
}

#[test]
fn the_same_news_twice_draws_nothing_again() {
    let mut focus = focus();
    let _ = focus.handle(connection(ConnectionState::Offline));
    let effects = focus.handle(connection(ConnectionState::Offline));
    assert!(banners(&effects).is_empty());
    assert!(labels(&effects).is_empty());
}

#[test]
fn the_first_word_about_an_account_asks_who_it_is() {
    let mut focus = focus();
    let effects = focus.handle(connection(ConnectionState::Connecting));
    assert_eq!(
        asks(&effects)
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>(),
        vec![Request::Accounts]
    );
    let effects = focus.handle(connection(ConnectionState::Online));
    assert!(
        asks(&effects).is_empty(),
        "an account asked about is not asked about again"
    );
}

#[test]
fn a_refused_password_offers_to_update_it() {
    let mut focus = focus();
    let effects = focus.handle(connection(ConnectionState::Failing {
        reason: FailureReason::Auth,
    }));
    assert_eq!(labels(&effects), vec!["Sync failed".to_owned()]);
    let (ticket, _) = asks(&effects).pop().expect("who the account is");
    // A reply asked before an invalidation is still who the account is.
    focus.invalidate();
    let effects = focus.handle(Input::Reply(
        ticket,
        Reply::Accounts(Ok(AccountsRead {
            facts: vec![ada()],
            last_synced: None,
        })),
    ));
    let refused = Banner::SignIn {
        server: "imap.example.com".to_owned(),
        address: "ada@example.com".to_owned(),
        missing: false,
    };
    assert_eq!(
        banners(&effects),
        vec![Some(BannerView {
            heading: "Can't sign in to imap.example.com".to_owned(),
            sentence: refused.sentence(),
            button: Some(BannerButton {
                label: "Update password\u{2026}".to_owned(),
                command: CommandId::UpdateCredential,
                key: key(&focus, CommandId::UpdateCredential),
            }),
            progress: None,
            error: true,
            account: Some(ADA),
        })]
    );
}

#[test]
fn an_empty_inbox_before_its_first_sync_offers_only_compose() {
    let mut focus = focus();
    let _ = focus.open(INBOX);
    let effects = counted(&mut focus, 0, 0);
    let keymap = focus.keymap().clone();
    let page = focus_state::empty_inbox(&FocusConfig::default(), 0, &keymap, &at()).saying(
        &InboxSaying::Syncing { progress: None },
        &keymap,
        &Local,
    );
    assert_eq!(empties(&effects), vec![Some(page.clone())]);
    assert_eq!(
        page.shortcuts
            .iter()
            .map(|(_, _, command)| *command)
            .collect::<Vec<_>>(),
        vec![CommandId::Compose],
        "nothing has been filtered or archived yet"
    );
}

#[test]
fn an_empty_inbox_after_a_pass_offers_only_what_exists() {
    let mut focus = focus();
    let _ = focus.handle(Input::Config(FocusConfig {
        filtering: false,
        ..FocusConfig::default()
    }));
    let _ = focus.open(INBOX);
    let _ = focus.handle(connection(ConnectionState::Online));
    let _ = focus.handle(progress(5, 5));
    let effects = counted(&mut focus, 0, 3);
    let page = empties(&effects)
        .pop()
        .flatten()
        .expect("the inbox says it is empty");
    assert_eq!(page.heading, "Inbox is empty");
    assert_eq!(
        page.shortcuts
            .iter()
            .map(|(_, _, command)| *command)
            .collect::<Vec<_>>(),
        vec![CommandId::GoToArchive, CommandId::Compose],
        "with filtering off, Filtered is not offered"
    );
    let effects = focus.handle(Input::Config(FocusConfig {
        filtering: true,
        ..FocusConfig::default()
    }));
    let page = empties(&effects)
        .pop()
        .flatten()
        .expect("the page again, with Filtered");
    assert_eq!(
        page.shortcuts
            .iter()
            .map(|(_, words, command)| (words.as_str(), *command))
            .collect::<Vec<_>>(),
        vec![
            ("3 filtered today", CommandId::GoToFiltered),
            ("archive", CommandId::GoToArchive),
            ("compose", CommandId::Compose),
        ]
    );
}

#[test]
fn an_inbox_with_mail_shows_the_list() {
    let mut focus = focus();
    let _ = focus.open(INBOX);
    assert!(
        empties(&counted(&mut focus, 4, 0)).is_empty(),
        "the list is what is drawn until something says otherwise"
    );
    assert_eq!(empties(&counted(&mut focus, 0, 0)).len(), 1);
    assert_eq!(
        empties(&counted(&mut focus, 1, 0)),
        vec![None],
        "mail arrived: the list again"
    );
    assert!(
        empties(&counted(&mut focus, 5, 0)).is_empty(),
        "still the list: nothing to draw again"
    );
}

#[test]
fn an_empty_place_says_why() {
    let mut focus = focus();
    let effects = focus.handle(Input::Command(CommandId::GoToSnoozed));
    let snoozed = ListScope::Focus(FocusScope::Snoozed);
    let (ticket, _) = asks(&effects).pop().expect("the view is counted");
    let _ = focus.handle(Input::Reply(
        ticket,
        Reply::Opened(Opened {
            scope: snoozed,
            accounts: vec![ADA],
            folders: vec![(MailboxId::new(1), true)],
            total: Ok(0),
            surfaced: None,
        }),
    ));
    let (ticket, request) = asks(&focus.page_wanted(0, 1))
        .pop()
        .expect("the first page");
    let Request::Page {
        page,
        stamp,
        start,
        count,
        ..
    } = request
    else {
        panic!("a page: {request:?}");
    };
    let _ = focus.handle(Input::Reply(
        ticket,
        Reply::Page {
            page,
            stamp,
            start,
            count,
            answer: Ok(PageAnswer::Threads {
                total: 0,
                rows: Vec::new(),
                labels: Vec::new(),
            }),
        },
    ));
    let effects = focus.landed(&NoRows, true);
    let keymap = focus.keymap().clone();
    assert_eq!(
        empties(&effects),
        vec![Some(focus_state::empty_place(
            &EmptyPlace::Snoozed,
            &keymap
        ))]
    );
}
