//! The composer: the controller says which one opens, when its draft is
//! saved and when it closes; the keys and the text are this app's.

use postio_body::replying::ReplyKind;
use postio_focus::{ComposerKind, Input as Told, Intent, SurfaceKind};
use postio_model::MessageId;

use super::{App, Effect, Focus};

impl App {
    pub(super) fn show_composer_intent(&mut self, intent: Intent) -> Vec<Effect> {
        match intent {
            Intent::Composer { kind, message } => self.write(kind, message),
            Intent::CloseSurface(SurfaceKind::Composer) => self.composer_closed(),
            Intent::SaveDraft { composition } => self.save_for(composition),
            Intent::ComposerDraftWanted(edit) => match self.composer.as_ref().map(|c| c.draft()) {
                Some(draft) => self.tell(Told::ComposerDraft(edit, Box::new(draft))),
                None => Vec::new(),
            },
            Intent::ComposerDraft(draft) => self.compose(*draft),
            _ => Vec::new(),
        }
    }

    fn write(&mut self, kind: ComposerKind, message: Option<MessageId>) -> Vec<Effect> {
        let reply = |kind| message.map(|message| vec![Effect::ReplySource { kind, message }]);
        match kind {
            ComposerKind::New => self
                .account_here()
                .map(|account| self.compose(postio_model::Draft::new(account))),
            ComposerKind::Reply => reply(ReplyKind::Reply),
            ComposerKind::ReplyAll => reply(ReplyKind::ReplyAll),
            ComposerKind::Forward { .. } => reply(ReplyKind::Forward),
            ComposerKind::Draft => message.map(|message| vec![Effect::Resume(message)]),
            _ => None,
        }
        .unwrap_or_default()
    }

    /// The composer leaves the screen; its draft waits for the controller's
    /// word on saving it.
    fn composer_closed(&mut self) -> Vec<Effect> {
        self.closed_composer = self.composer.take();
        self.detached = false;
        self.focus = match self.ctl.top_surface() {
            Some(SurfaceKind::Message) if self.reading.is_some() => Focus::Reader,
            _ => Focus::List,
        };
        vec![Effect::Redraw]
    }

    fn save_for(&mut self, composition: u64) -> Vec<Effect> {
        let composer = match self.composer.as_ref() {
            Some(composer) => Some(composer),
            None => self.closed_composer.as_ref(),
        };
        let kept = composer.map(|composer| (composer.generation(), composer.draft()));
        if self.composer.is_none() {
            self.closed_composer = None;
        }
        match kept {
            Some((generation, draft))
                if postio_model::draft::closing(&draft) == postio_model::draft::Closing::Keep =>
            {
                vec![Effect::SaveDraft {
                    generation,
                    composition: Some(composition),
                    draft: Box::new(draft),
                }]
            }
            Some((generation, draft)) => {
                let mut effects = vec![Effect::DiscardDraft {
                    generation,
                    known: draft.id.is_assigned().then_some(draft.id),
                }];
                effects.extend(self.tell(Told::DraftSaved {
                    composition,
                    saved: Ok(false),
                }));
                effects
            }
            // Sent, so there is nothing left of it to keep.
            None => self.tell(Told::DraftSaved {
                composition,
                saved: Ok(false),
            }),
        }
    }

    /// A composer that closed with nothing to say about saving it, and
    /// nothing worth keeping, is discarded if it was ever saved.
    pub(super) fn drop_closed_composer(&mut self) -> Vec<Effect> {
        let Some(composer) = self.closed_composer.as_ref() else {
            return Vec::new();
        };
        let draft = composer.draft();
        if postio_model::draft::closing(&draft) == postio_model::draft::Closing::Keep {
            return Vec::new();
        }
        let generation = composer.generation();
        self.closed_composer = None;
        vec![Effect::DiscardDraft {
            generation,
            known: draft.id.is_assigned().then_some(draft.id),
        }]
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;
    use crate::app::{Input, update};
    use crate::test_support::{app, key, type_text};

    fn writing() -> (App, Vec<Effect>) {
        let mut app = app((120, 36));
        let effects = app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        for _ in 0..3 {
            update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        }
        (app, effects)
    }

    #[test]
    fn a_composer_the_app_opens_is_one_the_controller_knows() {
        let (app, _) = writing();
        assert_eq!(app.ctl.top_surface(), Some(SurfaceKind::Composer));
    }

    #[test]
    fn typing_starts_the_controllers_clock_and_it_saves_the_draft_when_it_runs_out() {
        let (mut app, _) = writing();
        let effects = type_text(&mut app, "two words");
        let token = effects
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Effect::FocusTimer { token, .. } => Some(*token),
                _ => None,
            })
            .expect("an edit starts the autosave clock");
        let saved = update(&mut app, Input::FocusTimer(token));
        assert!(
            saved.iter().any(|effect| matches!(
                effect,
                Effect::SaveDraft {
                    composition: Some(_),
                    ..
                }
            )),
            "{saved:?}"
        );
    }

    #[test]
    fn escape_leaves_the_composer_and_saves_what_was_written() {
        let (mut app, _) = writing();
        type_text(&mut app, "two words");
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.composer().is_none());
        assert_eq!(app.focus(), Focus::List);
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::SaveDraft { .. })),
            "{effects:?}"
        );
    }

    #[test]
    fn leaving_a_draft_never_written_saves_nothing() {
        let (mut app, _) = writing();
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::List);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::SaveDraft { .. })),
            "{effects:?}"
        );
    }

    #[test]
    fn a_queued_send_is_the_controllers_toast_with_its_undo() {
        let mut app = app((120, 36));
        update(
            &mut app,
            Input::Queued {
                at: None,
                queued: Ok(postio_model::DraftId::new(4)),
            },
        );
        assert_eq!(
            app.notice(),
            Some(postio_ui::sending::QUEUED_TO_SEND),
            "the controller says it"
        );
    }
}
