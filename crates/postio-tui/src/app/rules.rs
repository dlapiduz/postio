//! The digest rules (`g r`), which are this terminal's own, and the rule
//! dialog (`d`), which is the controller's, typed into and walked here.

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent};
use postio_core::CommandId;
use postio_focus::{Input as Told, Intent, SurfaceKind};
use postio_ui::keymap::{KeyContext, Outcome};
use tui_input::backend::crossterm::EventHandler;

use super::{App, Effect, Focus, Tone};
use crate::ask::{Answer, Ask};
use crate::rule_dialog::{Field, Form};
use crate::rules::Rules;
use crate::surface::Part;

impl App {
    pub fn rules(&self) -> Option<&Rules> {
        self.surfaces.rules.as_ref()
    }

    pub fn rule_owner(&self, name: &str) -> Option<&str> {
        self.features.rule_owner(name)
    }

    pub fn rule_form(&self) -> Option<&Form> {
        self.surfaces.rule.as_ref()
    }

    pub fn rules_height(&self) -> usize {
        usize::from(self.window().list.height.saturating_sub(1))
    }

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

    fn leave_rules(&mut self) -> Vec<Effect> {
        self.surfaces.rules = None;
        self.focus = Focus::List;
        let mut effects = vec![Effect::Redraw];
        if !matches!(self.scope, Some(postio_model::ListScope::Focus(_))) {
            effects.extend(self.command("go_to_inbox"));
        }
        effects
    }

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
                let name = rule.name.clone();
                return Some(self.tell(Told::EditRule { name }));
            }
            "delete" => list.ask_remove(),
            "back" | "go_to_inbox" => return Some(self.leave_rules()),
            "undo" | "cheat_sheet" => return Some(self.global(id)),
            _ => return None,
        }
        list.reveal(height);
        Some(vec![Effect::Redraw])
    }

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

    pub(super) fn digest_like_this(&mut self) -> Vec<Effect> {
        if !self.features.like_this {
            return self.say(postio_ui::digest::LIKE_THIS_NEEDS_A_MODEL);
        }
        let mut effects = self.tell(Told::Command(CommandId::DigestRule));
        if self
            .surfaces
            .rule
            .as_ref()
            .is_some_and(|form| form.view().like_this.is_some())
        {
            effects.extend(self.tell(Told::RuleLikeThis));
        }
        effects
    }

    pub(super) fn show_rule_intent(&mut self, intent: Intent) -> Result<Vec<Effect>, Box<Intent>> {
        match intent {
            Intent::OpenRule(view) => {
                let from = self.surfaces.rule.as_ref().map_or(self.focus, Form::from);
                self.surfaces.rule = Some(Form::new(*view, from));
                self.focus = Focus::RuleDialog;
            }
            Intent::Rule(view) => {
                if let Some(form) = self.surfaces.rule.as_mut() {
                    form.replace(*view);
                }
            }
            Intent::CloseSurface(SurfaceKind::Dialog) => {
                let Some(form) = self.surfaces.rule.take() else {
                    return Err(Box::new(Intent::CloseSurface(SurfaceKind::Dialog)));
                };
                self.focus = form.from();
                return Ok(vec![Effect::RefreshPlaces, Effect::Redraw]);
            }
            other => return Err(Box::new(other)),
        }
        Ok(vec![Effect::Redraw])
    }

    pub(super) fn rule_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(form) = self.surfaces.rule.as_mut() else {
            return Vec::new();
        };
        let typing = form.typing();
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
                return self.change_rule(by);
            }
            _ => {}
        }
        if typing {
            match key.code {
                KeyCode::Enter => return self.rule_confirm(),
                KeyCode::Esc => return self.tell(Told::Command(CommandId::Back)),
                _ => {}
            }
        }
        match self.keys.press(key, KeyContext::Picker, typing) {
            Outcome::Command(id) => match id.as_str() {
                "picker_confirm" => return self.rule_confirm(),
                "back" => return self.tell(Told::Command(CommandId::Back)),
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
                return self.tell(Told::RuleLikeThis);
            }
            return Vec::new();
        }
        let Some(form) = self.surfaces.rule.as_mut() else {
            return Vec::new();
        };
        let field = form.field();
        let Some(input) = form.input() else {
            return Vec::new();
        };
        let before = input.value().to_owned();
        input.handle_event(&TerminalEvent::Key(*key));
        let text = input.value().to_owned();
        if text == before {
            return vec![Effect::Redraw];
        }
        match field {
            Field::Query => self.tell(Told::RuleQuery { text }),
            _ => {
                let mut schedule = form.schedule().clone();
                schedule.at = text;
                self.tell(Told::RuleSchedule(schedule))
            }
        }
    }

    fn change_rule(&mut self, by: isize) -> Vec<Effect> {
        match self
            .surfaces
            .rule
            .as_ref()
            .and_then(|form| form.changed(by))
        {
            Some(schedule) => self.tell(Told::RuleSchedule(schedule)),
            None => Vec::new(),
        }
    }

    fn rule_confirm(&mut self) -> Vec<Effect> {
        let Some(form) = self.surfaces.rule.as_ref() else {
            return Vec::new();
        };
        match form.field() {
            Field::MatchInstead => self.tell(Told::RuleMatchInstead),
            Field::LikeThis => self.tell(Told::RuleLikeThis),
            _ => self.tell(Told::RuleCreate),
        }
    }

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
        }
        vec![Effect::Redraw]
    }

    pub(super) fn rules_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        let height = self.rules_height();
        let Some(list) = self.surfaces.rules.as_mut() else {
            return Vec::new();
        };
        match part {
            Part::RuleRow => {
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
                list.keep();
                vec![Effect::Redraw]
            }
            Part::RemoveConfirm => self.confirm_remove(),
            _ => Vec::new(),
        }
    }

    pub(super) fn rule_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        match part {
            Part::RuleCancel => self.tell(Told::Command(CommandId::Back)),
            Part::RuleCreate => self.tell(Told::RuleCreate),
            Part::RuleField => {
                let Some(form) = self.surfaces.rule.as_mut() else {
                    return Vec::new();
                };
                let Some(field) = form.fields().get(index).copied() else {
                    return Vec::new();
                };
                form.focus(field);
                match field {
                    Field::Cadence | Field::Day => self.change_rule(1),
                    Field::MatchInstead | Field::LikeThis | Field::Create => self.rule_confirm(),
                    Field::Time | Field::Query => vec![Effect::Redraw],
                }
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use postio_focus::{Reply, Request};
    use postio_model::MessageId;
    use postio_ui::focus_list::FocusRow;

    use crate::app::{App, Effect, Focus, Input, update};
    use crate::ask::{Answer, Ask};
    use crate::test_support::{
        app, conversation, digest_row, held, key, local, newsletters_rule, places_with_features,
        press, screen, seed_places, serve_focus, serve_preview, show_focus, type_text, with_model,
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

    fn inbox(size: (u16, u16), model: bool) -> App {
        let mut app = app(size);
        app.clock_for_tests(crate::test_support::now());
        let mut places = places_with_features();
        places.features.digests = crate::places::Rules(vec![
            newsletters_rule(),
            rule("Receipts", "from:billing@example.com", "daily"),
        ]);
        places.features.digest_rules = 2;
        if model {
            places.features.like_this = true;
            places.features.section = crate::places::Section::new(with_model());
        }
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
        let mut app = inbox(size, false);
        update(&mut app, press('g'));
        let effects = update(&mut app, press('r'));
        let names = vec!["Newsletters".to_owned(), "Receipts".to_owned()];
        assert!(effects.contains(&Effect::Ask(Ask::Waiting(names.clone()))));
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

    fn asked(effects: &[Effect], wanted: impl Fn(&Request) -> bool) -> bool {
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Focus { request, .. } if wanted(request)))
    }

    fn preview() -> postio_client::protocol::DigestPreview {
        postio_client::protocol::DigestPreview {
            count: 9,
            first: vec![
                held(31, "Ada", "Atlas weekly", ""),
                held(32, "Ada", "Atlas notes", ""),
            ],
        }
    }

    fn digesting(app: &mut App) -> Vec<Effect> {
        let effects = update(app, press('d'));
        serve_preview(app, effects, preview())
    }

    #[test]
    fn g_r_lists_every_rule_with_when_it_delivers_and_what_it_holds() {
        let app = open_rules((120, 30));
        assert_eq!(app.focus(), Focus::Rules);
        let drawn = screen(120, 30, &app);
        assert!(
            drawn
                .lines()
                .nth(1)
                .unwrap()
                .contains("‹ Inbox g i   Digest rules · 2 rules"),
            "{drawn}"
        );
        let row = line_with(&drawn, "Newsletters");
        assert!(
            row.contains("Weekly, Sunday 09:00") && row.contains("holds 3"),
            "{row}"
        );
        assert!(row.contains('▌'), "{row}");
    }

    #[test]
    fn delete_asks_before_it_removes_and_escape_keeps_the_rule() {
        let mut app = open_rules((120, 30));
        update(&mut app, key(KeyCode::Delete, KeyModifiers::NONE));
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Remove “Newsletters”?"), "{drawn}");
        update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::Rules);
        update(&mut app, key(KeyCode::Delete, KeyModifiers::NONE));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(effects.contains(&Effect::Ask(Ask::DeleteRule("Newsletters".into()))));
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
    }

    #[test]
    fn enter_on_a_rule_edits_it_in_the_controllers_dialog_and_a_click_does_too() {
        use crate::surface::Part;
        use crate::test_support::click;
        use crate::view::hit::Target;
        let mut app = open_rules((120, 30));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.focus(), Focus::RuleDialog);
        assert!(asked(&effects, |request| matches!(
            request,
            Request::DigestPreview { queries, .. } if queries == &["from:sender0@example.com"]
        )));
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Digest rule · Newsletters"), "{drawn}");
        assert!(drawn.contains("Save"), "{drawn}");
        update(
            &mut app,
            click(Target::Surface(Part::RuleCancel, 0), false, false),
        );
        assert_eq!(app.focus(), Focus::Rules, "back where it came from");
        update(&mut app, press('j'));
        update(
            &mut app,
            click(Target::Surface(Part::RuleRow, 1), false, false),
        );
        assert_eq!(app.focus(), Focus::RuleDialog, "a second click edits");
    }

    #[test]
    fn d_opens_the_dialog_with_the_sender_and_the_preview_says_what_it_would_have_caught() {
        let mut app = inbox((120, 30), false);
        let effects = update(&mut app, press('d'));
        assert_eq!(app.focus(), Focus::RuleDialog);
        serve_preview(&mut app, effects, preview());
        let drawn = screen(120, 30, &app);
        assert!(drawn.contains("Digest this sender"), "{drawn}");
        assert!(drawn.contains("From     ada@example.com"), "{drawn}");
        let deliver = line_with(&drawn, "Deliver");
        assert!(
            deliver.contains("Weekly") && deliver.contains("Sunday") && deliver.contains("09:00"),
            "{deliver}"
        );
        assert!(
            drawn.contains("Would have caught 9 messages in the last 90 days"),
            "{drawn}"
        );
        assert!(
            drawn.contains("Atlas weekly") && drawn.contains("and 7 more"),
            "{drawn}"
        );
        assert!(
            !drawn.contains("Digest mail like this"),
            "no model:\n{drawn}"
        );
    }

    #[test]
    fn cadence_day_and_time_are_changed_in_the_dialog_and_written_on_create() {
        use postio_client::protocol::{DigestRuleDraft, RuleDay};
        let mut app = inbox((120, 30), false);
        digesting(&mut app);
        update(&mut app, key(KeyCode::Right, KeyModifiers::NONE));
        assert!(screen(120, 30, &app).contains("Monthly"));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Right, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        for _ in 0..5 {
            update(&mut app, key(KeyCode::Backspace, KeyModifiers::NONE));
        }
        type_text(&mut app, "18:30");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let saved = effects.iter().find_map(|effect| match effect {
            Effect::Focus {
                request:
                    Request::SaveDigestRule {
                        replacing, draft, ..
                    },
                ..
            } => Some((replacing.clone(), draft.clone())),
            _ => None,
        });
        let (replacing, draft) = saved.unwrap_or_else(|| panic!("{effects:?}"));
        assert_eq!(replacing, None);
        assert_eq!(
            draft,
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
        let mut app = inbox((120, 30), false);
        digesting(&mut app);
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        type_text(&mut app, "x");
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            !asked(&effects, |request| matches!(
                request,
                Request::SaveDigestRule { .. }
            )),
            "{effects:?}"
        );
        assert!(screen(120, 30, &app).contains("Give the time as 24-hour HH:MM, such as 09:00"));
    }

    #[test]
    fn saving_closes_the_dialog_and_says_so_and_a_refusal_stays_in_its_words() {
        let mut app = inbox((120, 30), false);
        let effects = update(&mut app, press('d'));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE))
            .into_iter()
            .chain(effects)
            .collect::<Vec<_>>();
        let saved = |app: &mut App, answer: Result<(), String>| {
            serve_focus(app, effects.clone(), |request| match request {
                Request::SaveDigestRule { stamp, .. } => Some(Reply::RuleSaved {
                    stamp: *stamp,
                    answer: answer.clone(),
                }),
                _ => None,
            })
        };
        let seen = saved(&mut app, Ok(()));
        assert!(seen.contains(&Effect::RefreshPlaces), "{seen:?}");
        assert_eq!(app.focus(), Focus::List);
        assert!(screen(120, 30, &app).contains("Digest rule “Ada Moreno” saved"));
        let mut app = inbox((120, 30), false);
        let effects = update(&mut app, press('d'));
        let asks = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let refused = serve_focus(
            &mut app,
            asks.into_iter().chain(effects).collect(),
            |request| match request {
                Request::SaveDigestRule { stamp, .. } => Some(Reply::RuleSaved {
                    stamp: *stamp,
                    answer: Err("A rule called Ada is already there".into()),
                }),
                _ => None,
            },
        );
        let _ = refused;
        assert_eq!(app.focus(), Focus::RuleDialog);
        assert!(screen(120, 30, &app).contains("A rule called Ada is already there"));
    }

    #[test]
    fn match_instead_swaps_the_senders_for_typed_queries_previewed_as_they_change() {
        let mut app = inbox((120, 30), false);
        digesting(&mut app);
        for _ in 0..3 {
            update(&mut app, key(KeyCode::Tab, KeyModifiers::NONE));
        }
        update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        let effects = type_text(&mut app, "list:weekly.example.org, from:a@b.test");
        let last = effects.iter().rev().find_map(|effect| match effect {
            Effect::Focus {
                request: Request::DigestPreview { queries, .. },
                ..
            } => Some(queries.clone()),
            _ => None,
        });
        assert_eq!(
            last,
            Some(vec![
                "list:weekly.example.org".to_owned(),
                "from:a@b.test".to_owned()
            ])
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
    }

    #[test]
    fn escape_closes_the_dialog_and_writes_nothing() {
        let mut app = inbox((120, 30), false);
        digesting(&mut app);
        let effects = update(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            !asked(&effects, |request| matches!(
                request,
                Request::SaveDigestRule { .. }
            )),
            "{effects:?}"
        );
        assert_eq!(app.focus(), Focus::List);
    }

    #[test]
    fn like_this_is_offered_with_a_model_and_asks_it_and_without_one_l_says_what_it_needs() {
        let mut app = inbox((120, 30), true);
        digesting(&mut app);
        let drawn = screen(120, 30, &app);
        assert!(
            line_with(&drawn, "Digest mail like this").contains('L'),
            "{drawn}"
        );
        let effects = update(&mut app, press('L'));
        assert!(asked(&effects, |request| matches!(
            request,
            Request::DigestLikeThis { message, .. } if *message == MessageId::new(1)
        )));
        let mut app = inbox((120, 30), false);
        let effects = update(&mut app, press('L'));
        assert!(!asked(&effects, |_| true), "{effects:?}");
        assert_eq!(app.focus(), Focus::List);
        assert!(screen(120, 30, &app).contains("[focus.model]"));
    }

    #[test]
    fn d_in_a_digest_edits_its_rule_and_escape_comes_back_to_the_window() {
        use crate::test_support::serve_digest;
        let mut app = inbox((120, 36), false);
        update(&mut app, press('G'));
        let effects = update(&mut app, key(KeyCode::Enter, KeyModifiers::NONE));
        serve_digest(
            &mut app,
            effects,
            vec![held(21, "Harbor Weekly", "Tides", "")],
            None,
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
        assert!(app.rule_form().is_none());
    }

    #[test]
    fn what_a_subject_holds_never_reaches_the_terminal() {
        let mut app = inbox((120, 30), false);
        let effects = update(&mut app, press('d'));
        let mut bad = held(41, "Ada", "x", "");
        bad.subject = Some("\u{1b}[2Jwiped\u{7}".into());
        serve_preview(
            &mut app,
            effects,
            postio_client::protocol::DigestPreview {
                count: 1,
                first: vec![bad],
            },
        );
        let drawn = screen(120, 30, &app);
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
            "{drawn:?}"
        );
        assert!(drawn.contains("wiped"), "{drawn}");
    }
}
