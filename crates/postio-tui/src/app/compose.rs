//! The composer: the controller says which one opens, what it offers about its thread, when its
//! draft is saved and when it closes; the keys and the text are this app's.

use postio_body::replying::ReplyKind;
use postio_focus::{ComposerKind, Input as Told, Intent, SurfaceKind};
use postio_model::MessageId;
use postio_ui::compose::{ChooserView, OlderReplyView};
use postio_ui::keymap::{KeyContext, Outcome};

use super::{App, Effect, Focus};

/// What the controller says about the composition's thread, which can come before its draft.
#[derive(Debug, Default)]
pub(crate) struct Writing {
    pub chooser: Option<ChooserView>,
    /// Keys 1, 2 and 3 are its while it holds the keyboard, as it does when it first shows.
    pub on_chooser: bool,
    pub older: Option<OlderReplyView>,
    pub newer: Vec<postio_model::listing::MessageSummary>,
    pub quoted: bool,
}

impl App {
    /// The controller's clock saves the draft once the typing stops.
    pub(super) fn edited(&mut self) -> Vec<Effect> {
        let mut effects = self.tell(Told::ComposerEdited);
        effects.push(Effect::Redraw);
        effects
    }

    pub(crate) fn writing(&self) -> &Writing {
        &self.writing
    }

    pub(super) fn show_composer_intent(&mut self, intent: Intent) -> Vec<Effect> {
        let writing = &mut self.writing;
        match intent {
            Intent::Composer { kind, message } => {
                *writing = Writing::default();
                return self.write(kind, message);
            }
            Intent::CloseSurface(SurfaceKind::Composer) => return self.composer_closed(),
            Intent::SaveDraft { composition } => return self.save_for(composition),
            Intent::ComposerDraftWanted(edit) => {
                return match self.composer.as_ref().map(|c| c.draft()) {
                    Some(draft) => self.tell(Told::ComposerDraft(edit, Box::new(draft))),
                    None => Vec::new(),
                };
            }
            Intent::ComposerDraft(draft) => return self.redraft(*draft),
            Intent::NewerMessages(Some(ids)) => return vec![Effect::ReadNewer(ids)],
            Intent::ForwardChooser(view) => {
                writing.on_chooser |= writing.chooser.is_none();
                writing.chooser = Some(*view);
            }
            Intent::OlderReply(view) => writing.older = view.map(|view| *view),
            Intent::NewerMessages(None) => writing.newer.clear(),
            Intent::QuotedExpanded(expanded) => writing.quoted = expanded,
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    pub(super) fn newer_read(
        &mut self,
        rows: Vec<postio_model::listing::MessageSummary>,
    ) -> Vec<Effect> {
        if self
            .writing
            .older
            .as_ref()
            .is_none_or(|older| !older.newer_shown)
        {
            return Vec::new();
        }
        self.writing.newer = rows;
        vec![Effect::Redraw]
    }

    /// A card's key, as the controller names it, chooses it: the registry offers this terminal
    /// no key of its own for them. Any other key puts the keyboard in the fields, with nothing typed.
    pub(super) fn forward_chooser_key(&mut self, key: &crossterm::event::KeyEvent) -> Vec<Effect> {
        use crossterm::event::{KeyCode, KeyModifiers};
        let card = match key.code {
            KeyCode::Char(typed) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
                self.writing.chooser.as_ref().and_then(|chooser| {
                    chooser.cards.iter().position(|card| {
                        card.key.as_deref() == Some(typed.encode_utf8(&mut [0; 4]))
                    })
                })
            }
            _ => None,
        };
        if let Some(card) = card {
            return self.forward_card_clicked(card);
        }
        if let Outcome::Command(id) = self.keys.press(key, KeyContext::Composer, false)
            && id == "back"
        {
            return self.composer_command(&id);
        }
        self.writing.on_chooser = false;
        vec![Effect::Redraw]
    }

    pub(super) fn forward_card_clicked(&mut self, index: usize) -> Vec<Effect> {
        let form = self
            .writing
            .chooser
            .as_ref()
            .and_then(|chooser| chooser.cards.get(index))
            .map(|card| card.form);
        match form {
            Some(form) => self.tell(Told::ForwardFormClicked(form)),
            None => Vec::new(),
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
            // Built as This; the controller has the engine build any other form, which replaces it.
            ComposerKind::Forward { .. } => reply(ReplyKind::Forward),
            ComposerKind::Draft => message.map(|message| vec![Effect::Resume(message)]),
            ComposerKind::ThreadDraft { draft } => Some(vec![Effect::ResumeDraft(draft)]),
            _ => None,
        }
        .unwrap_or_default()
    }

    /// The same composition, so its saves keep writing the same draft.
    fn redraft(&mut self, draft: postio_model::Draft) -> Vec<Effect> {
        let Some(composer) = self.composer.as_ref() else {
            return self.compose(draft);
        };
        let identities = self
            .accounts
            .iter()
            .find(|account| account.id == draft.account_id)
            .map(|account| account.identities.clone())
            .unwrap_or_default();
        self.composer = Some(
            crate::composer::Composer::new(composer.generation(), draft)
                .with_identities(identities),
        );
        self.edited()
    }

    /// Its draft waits for the controller's word on saving it.
    fn composer_closed(&mut self) -> Vec<Effect> {
        self.closed_composer = self.composer.take();
        self.writing = Writing::default();
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

    /// Closed with nothing said about saving it and nothing worth keeping: discarded if it was
    /// ever saved.
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

    #[test]
    fn escape_leaves_the_composer_and_saves_what_was_written() {
        let mut app = app((120, 36));
        app.compose(postio_model::Draft::new(postio_model::AccountId::new(1)));
        for _ in 0..3 {
            update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        }
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
    fn forwarding_a_thread_offers_the_chooser_and_its_choice_replaces_the_draft() {
        use crate::test_support::{conversation, local, places, press, screen, seed_places};
        use postio_model::ForwardForm;
        let mut app = app((120, 36));
        seed_places(&mut app, places());
        let mut thread = conversation(1, "Ada", "Tide gate", "", local(23, 9, 0));
        thread.message_count = 3;
        crate::test_support::show_focus(
            &mut app,
            vec![postio_ui::focus_list::FocusRow::conversation(thread)],
        );
        let effects = update(&mut app, press('f'));
        let message = MessageId::new(1);
        assert!(
            effects.contains(&Effect::ReplySource {
                kind: ReplyKind::Forward,
                message
            }),
            "{effects:?}"
        );
        let found = crate::composer::tests::a_message_and_its_account();
        update(
            &mut app,
            Input::ReplySource {
                kind: ReplyKind::Forward,
                found: Some(Box::new(found)),
            },
        );
        let drawn = screen(120, 36, &app);
        assert!(drawn.contains("Whole thread"), "{drawn}");
        let effects = update(&mut app, press('2'));
        let ticket = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Focus {
                    ticket,
                    request:
                        postio_focus::Request::SetForwardForm {
                            form: ForwardForm::Thread,
                            ..
                        },
                } => Some(*ticket),
                _ => None,
            })
            .unwrap_or_else(|| panic!("2 asks for the whole thread: {effects:?}"));
        let generation = app.composer().map(|composer| composer.generation());
        let mut whole = postio_model::Draft::new(postio_model::AccountId::new(1));
        whole.subject = "Fwd: the whole thread".to_owned();
        update(
            &mut app,
            Input::FocusReply(
                ticket,
                postio_focus::Reply::Forwarded {
                    form: ForwardForm::Thread,
                    answer: Ok(postio_model::Forwarded::Draft(Box::new(whole))),
                },
            ),
        );
        let composer = app.composer().expect("still writing");
        assert_eq!(composer.draft().subject, "Fwd: the whole thread");
        assert_eq!(
            Some(composer.generation()),
            generation,
            "the same composition"
        );
    }
}
