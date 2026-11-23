//! The list, driven by `postio_focus::FocusController`: its intents become
//! this app's cursor, selection, rows and toast.

use postio_focus::{Effect as Said, Input as Told, Intent, ToastKind};

use super::{App, Effect, TOAST, Tone};
use crate::focus::{Held, RowsView};

impl App {
    /// Tell the controller `input` over the rows held, and do what it says.
    pub(super) fn tell(&mut self, input: Told) -> Vec<Effect> {
        let said = self.ctl.handle_on(input, &RowsView(&self.list));
        self.carry_out(said)
    }

    /// Open `scope`: the controller reads it, and the rows arrive as intents.
    pub(super) fn open_scope(&mut self, scope: postio_model::ListScope) -> Vec<Effect> {
        let said = self.ctl.open(scope);
        self.scope = Some(scope);
        self.account = self.account_of(scope);
        let mut effects = self.carry_out(said);
        effects.push(Effect::Redraw);
        effects
    }

    fn carry_out(&mut self, said: Vec<Said>) -> Vec<Effect> {
        let mut effects = Vec::new();
        for one in said {
            match one {
                Said::Show(intent) => effects.extend(self.show_intent(intent)),
                Said::Ask(ticket, request) => effects.push(Effect::Focus { ticket, request }),
                Said::Timer { token, after } => effects.push(Effect::FocusTimer { token, after }),
                _ => {}
            }
        }
        effects
    }

    fn show_intent(&mut self, intent: Intent) -> Vec<Effect> {
        match intent {
            Intent::ReplaceSource { total } => {
                let stamp = self.list.reset(total);
                self.cursor = 0;
                self.top = 0;
                let said = self.ctl.page_wanted(0, stamp);
                let mut effects = self.carry_out(said);
                effects.push(Effect::Redraw);
                effects
            }
            Intent::PagePending { page, .. } => {
                self.list.note_pending(page);
                Vec::new()
            }
            Intent::DeliverPage {
                stamp,
                page,
                total,
                rows,
            } => {
                let rows = rows.into_iter().map(Held::of).collect();
                if self.list.generation() == stamp {
                    let _ = self.list.set_total(total);
                }
                let delivered = self.list.deliver(stamp, page, rows);
                if delivered.stale {
                    return Vec::new();
                }
                self.reveal();
                vec![Effect::Redraw]
            }
            Intent::AbandonPage { stamp, page } | Intent::GiveUp { stamp, page } => {
                self.list.abandon(stamp, page);
                Vec::new()
            }
            Intent::RefreshList => {
                let stamp = self.list.generation();
                let held: Vec<u32> = self
                    .list
                    .resident_pages()
                    .into_iter()
                    .filter(|page| self.list.note_pending(*page))
                    .collect();
                if held.is_empty() {
                    self.list.invalidate();
                    return vec![Effect::Redraw];
                }
                let mut effects = Vec::new();
                for page in held {
                    let said = self.ctl.page_wanted(page, stamp);
                    effects.extend(self.carry_out(said));
                }
                effects
            }
            Intent::Filled => {
                let opened = self.ctl.take_opened();
                let said = self.ctl.landed(&RowsView(&self.list), opened);
                let mut effects = self.carry_out(said);
                effects.push(Effect::Redraw);
                effects
            }
            Intent::Cursor { position, .. } => {
                self.move_to(position);
                vec![Effect::Redraw]
            }
            Intent::Selection { selection, summary } => {
                self.selection.set(selection);
                self.selection_words = summary;
                vec![Effect::Redraw]
            }
            Intent::Counts(counts) => {
                self.counts = Some(counts);
                vec![Effect::Redraw]
            }
            Intent::Toast { text, kind } => self.toast(&text, &kind),
            _ => Vec::new(),
        }
    }

    fn toast(&mut self, text: &str, kind: &ToastKind) -> Vec<Effect> {
        match kind {
            ToastKind::Completed { undoable, seconds } => {
                let key = self
                    .keys
                    .key_for(postio_ui::keymap::KeyContext::List, "undo")
                    .filter(|_| *undoable);
                let after = seconds.map_or(TOAST, |seconds| {
                    std::time::Duration::from_secs(u64::from(seconds))
                });
                self.say_for(Tone::Worked, text, key, after)
            }
            ToastKind::Undone => self.say_as(Tone::Worked, text, None),
            ToastKind::Notice => self.say_as(Tone::Plain, text, None),
            ToastKind::Offer { .. } => self.say_as(Tone::Plain, text, None),
        }
    }
}

impl App {
    /// Whether the controller, not this app, runs `command` on the list.
    pub(super) fn the_controllers(command: postio_core::CommandId) -> bool {
        use postio_core::CommandId as C;
        matches!(
            command,
            C::NextMessage
                | C::PrevMessage
                | C::FirstMessage
                | C::LastMessage
                | C::ToggleSelection
                | C::ExtendSelectionDown
                | C::ExtendSelectionUp
                | C::SelectAll
                | C::ToggleHasAction
                | C::Back
                | C::AcceptInvite
                | C::DeclineInvite
                | C::DismissMarker
        ) || postio_ui::focus_target::dispatch(command).is_some()
    }

    /// Ask for every page in view that is neither here nor on its way.
    pub(super) fn fetches(&mut self) -> Vec<Effect> {
        let end = (self.top + self.fit_from(self.top)).min(self.list.total());
        let mut wanted = Vec::new();
        for position in self.top..end {
            if let Some(postio_ui::list::Lookup::Missing { request }) = self.list.row_at(position) {
                wanted.extend(request);
            }
        }
        let stamp = self.list.generation();
        let mut effects = Vec::new();
        for page in wanted {
            let said = self.ctl.page_wanted(page, stamp);
            effects.extend(self.carry_out(said));
        }
        effects
    }
}

#[cfg(any(test, feature = "test-support"))]
impl App {
    /// Open `scope` over `total` rows the way the host's answer would,
    /// with no page read yet.
    pub(crate) fn open_for_tests(&mut self, scope: postio_model::ListScope, total: u32) {
        let asked = self.ctl.open(scope);
        self.scope = Some(scope);
        self.account = self.account_of(scope);
        let ticket = asked.iter().find_map(|said| match said {
            Said::Ask(ticket, postio_focus::Request::OpenScope { .. }) => Some(*ticket),
            _ => None,
        });
        if let Some(ticket) = ticket {
            let opened = postio_focus::Opened {
                scope,
                accounts: self
                    .accounts
                    .iter()
                    .filter(|account| account.enabled)
                    .map(|account| account.id)
                    .collect(),
                folders: self
                    .folders
                    .iter()
                    .map(|folder| {
                        (
                            folder.id,
                            folder.role == postio_model::mailbox::MailboxRole::Inbox,
                        )
                    })
                    .collect(),
                total: Ok(total),
                surfaced: matches!(scope, postio_model::ListScope::Focus(_))
                    .then(|| Ok(Vec::new())),
            };
            let _ = self.tell(postio_focus::Input::Reply(
                ticket,
                postio_focus::Reply::Opened(opened),
            ));
        }
        self.list.reset(total);
        self.cursor = 0;
        self.top = 0;
    }

    pub(crate) fn counts_for_tests(&mut self, counts: postio_client::protocol::FocusCounts) {
        self.counts = Some(counts);
    }

    /// Deliver every page of the list from `row_at`; how many pages.
    pub(crate) fn serve_for_tests(&mut self, row_at: impl Fn(u32) -> Held) -> usize {
        let (stamp, total) = (self.list.generation(), self.list.total());
        let size = postio_ui::list::PAGE_SIZE;
        let pages = total.div_ceil(size);
        let _ = self.fetches();
        let mut delivered = 0;
        let wanted: Vec<u32> = (0..pages)
            .filter(|page| self.list.is_pending(*page))
            .collect();
        for page in wanted {
            delivered += 1;
            let rows = (page * size..((page + 1) * size).min(total))
                .map(&row_at)
                .collect();
            self.list.deliver(stamp, page, rows);
        }
        self.ctl.cursor_is(Some(self.cursor));
        let _ = self.ctl.take_opened();
        self.reveal();
        delivered
    }
}
