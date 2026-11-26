use chrono::Local;
use postio_core::CommandId;
use postio_model::calendar::AgendaEntry;
use postio_search::results::ResultsTab;

use super::{Results, Words, cursor_moved};
use crate::feed::Step;
use crate::{FocusController, Request};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub when: String,
    pub title: String,
    pub location: String,
    pub calendar_name: String,
    pub colour: Option<String>,
    pub focused: bool,
    pub accessible: String,
}

#[derive(Debug, Default)]
pub(super) struct Events {
    pub(super) wanted: bool,
    pub(super) found: Option<Vec<AgendaEntry>>,
    pub(super) ignored: Vec<String>,
    asked: bool,
    pub(super) cursor: Option<u64>,
}

impl Events {
    pub(super) fn forget(&mut self) {
        self.found = None;
        self.ignored.clear();
        self.asked = false;
    }

    pub(super) fn len(&self) -> u64 {
        self.found.as_ref().map_or(0, |found| found.len() as u64)
    }

    pub(super) fn capped(&self) -> bool {
        self.len() >= u64::from(crate::AGENDA_PAGE)
    }
}

impl Results {
    pub(super) fn ask_events(&mut self) -> Option<Step> {
        let events = &mut self.events;
        if !events.wanted || events.found.is_some() || events.asked {
            return None;
        }
        events.asked = true;
        Some(Step::Ask(Request::EventSearch {
            text: self.query.clone(),
            limit: crate::AGENDA_PAGE,
            stamp: self.stamp,
        }))
    }

    pub(super) fn events_landed(
        &mut self,
        stamp: u64,
        answer: Result<(Vec<AgendaEntry>, Vec<String>), String>,
    ) -> bool {
        if stamp != self.stamp {
            return false;
        }
        let (found, ignored) = answer.unwrap_or_else(|error| {
            tracing::debug!(%error, "the events could not be searched");
            Default::default()
        });
        let events = &mut self.events;
        events.asked = false;
        events.cursor = match events.cursor {
            Some(at) if !found.is_empty() => Some(at.min(found.len() as u64 - 1)),
            _ if !found.is_empty() => Some(0),
            _ => None,
        };
        events.found = Some(found);
        events.ignored = ignored;
        self.events_instead_of_nothing();
        true
    }

    pub(super) fn events_instead_of_nothing(&mut self) {
        if self.tab == ResultsTab::Conversations && self.found_nothing() && self.events.len() > 0 {
            self.tab = ResultsTab::Events;
        }
    }

    fn cursor_event(&self) -> Option<&AgendaEntry> {
        self.events
            .found
            .as_ref()?
            .get(self.events.cursor? as usize)
    }

    pub(super) fn event_row(&self, position: u64, with: &Words<'_>) -> Option<EventRow> {
        let entry = self
            .events
            .found
            .as_ref()?
            .get(usize::try_from(position).ok()?)?;
        let found = postio_ui::calendar::found_event(entry, with.now.date_naive(), &Local);
        Some(EventRow {
            when: found.when,
            title: found.title,
            location: found.location,
            calendar_name: found.calendar_name,
            colour: found.colour,
            focused: self.events.cursor == Some(position),
            accessible: found.accessible,
        })
    }
}

impl FocusController {
    pub fn result_event(&self, position: u64) -> Option<EventRow> {
        let words = self.results_words();
        self.results.as_ref()?.event_row(position, &words)
    }

    pub(super) fn events_command(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let results = self.results.as_mut()?;
        if results.tab != ResultsTab::Events {
            return None;
        }
        let steps = match id {
            CommandId::NextMessage | CommandId::PrevMessage => {
                cursor_moved(results.step(if id == CommandId::NextMessage { 1 } else { -1 }))
            }
            CommandId::FirstMessage => cursor_moved(results.step(i64::MIN / 2)),
            CommandId::LastMessage => cursor_moved(results.step(i64::MAX / 2)),
            CommandId::OpenMessage => match results.cursor_event().cloned() {
                Some(found) => self.calendar_show(&found),
                None => Vec::new(),
            },
            CommandId::ToggleSelection
            | CommandId::SelectAll
            | CommandId::QuickLook
            | CommandId::SaveFile
            | CommandId::NextMatch
            | CommandId::PrevMatch => Vec::new(),
            _ if matches!(
                postio_ui::focus_target::dispatch(id),
                Some(postio_ui::focus_target::Dispatch::OnMail(_))
            ) || crate::pickers::opens_picker(id) =>
            {
                Vec::new()
            }
            _ => return None,
        };
        Some(steps)
    }
}
