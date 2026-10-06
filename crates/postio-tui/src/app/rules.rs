//! The digest rules (`g d`) and the rule dialog (`d`).
//!
//! The state is `crate::rules` and `crate::rule_dialog`; the drawing is
//! `view::rules` and `view::rule_dialog`. The rules take Filtered's keys
//! (`j`, `k`, `Return`, `g i`) and `Delete`; the dialog is typed into and
//! walked with `Tab`.

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent};
use postio_model::{EmailAddress, MessageId};
use postio_ui::keymap::{KeyContext, Outcome};
use tui_input::backend::crossterm::EventHandler;

use super::{App, Effect, Focus, Tone};
use crate::ask::{Answer, Ask};
use crate::rule_dialog::{Field, Form};
use crate::rules::Rules;
use crate::surface::Part;

impl App {
    /// The digest rules, while they are the window's body.
    pub fn rules(&self) -> Option<&Rules> {
        self.surfaces.rules.as_ref()
    }

    /// The rule dialog, while it is open.
    pub fn rule_form(&self) -> Option<&Form> {
        self.surfaces.rule.as_ref().map(|(form, _)| form)
    }

    /// How many rules show at once: the body less the footer.
    pub fn rules_height(&self) -> usize {
        usize::from(self.window().list.height.saturating_sub(1))
    }

    /// `g d`: every rule, in place of the strip and the list.
    pub(super) fn go_to_digest_rules(&mut self) -> Vec<Effect> {
        let list = Rules::new(self.features.digests.0.clone());
        let names = list.names();
        self.surfaces.rules = Some(list);
        self.focus = Focus::Rules;
        let mut effects = vec![Effect::Redraw];
        if !names.is_empty() {
            effects.push(Effect::Ask(Ask::Waiting(names)));
        }
        effects
    }

    /// Back to the inbox, as it was.
    fn leave_rules(&mut self) -> Vec<Effect> {
        self.surfaces.rules = None;
        self.focus = Focus::List;
        let mut effects = vec![Effect::Redraw];
        if !matches!(self.scope, Some(postio_model::ListScope::Focus(_))) {
            effects.extend(self.command("go_to_inbox"));
        }
        effects
    }

    /// `config.toml` was read again: the rules listed follow it.
    pub(super) fn rules_reread(&mut self) -> Vec<Effect> {
        let rules = self.features.digests.0.clone();
        let Some(list) = self.surfaces.rules.as_mut() else {
            return Vec::new();
        };
        list.reread(rules);
        let height = self.rules_height();
        let names = {
            let list = self.surfaces.rules.as_mut().expect("just read");
            list.reveal(height);
            list.names()
        };
        let mut effects = vec![Effect::Redraw];
        if !names.is_empty() {
            effects.push(Effect::Ask(Ask::Waiting(names)));
        }
        effects
    }

    /// A key with the keyboard in the rules.
    pub(super) fn rules_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let outcome = match self.keys.press(key, KeyContext::Filtered, false) {
            // `Delete` is the list's key for removing; here it removes a rule.
            Outcome::Unhandled => match self.keys.press(key, KeyContext::List, false) {
                Outcome::Command(id) if id == "delete" => Outcome::Command(id),
                _ => Outcome::Unhandled,
            },
            other => other,
        };
        let Outcome::Command(id) = outcome else {
            return Vec::new();
        };
        self.rules_command(&id).unwrap_or_else(|| {
            let mut effects = self.leave_rules();
            effects.extend(self.command(&id));
            effects
        })
    }

    /// What the rules do for `id`, or `None` when it is not theirs.
    pub(super) fn rules_command(&mut self, id: &str) -> Option<Vec<Effect>> {
        let height = self.rules_height();
        let list = self.surfaces.rules.as_mut()?;
        if list.removing().is_some() {
            return Some(match id {
                "open_message" => self.confirm_remove(),
                "back" => {
                    list.keep();
                    vec![Effect::Redraw]
                }
                _ => Vec::new(),
            });
        }
        match id {
            "next_message" => list.step(1),
            "prev_message" => list.step(-1),
            "first_message" => list.go_to(0),
            "last_message" => list.last(),
            "open_message" => {
                let Some((rule, _)) = list.focused() else {
                    return Some(Vec::new());
                };
                let rule = rule.clone();
                return Some(self.open_rule_dialog(Form::edit(&rule)));
            }
            "delete" => list.ask_remove(),
            "back" | "go_to_inbox" => return Some(self.leave_rules()),
            "quit" | "undo" | "cheat_sheet" => return Some(self.global(id)),
            _ => return None,
        }
        list.reveal(height);
        Some(vec![Effect::Redraw])
    }

    /// `Return` over the question: remove the rule, and let what it held
    /// come back.
    fn confirm_remove(&mut self) -> Vec<Effect> {
        let Some(list) = self.surfaces.rules.as_mut() else {
            return Vec::new();
        };
        let Some(name) = list.removing().map(|remove| remove.name.clone()) else {
            return Vec::new();
        };
        list.keep();
        vec![Effect::Ask(Ask::DeleteRule(name)), Effect::Redraw]
    }

    /// The senders of what a verb would aim at: the selection, or the row
    /// under the cursor, or the message open; once each.
    fn aimed_senders(&self) -> Vec<EmailAddress> {
        let picked: Vec<MessageId> = match self.selection.selection() {
            postio_core::Selection::These(picked) => picked,
            postio_core::Selection::Everything { .. } => Vec::new(),
        };
        let mut rows: Vec<&crate::row::Row> = Vec::new();
        if self.focus == Focus::Reader
            && let Some(row) = self.row_of_open()
        {
            rows.push(row);
        } else if picked.is_empty() {
            rows.extend(self.row_at(self.cursor));
        } else {
            rows.extend(
                picked
                    .iter()
                    .filter_map(|message| self.list.row_of(*message)),
            );
        }
        let mut senders: Vec<EmailAddress> = Vec::new();
        for row in rows {
            let Some(address) = row.address.as_deref() else {
                continue;
            };
            if row.kind == crate::row::Kind::Digest
                || senders
                    .iter()
                    .any(|sender| sender.address.eq_ignore_ascii_case(address))
            {
                continue;
            }
            let name = (row.count <= 1 && !row.from.as_str().is_empty())
                .then(|| row.from.as_str().to_owned());
            senders.push(EmailAddress::new(name, address.to_owned()));
        }
        senders
    }

    /// The one message "Digest mail like this" would check other mail
    /// against: the cursor's, when nothing beyond it is selected, and only
    /// when the person has a model for it.
    fn like_this_message(&self) -> Option<MessageId> {
        if !self.features.like_this || self.focus == Focus::Reader {
            return None;
        }
        let single = match self.selection.selection() {
            postio_core::Selection::These(picked) => picked.len() <= 1,
            postio_core::Selection::Everything { .. } => false,
        };
        let row = self.row_at(self.cursor)?;
        (single && row.kind == crate::row::Kind::Message).then_some(row.id)
    }

    /// `d`: a new rule for the senders aimed at, or, in a digest, that
    /// digest's rule.
    pub(super) fn digest_rule(&mut self) -> Vec<Effect> {
        if self.focus == Focus::Digest {
            let Some(name) = self.digest().map(|window| window.rule.as_str().to_owned()) else {
                return Vec::new();
            };
            let rule = self
                .features
                .digests
                .0
                .iter()
                .find(|rule| rule.name.trim() == name.trim())
                .cloned();
            return match rule {
                Some(rule) => self.open_rule_dialog(Form::edit(&rule)),
                None => self.say(postio_ui::focus_target::RULE_MISSING),
            };
        }
        let senders = self.aimed_senders();
        if senders.is_empty() {
            return Vec::new();
        }
        let like_this = self.like_this_message();
        self.open_rule_dialog(Form::new_rule(&senders, like_this))
    }

    /// `L` over the inbox: a new rule for the message's sender, asking the
    /// model for mail like it at once.
    pub(super) fn digest_like_this(&mut self) -> Vec<Effect> {
        if !self.features.like_this {
            return self.say(postio_ui::digest::LIKE_THIS_NEEDS_A_MODEL);
        }
        let Some(message) = self.like_this_message() else {
            return Vec::new();
        };
        let senders = self.aimed_senders();
        if senders.is_empty() {
            return Vec::new();
        }
        let mut effects = self.open_rule_dialog(Form::new_rule(&senders, Some(message)));
        effects.push(Effect::Ask(Ask::LikeThis(message)));
        effects
    }

    /// Open the dialog over wherever the keyboard is, and read what the
    /// rule would have caught.
    fn open_rule_dialog(&mut self, mut form: Form) -> Vec<Effect> {
        let ask = form.preview_ask();
        let from = if self.focus == Focus::RuleDialog {
            self.surfaces
                .rule
                .as_ref()
                .map_or(Focus::List, |(_, from)| *from)
        } else {
            self.focus
        };
        self.surfaces.rule = Some((form, from));
        self.focus = Focus::RuleDialog;
        vec![Effect::Ask(ask), Effect::Redraw]
    }

    /// Put the dialog away: the keyboard goes back where it was.
    fn close_rule_dialog(&mut self) -> Vec<Effect> {
        if let Some((_, from)) = self.surfaces.rule.take() {
            self.focus = from;
        }
        vec![Effect::Redraw]
    }

    /// A key with the keyboard in the rule dialog.
    pub(super) fn rule_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some((form, _)) = self.surfaces.rule.as_mut() else {
            return Vec::new();
        };
        let typing = form.typing();
        // The walk and the choices are not text, whichever field is on.
        match key.code {
            KeyCode::Tab => {
                form.walk(1);
                return vec![Effect::Redraw];
            }
            KeyCode::BackTab => {
                form.walk(-1);
                return vec![Effect::Redraw];
            }
            KeyCode::Left | KeyCode::Up | KeyCode::Right | KeyCode::Down if !typing => {
                let by = if matches!(key.code, KeyCode::Left | KeyCode::Up) {
                    -1
                } else {
                    1
                };
                form.change(by);
                return vec![Effect::Redraw];
            }
            _ => {}
        }
        // A text field's own Return writes the rule and its Escape gives it up.
        if typing {
            match key.code {
                KeyCode::Enter => return self.rule_confirm(),
                KeyCode::Esc => return self.close_rule_dialog(),
                _ => {}
            }
        }
        match self.keys.press(key, KeyContext::Picker, typing) {
            Outcome::Command(id) => match id.as_str() {
                "picker_confirm" => return self.rule_confirm(),
                "back" => return self.close_rule_dialog(),
                _ => {}
            },
            Outcome::Pending(_) => return Vec::new(),
            Outcome::Unhandled => {}
        }
        if !typing {
            // `L` is the inbox's key for mail like this one.
            if let Outcome::Command(id) = self.keys.press(key, KeyContext::List, false)
                && id == "digest_like_this"
            {
                return self.rule_like_this();
            }
            return Vec::new();
        }
        let Some((form, _)) = self.surfaces.rule.as_mut() else {
            return Vec::new();
        };
        let field = form.field();
        let Some(input) = form.input() else {
            return Vec::new();
        };
        let before = input.value().to_owned();
        input.handle_event(&TerminalEvent::Key(*key));
        let changed = input.value() != before;
        let mut effects = vec![Effect::Redraw];
        if changed {
            form.say(None);
            if field == Field::Query {
                form.apply_query();
                effects.push(Effect::Ask(form.preview_ask()));
            }
        }
        effects
    }

    /// `Return`: whatever the control the keyboard is on does -- a link is
    /// followed, anything else writes the rule.
    fn rule_confirm(&mut self) -> Vec<Effect> {
        let Some((form, _)) = self.surfaces.rule.as_mut() else {
            return Vec::new();
        };
        match form.field() {
            Field::MatchInstead => {
                form.match_instead("");
                vec![Effect::Ask(form.preview_ask()), Effect::Redraw]
            }
            Field::LikeThis => self.rule_like_this(),
            _ => self.rule_create(),
        }
    }

    /// Ask the model which rule is like the message.
    fn rule_like_this(&mut self) -> Vec<Effect> {
        match self
            .surfaces
            .rule
            .as_ref()
            .and_then(|(form, _)| form.like_this())
        {
            Some(message) => vec![Effect::Ask(Ask::LikeThis(message))],
            None => Vec::new(),
        }
    }

    /// Create, or Save: write the rule, or say why it cannot be one.
    fn rule_create(&mut self) -> Vec<Effect> {
        let Some((form, _)) = self.surfaces.rule.as_mut() else {
            return Vec::new();
        };
        match form.draft() {
            Ok(rule) => {
                form.say(None);
                vec![
                    Effect::Ask(Ask::SaveRule {
                        replacing: form.replacing(),
                        rule,
                    }),
                    Effect::Redraw,
                ]
            }
            Err(sentence) => {
                form.say(Some(sentence));
                vec![Effect::Redraw]
            }
        }
    }

    /// What the host answered the rules and their dialog.
    pub(super) fn rules_answered(&mut self, answer: Answer) -> Vec<Effect> {
        match answer {
            Answer::Waiting { names, holds } => match holds {
                Ok(holds) => {
                    if let Some(list) = self.surfaces.rules.as_mut() {
                        list.held(&names, holds);
                    }
                }
                Err(reason) => tracing::warn!(%reason, "could not read what the rules hold"),
            },
            Answer::RuleDeleted { name, released } => match released {
                Ok(released) => {
                    if let Some(list) = self.surfaces.rules.as_mut() {
                        list.forget(&name);
                    }
                    let mut effects =
                        self.say(&postio_ui::focus_target::rule_removed(&name, released));
                    effects.push(Effect::RefreshPlaces);
                    return effects;
                }
                Err(reason) => return self.say_as(Tone::Failed, &reason, None),
            },
            Answer::Preview {
                generation,
                preview,
            } => {
                let Some((form, _)) = self.surfaces.rule.as_mut() else {
                    return Vec::new();
                };
                match preview {
                    Ok(preview) => {
                        form.previewed(generation, preview);
                    }
                    Err(reason) => form.say(Some(reason)),
                }
            }
            Answer::RuleSaved { name, saved } => match saved {
                Ok(()) => {
                    let mut effects = self.close_rule_dialog();
                    effects.extend(self.say(&postio_ui::focus_target::rule_saved(&name)));
                    effects.push(Effect::RefreshPlaces);
                    return effects;
                }
                Err(reason) => {
                    if let Some((form, _)) = self.surfaces.rule.as_mut() {
                        form.say(Some(reason));
                    }
                }
            },
            Answer::LikeThis(found) => {
                let Some((form, _)) = self.surfaces.rule.as_mut() else {
                    return Vec::new();
                };
                match found {
                    Ok(Some(rule)) => {
                        form.match_instead(&rule.queries.join(", "));
                        return vec![Effect::Ask(form.preview_ask()), Effect::Redraw];
                    }
                    Ok(None) => form.say(Some(postio_ui::digest::NOTHING_ALIKE.to_owned())),
                    Err(reason) => form.say(Some(reason)),
                }
            }
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    /// A click on a part of the rules or their dialog.
    pub(super) fn rules_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        let height = self.rules_height();
        match part {
            Part::RuleRow => {
                let Some(list) = self.surfaces.rules.as_mut() else {
                    return Vec::new();
                };
                if list.removing().is_some() {
                    return Vec::new();
                }
                if list.cursor() == index {
                    return self.rules_command("open_message").unwrap_or_default();
                }
                list.go_to(index);
                list.reveal(height);
                vec![Effect::Redraw]
            }
            Part::RemoveCancel => {
                if let Some(list) = self.surfaces.rules.as_mut() {
                    list.keep();
                }
                vec![Effect::Redraw]
            }
            Part::RemoveConfirm => self.confirm_remove(),
            Part::RuleCancel => self.close_rule_dialog(),
            Part::RuleCreate => self.rule_create(),
            Part::RuleField => {
                let Some((form, _)) = self.surfaces.rule.as_mut() else {
                    return Vec::new();
                };
                let Some(field) = form.fields().get(index).copied() else {
                    return Vec::new();
                };
                form.focus(field);
                match field {
                    Field::Cadence | Field::Day => form.change(1),
                    Field::MatchInstead | Field::LikeThis => return self.rule_confirm(),
                    Field::Create => return self.rule_create(),
                    Field::Time | Field::Query => {}
                }
                vec![Effect::Redraw]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_core::CommandId;
    use postio_model::MessageId;
    use postio_model::ids::DeliveryId;
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Focus, Input, update};
    use crate::ask::{Answer, Ask};
    use crate::test_support::{
        app, conversation, digest_row, held, key, local, newsletters_rule, places_with_features,
        press, screen, seed_places, show_focus, type_text,
    };

    fn rule(name: &str, query: &str, cadence: &str) -> postio_config::DigestRule {
        postio_config::Config::from_toml_str(&format!(
            "[[focus.digests]]\nname = \"{name}\"\nmatch = [\"{query}\"]\ncadence = \"{cadence}\"\n{}at = \"08:30\"\n",
            if cadence == "weekly" { "day = \"monday\"\n" } else { "" }
        ))
        .unwrap()
        .focus
        .digests
        .remove(0)
    }

    /// The inbox with two conversations, one digest, and two rules.
    fn inbox(size: (u16, u16)) -> App {
        let mut app = app(size);
        let mut places = places_with_features();
        places.features.digests = crate::places::Rules(vec![
            newsletters_rule(),
            rule("Receipts", "from:billing@example.com", "daily"),
        ]);
        places.features.digest_rules = 2;
        seed_places(&mut app, places);
        show_focus(
            &mut app,
            vec![
                FocusRow::conversation(conversation(
                    1,
                    "Ada Moreno",
                    "Atlas budget",
                    "",
                    local(23, 9, 0),
                )),
                FocusRow::conversation(conversation(
                    2,
                    "Grace Oyelaran",
                    "Harbor review",
                    "",
                    local(23, 8, 0),
                )),
                digest_row(1, "Newsletters", 14, 6),
            ],
        );
        app
    }

    fn open_rules(size: (u16, u16)) -> App {
        let mut app = inbox(size);
        update(&mut app, press('g'));
        let effects = update(&mut app, press('d'));
        let names = vec!["Newsletters".to_owned(), "Receipts".to_owned()];
        assert!(
            effects.contains(&Effect::Ask(Ask::Waiting(names.clone()))),
            "{effects:?}"
        );
        update(
            &mut app,
            Input::Answer(Answer::Waiting {
                names,
                holds: Ok(vec![3, 0]),
            }),
        );
        app
    }

    fn line_with<'a>(screen: &'a str, wanted: &str) -> &'a str {
        screen
            .lines()
            .find(|line| line.contains(wanted))
            .unwrap_or_else(|| panic!("no line holds {wanted:?} in\n{screen}"))
    }

    #[test]
    fn g_d_lists_every_rule_with_when_it_delivers_what_it_holds_and_what_it_matches() {
        let app = open_rules((120, 30));
        assert_eq!(app.focus(), Focus::Rules);
        let drawn = screen(120, 30, &app);
        let strip = drawn.lines().nth(1).unwrap();
        assert!(
            strip.contains("‹ Inbox g i   Digest rules · 2 rules"),
            "{strip}"
        );
        let row = line_with(&drawn, "Newsletters");
        assert!(row.contains("from:sender0@example.com"), "{row}");
        assert!(row.contains("Weekly, Sunday 09:00"), "{row}");
        assert!(row.contains("next "), "{row}");
        assert!(row.contains("holds 3"), "{row}");
        assert!(row.contains('▌'), "{row}");
        let row = line_with(&drawn, "Receipts");
        assert!(
            row.contains("Daily, 08:30") && row.contains("holds 0"),
            "{row}"
        );
    }

    #[test]
    fn with_no_rule_it_says_how_to_make_one() {
        let mut app = app((120, 30));
        seed_places(&mut app, places_with_features());
        show_focus(&mut app, vec![]);
        update(&mut app, press('g'));
        update(&mut app, press('d'));
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("No digest rules yet"), "{drawn}");
        assert!(
            drawn.contains("Press d on a message to digest its sender"),
            "{drawn}"
        );
    }

    #[test]
    fn j_and_k_walk_the_rules_and_g_i_goes_back() {
        let mut app = open_rules((120, 30));
        update(&mut app, press('j'));
        let drawn = screen(120, 30, &app);
        assert!(line_with(&drawn, "Receipts").contains('▌'), "{drawn}");
        update(&mut app, press('g'));
        update(&mut app, press('i'));
        assert_eq!(app.focus(), Focus::List);
        assert!(app.rules().is_none());
    }

    #[test]
    fn enter_edits_the_focused_rule_in_the_dialog() {
        let mut app = open_rules((120, 30));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::RuleDialog);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::Preview { queries, .. })
                if queries == &vec!["from:sender0@example.com".to_owned()])),
            "{effects:?}"
        );
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Digest rule · Newsletters"), "{drawn}");
        assert!(drawn.contains("sender0@example.com"), "{drawn}");
        assert!(
            drawn.contains("Weekly") && drawn.contains("Sunday") && drawn.contains("09:00"),
            "{drawn}"
        );
        assert!(drawn.contains("Save"), "{drawn}");
    }

    #[test]
    fn delete_asks_before_it_removes_and_says_what_came_back() {
        let mut app = open_rules((120, 30));
        let effects = update(&mut app, key(KeyCode::Delete, KeyModifiers::NONE));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::DeleteRule(_))))
        );
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Remove “Newsletters”?"), "{drawn}");
        assert!(
            drawn.contains("What it holds now — 3 messages — comes to the inbox"),
            "{drawn}"
        );
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects.contains(&Effect::Ask(Ask::DeleteRule("Newsletters".into()))),
            "{effects:?}"
        );
        let effects = update(
            &mut app,
            Input::Answer(Answer::RuleDeleted {
                name: "Newsletters".into(),
                released: Ok(3),
            }),
        );
        assert!(effects.contains(&Effect::RefreshPlaces), "{effects:?}");
        let drawn = screen(120, 30, &app);
        assert!(
            drawn.contains("Removed “Newsletters” · 3 messages back in the inbox"),
            "{drawn}"
        );
        assert!(!drawn.contains("Weekly, Sunday 09:00"), "{drawn}");
    }

    #[test]
    fn escape_over_the_question_keeps_the_rule() {
        let mut app = open_rules((120, 30));
        update(&mut app, key(KeyCode::Delete, KeyModifiers::NONE));
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Ask(_))));
        assert_eq!(app.focus(), Focus::Rules);
        assert!(screen(120, 30, &app).contains("Newsletters"));
    }

    /// `d` on the first conversation, the rule dialog open for its sender.
    fn digesting(app: &mut App) -> Vec<Effect> {
        update(app, press('d'))
    }

    #[test]
    fn d_on_a_row_opens_the_dialog_with_its_sender_and_the_weekly_sunday_default() {
        let mut app = inbox((120, 30));
        let effects = digesting(&mut app);
        assert_eq!(app.focus(), Focus::RuleDialog);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::Preview { queries, .. })
                if queries == &vec!["from:ada@example.com".to_owned()])),
            "{effects:?}"
        );
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Digest this sender"), "{drawn}");
        assert!(drawn.contains("From     ada@example.com"), "{drawn}");
        let deliver = line_with(&drawn, "Deliver");
        assert!(
            deliver.contains("Weekly") && deliver.contains("Sunday") && deliver.contains("09:00"),
            "{deliver}"
        );
        assert!(
            drawn.contains("Match a list or a search instead…"),
            "{drawn}"
        );
        assert!(
            drawn.contains("Mail from this sender with an invite, question or"),
            "{drawn}"
        );
        assert!(
            drawn.contains("to-do still comes straight to the inbox."),
            "{drawn}"
        );
        assert!(
            !drawn.contains("Digest mail like this"),
            "no model, no control:\n{drawn}"
        );
        assert!(drawn.contains("Create"), "{drawn}");
    }

    #[test]
    fn d_on_a_selection_digests_every_sender_once() {
        let mut app = inbox((120, 30));
        update(&mut app, press('x'));
        update(&mut app, press('j'));
        update(&mut app, press('x'));
        let effects = digesting(&mut app);
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Ask(Ask::Preview { queries, .. })
                if queries == &vec!["from:ada@example.com".to_owned(), "from:grace@example.com".to_owned()])),
            "{effects:?}"
        );
        assert!(screen(120, 30, &app).contains("Digest these senders"));
    }

    #[test]
    fn the_preview_says_what_the_rule_would_have_caught() {
        let mut app = inbox((120, 30));
        let effects = digesting(&mut app);
        let generation = effects
            .iter()
            .find_map(|e| match e {
                Effect::Ask(Ask::Preview { generation, .. }) => Some(*generation),
                _ => None,
            })
            .unwrap();
        update(
            &mut app,
            Input::Answer(Answer::Preview {
                generation,
                preview: Ok(postio_client::protocol::DigestPreview {
                    count: 9,
                    first: vec![
                        held(31, "Ada", "Atlas weekly", ""),
                        held(32, "Ada", "Atlas notes", ""),
                    ],
                }),
            }),
        );
        let drawn = screen(120, 30, &app);
        assert!(
            drawn.contains("Would have caught 9 messages in the last 90 days"),
            "{drawn}"
        );
        assert!(
            drawn.contains("Atlas weekly") && drawn.contains("Atlas notes"),
            "{drawn}"
        );
        assert!(drawn.contains("and 7 more"), "{drawn}");
        // An older answer is dropped.
        update(
            &mut app,
            Input::Answer(Answer::Preview {
                generation: generation + 5,
                preview: Ok(postio_client::protocol::DigestPreview {
                    count: 1,
                    first: vec![],
                }),
            }),
        );
        assert!(screen(120, 30, &app).contains("caught 9 messages"));
    }

    #[test]
    fn cadence_day_and_time_are_changed_in_the_dialog_and_written_on_create() {
        use postio_client::protocol::{DigestRuleDraft, RuleDay};
        let mut app = inbox((120, 30));
        digesting(&mut app);
        // Cadence: Weekly -> Monthly with the arrow; the day becomes a date.
        update(&mut app, key(KeyCode::Right, KeyModifiers::NONE));
        assert!(screen(120, 30, &app).contains("Monthly"));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Right, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        // The time: replace it.
        for _ in 0..5 {
            update(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        }
        type_text(&mut app, "18:30");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let saved = effects.iter().find_map(|e| match e {
            Effect::Ask(Ask::SaveRule { replacing, rule }) => {
                Some((replacing.clone(), rule.clone()))
            }
            _ => None,
        });
        let (replacing, rule) = saved.unwrap_or_else(|| panic!("{effects:?}"));
        assert_eq!(replacing, None);
        assert_eq!(
            rule,
            DigestRuleDraft {
                name: "Ada Moreno".into(),
                queries: vec!["from:ada@example.com".into()],
                cadence: postio_model::listing::Cadence::Monthly,
                day: Some(RuleDay::OfMonth(2)),
                at: chrono::NaiveTime::from_hms_opt(18, 30, 0).unwrap(),
            }
        );
    }

    #[test]
    fn a_time_that_is_no_time_says_so_and_writes_nothing() {
        let mut app = inbox((120, 30));
        digesting(&mut app);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        type_text(&mut app, "x");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::SaveRule { .. }))),
            "{effects:?}"
        );
        assert!(screen(120, 30, &app).contains("Give the time as 24-hour HH:MM, such as 09:00"));
    }

    #[test]
    fn saving_closes_the_dialog_says_so_and_reads_the_rules_again() {
        let mut app = inbox((120, 30));
        digesting(&mut app);
        let effects = update(
            &mut app,
            Input::Answer(Answer::RuleSaved {
                name: "Ada".into(),
                saved: Ok(()),
            }),
        );
        assert!(effects.contains(&Effect::RefreshPlaces), "{effects:?}");
        assert_eq!(app.focus(), Focus::List);
        assert!(screen(120, 30, &app).contains("Digest rule “Ada” saved"));
        // A refusal stays in the dialog, in its words.
        digesting(&mut app);
        update(
            &mut app,
            Input::Answer(Answer::RuleSaved {
                name: "Ada".into(),
                saved: Err("A rule called Ada is already there".into()),
            }),
        );
        assert_eq!(app.focus(), Focus::RuleDialog);
        assert!(screen(120, 30, &app).contains("A rule called Ada is already there"));
    }

    #[test]
    fn match_instead_swaps_the_senders_for_typed_queries_previewed_as_they_change() {
        let mut app = inbox((120, 30));
        digesting(&mut app);
        // Tab to the link and choose it.
        for _ in 0..3 {
            update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        }
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::Preview { .. }))),
            "{effects:?}"
        );
        let effects = type_text(&mut app, "list:weekly.example.org, from:a@b.test");
        let last = effects
            .iter()
            .rev()
            .find_map(|e| match e {
                Effect::Ask(Ask::Preview { queries, .. }) => Some(queries.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            last,
            vec![
                "list:weekly.example.org".to_owned(),
                "from:a@b.test".to_owned()
            ]
        );
        let drawn = screen(120, 30, &app);
        assert!(
            drawn.contains("list:weekly.example.org, from:a@b.test"),
            "{drawn}"
        );
        assert!(
            !drawn.contains("Match a list or a search instead"),
            "{drawn}"
        );
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::SaveRule { rule, .. })
            if rule.name == "list:weekly.example.org, from:a@b.test")),
            "{effects:?}"
        );
    }

    #[test]
    fn escape_closes_the_dialog_and_writes_nothing() {
        let mut app = inbox((120, 30));
        digesting(&mut app);
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(Ask::SaveRule { .. })))
        );
        assert_eq!(app.focus(), Focus::List);
    }

    #[test]
    fn like_this_is_offered_only_with_a_model_and_asks_it() {
        let mut app = inbox((120, 30));
        let mut places = places_with_features();
        places.features.like_this = true;
        seed_places(&mut app, places);
        digesting(&mut app);
        let drawn = screen(120, 30, &app);
        assert!(
            line_with(&drawn, "Digest mail like this").contains('L'),
            "{drawn}"
        );
        let effects = update(&mut app, press('L'));
        assert!(
            effects.contains(&Effect::Ask(Ask::LikeThis(MessageId::new(1)))),
            "{effects:?}"
        );
        // It answers with queries: they become the typed rule.
        update(
            &mut app,
            Input::Answer(Answer::LikeThis(Ok(Some(
                postio_client::protocol::LikeThisRule {
                    queries: vec!["list:atlas.example.org".into()],
                    preview: postio_client::protocol::DigestPreview {
                        count: 4,
                        first: vec![],
                    },
                },
            )))),
        );
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("list:atlas.example.org"), "{drawn}");
        // And a model that finds nothing alike says so.
        let mut app = inbox((120, 30));
        let mut places = places_with_features();
        places.features.like_this = true;
        seed_places(&mut app, places);
        digesting(&mut app);
        update(&mut app, press('L'));
        update(&mut app, Input::Answer(Answer::LikeThis(Ok(None))));
        assert!(screen(120, 30, &app).contains("The model found nothing alike to digest"));
    }

    #[test]
    fn l_from_the_list_without_a_model_says_what_it_needs() {
        let mut app = inbox((120, 30));
        let effects = update(&mut app, press('L'));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Ask(_) | Effect::Send(_))),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::List);
        assert!(screen(120, 30, &app).contains("[focus.model]"));
    }

    #[test]
    fn d_in_a_digest_edits_its_rule_and_says_when_the_rule_is_gone() {
        let mut app = inbox((120, 36));
        update(&mut app, press('G'));
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        update(
            &mut app,
            Input::Answer(Answer::Digest {
                delivery: DeliveryId::new(1),
                messages: Ok(vec![held(21, "Harbor Weekly", "Tides", "")]),
                summary: Ok(None),
            }),
        );
        update(&mut app, press('d'));
        assert_eq!(app.focus(), Focus::RuleDialog);
        assert!(screen(120, 36, &app).contains("Digest rule · Newsletters"));
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            app.focus(),
            Focus::Digest,
            "back in the window it came from"
        );
        // The rule is gone from the file.
        let mut places = places_with_features();
        places.features.digests = crate::places::Rules(vec![]);
        seed_places(&mut app, places);
        update(&mut app, press('d'));
        assert_eq!(app.focus(), Focus::Digest);
        assert!(screen(120, 36, &app).contains("That digest's rule is no longer in config.toml"));
    }

    #[test]
    fn every_part_is_a_click() {
        use crate::surface::Part;
        use crate::test_support::{click, hits_of};
        use crate::view::hit::Target;
        let mut app = open_rules((120, 30));
        let hits = hits_of(120, 30, &app);
        let on = |wanted: Target| {
            (0..30).any(|y| (0..120).any(|x| hits.at(x, y).is_some_and(|hit| hit.target == wanted)))
        };
        assert!(on(Target::Surface(Part::RuleRow, 0)) && on(Target::Surface(Part::RuleRow, 1)));
        assert!(on(Target::Command(CommandId::GoToInbox.as_str())));
        update(
            &mut app,
            click(Target::Surface(Part::RuleRow, 1), false, false),
        );
        assert_eq!(app.rules().unwrap().cursor(), 1);
        let effects = update(
            &mut app,
            click(Target::Surface(Part::RuleRow, 1), false, false),
        );
        assert_eq!(
            app.focus(),
            Focus::RuleDialog,
            "a second click edits: {effects:?}"
        );
        // The dialog's own parts.
        let hits = hits_of(120, 30, &app);
        let on = |wanted: Target| {
            (0..30).any(|y| (0..120).any(|x| hits.at(x, y).is_some_and(|hit| hit.target == wanted)))
        };
        assert!(on(Target::Surface(Part::RuleCancel, 0)));
        assert!(on(Target::Surface(Part::RuleCreate, 0)));
        let effects = update(
            &mut app,
            click(Target::Surface(Part::RuleCreate, 0), false, false),
        );
        assert!(effects.iter().any(|e| matches!(e, Effect::Ask(Ask::SaveRule { replacing: Some(name), .. }) if name == "Receipts")), "{effects:?}");
        update(
            &mut app,
            click(Target::Surface(Part::RuleCancel, 0), false, false),
        );
        assert_eq!(app.focus(), Focus::Rules);
    }

    #[test]
    fn what_a_subject_or_a_name_holds_never_reaches_the_terminal() {
        let mut app = inbox((120, 30));
        let effects = digesting(&mut app);
        let generation = effects
            .iter()
            .find_map(|e| match e {
                Effect::Ask(Ask::Preview { generation, .. }) => Some(*generation),
                _ => None,
            })
            .unwrap();
        let mut bad = held(41, "Ada", "x", "");
        bad.subject = Some("\u{1b}[2Jwiped\u{7}".into());
        update(
            &mut app,
            Input::Answer(Answer::Preview {
                generation,
                preview: Ok(postio_client::protocol::DigestPreview {
                    count: 1,
                    first: vec![bad],
                }),
            }),
        );
        let drawn = screen(120, 30, &app);
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
            "{drawn:?}"
        );
        assert!(drawn.contains("wiped"), "{drawn}");
    }
}
