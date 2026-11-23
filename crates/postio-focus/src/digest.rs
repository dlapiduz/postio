//! The digest window and the digest rule dialog. The digest opens on its summary when the person
//! has brought a model and one is written, else on its plain list. A reference's email opens in
//! the digest's own window and Esc returns to it; Linux's reading dialog stacks over the digest.

use chrono::{DateTime, Utc};
use postio_client::protocol::{DigestPreview, DigestRuleDraft};
use postio_core::{Command, CommandId, MessageTarget};
use postio_model::listing::{Cadence, MessageSummary};
use postio_model::summary::DigestSummary;
use postio_model::{DeliveryId, MessageId};
use postio_ui::digest::{self as words, Page, Schedule};
use postio_ui::focus_list::FocusRow;
use postio_ui::hints;
use postio_ui::rule_form::RuleForm;

use crate::cursor::{RowFacts, Rows};
use crate::feed::Step;
use crate::surfaces::{Host, Origin};
use crate::{FocusController, Intent, Reply, Request, SurfaceKind, ToastKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestView {
    pub delivery: DeliveryId,
    pub page: Page,
    pub title: String,
    pub subtitle: String,
    pub archive: String,
    pub archive_key: Option<String>,
    pub rule_line: String,
    pub rule_key: Option<String>,
    pub tabs: bool,
    pub list_tab: String,
    pub tab_key: Option<String>,
    pub rows: Vec<DigestLine>,
    pub focused: Option<u32>,
    pub topics: Vec<DigestTopic>,
    pub focused_reference: Option<u32>,
    pub card: Option<DigestCard>,
    pub footer: Option<String>,
    pub email: Option<DigestEmail>,
    pub back: String,
    pub back_key: Option<String>,
    pub loading: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestLine {
    pub message: MessageId,
    pub sender: String,
    pub subject: String,
    pub preview: Option<String>,
    pub time: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestTopic {
    pub heading: String,
    pub statements: Vec<DigestStatement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestStatement {
    pub index: u32,
    /// Plain text, never markup.
    pub text: String,
    pub number: u32,
    pub message: MessageId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestCard {
    pub title: String,
    pub hint: String,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestEmail {
    pub message: MessageId,
    pub number: Option<u32>,
    pub excerpt: Option<String>,
    pub banner: Option<String>,
}

#[derive(Debug)]
pub(crate) struct DigestWindow {
    delivery: DeliveryId,
    rule: String,
    cadence: Option<Cadence>,
    count: u32,
    senders: usize,
    at: DateTime<Utc>,
    rule_when: Option<String>,
    rows: Vec<MessageSummary>,
    summary: Option<DigestSummary>,
    page: Page,
    focused: Option<usize>,
    reference: Option<usize>,
    email: Option<(MessageId, Page)>,
    stamp: u64,
    loading: bool,
}

impl DigestWindow {
    pub(crate) fn facts(&self, index: usize) -> Option<RowFacts> {
        self.rows.get(index).map(|row| RowFacts {
            id: row.id,
            digest: false,
            threads: row.thread.into_iter().collect(),
            writes: false,
        })
    }

    pub(crate) fn index_of(&self, message: MessageId) -> Option<usize> {
        self.rows.iter().position(|row| row.id == message)
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn subject(&self, index: usize) -> Option<String> {
        self.rows.get(index)?.subject.clone()
    }

    fn statements(&self) -> usize {
        self.summary
            .as_ref()
            .map_or(0, |summary| summary.statements.len())
    }

    fn has_summary(&self) -> bool {
        self.statements() > 0
    }

    fn cited(&self, message: MessageId) -> Option<(usize, u32, String)> {
        self.summary
            .as_ref()?
            .statements
            .iter()
            .enumerate()
            .find(|(_, statement)| statement.reference.message == message)
            .map(|(index, statement)| {
                (
                    index,
                    statement.reference.number,
                    statement.reference.excerpt.clone(),
                )
            })
    }

    fn follow_reference(&mut self) {
        let message = self.reference.and_then(|index| {
            self.summary
                .as_ref()?
                .statements
                .get(index)
                .map(|statement| statement.reference.message)
        });
        if let Some(at) = message.and_then(|message| self.index_of(message)) {
            self.focused = Some(at);
        }
    }

    fn focused_message(&self) -> Option<&MessageSummary> {
        self.focused.and_then(|at| self.rows.get(at))
    }
}

#[derive(Debug)]
pub(crate) struct RuleDialog {
    form: RuleForm,
    schedule: Schedule,
    preview: Option<DigestPreview>,
    error: Option<String>,
    stamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleView {
    pub heading: String,
    pub from_label: String,
    pub from: Option<String>,
    pub query: Option<String>,
    pub placeholder: String,
    pub match_instead: Option<String>,
    pub like_this: Option<String>,
    pub deliver_label: String,
    pub schedule: Schedule,
    pub cadences: Vec<String>,
    pub weekdays: Vec<String>,
    pub note: String,
    pub create: String,
    pub create_key: Option<String>,
    pub preview_heading: Option<String>,
    pub preview: Vec<RulePreviewLine>,
    pub more: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePreviewLine {
    pub subject: String,
    pub day: String,
}

impl FocusController {
    pub(crate) fn open_digest(
        &mut self,
        row: MessageId,
        position: u32,
        rows: &dyn Rows,
    ) -> Vec<Step> {
        let digest = match rows.row(position) {
            Some(FocusRow::Digest(digest)) => Some(digest),
            _ => self.feed.digest(row),
        };
        let Some(digest) = digest else {
            return vec![Step::Show(Intent::OpenDigest { row })];
        };
        let stamp = self.stamp();
        let config = &self.states.config;
        let rule_when = config
            .digests
            .iter()
            .find(|rule| rule.name.trim() == digest.rule.trim())
            .and_then(words::rule_when);
        // A summary is read only when the person has brought a model.
        let summary = config
            .model_for(postio_config::model::ModelFeature::DigestSummary)
            .is_some();
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Digest, self.policy.caps.stacking);
        self.digest = Some(DigestWindow {
            delivery: digest.delivery,
            rule: digest.rule.clone(),
            cadence: digest.cadence,
            count: digest.count,
            senders: digest.senders.len(),
            at: digest.at,
            rule_when,
            rows: Vec::new(),
            summary: None,
            page: Page::List,
            focused: None,
            reference: None,
            email: None,
            stamp,
            loading: true,
        });
        steps.push(Step::Show(Intent::OpenDigest { row }));
        steps.extend(self.draw_digest());
        steps.push(Step::Ask(Request::DigestRead {
            delivery: digest.delivery,
            summary,
            stamp,
        }));
        steps
    }

    pub(crate) fn digest_answers(id: CommandId) -> bool {
        matches!(
            id,
            CommandId::NextReference
                | CommandId::PrevReference
                | CommandId::ToggleDigestSummary
                | CommandId::StopDigestingSender
                | CommandId::Unsubscribe
                | CommandId::DigestRule
                | CommandId::Archive
        )
    }

    pub(crate) fn digest_email(&self) -> Option<MessageId> {
        self.digest.as_ref()?.email.map(|(message, _)| message)
    }

    pub(crate) fn digest_command(&mut self, id: CommandId, rows: &dyn Rows) -> Option<Vec<Step>> {
        let window = self.digest.as_mut()?;
        let steps = match id {
            CommandId::Back => {
                if let Some((_, from)) = window.email.take() {
                    window.page = from;
                    self.draw_digest()
                } else {
                    self.close_digest()
                }
            }
            CommandId::GoToInbox => self.close_digest(),
            CommandId::Archive => {
                let delivery = window.delivery;
                let mut steps = self.verbs.archive_digest(delivery);
                steps.extend(self.close_digest());
                steps
            }
            CommandId::DigestRule => {
                let rule = window.rule.clone();
                self.edit_rule(&rule)
            }
            CommandId::NextReference | CommandId::PrevReference => {
                let by = if id == CommandId::NextReference {
                    1
                } else {
                    -1
                };
                self.step_reference(by)
            }
            CommandId::ToggleDigestSummary => match window.page.toggled(window.has_summary()) {
                Some(page) => {
                    window.page = page;
                    self.draw_digest()
                }
                None => Vec::new(),
            },
            CommandId::StopDigestingSender => {
                let Some(message) = window.focused_message() else {
                    return Some(Vec::new());
                };
                let id = message.id;
                let sender = message
                    .from
                    .as_ref()
                    .map(|from| from.address.clone())
                    .unwrap_or_default();
                vec![self.ask_first(
                    postio_ui::focus_target::stop_digesting_title(&sender),
                    postio_ui::focus_target::STOP_DIGESTING_BODY.to_owned(),
                    postio_ui::focus_target::STOP_DIGESTING.to_owned(),
                    false,
                    Command::StopDigestingSender {
                        target: MessageTarget::Messages(vec![id]),
                        stopped: true,
                        kept: None,
                    },
                )]
            }
            CommandId::Unsubscribe => match window.focused_message() {
                Some(message) => vec![Step::Ask(Request::Unsubscribe(message.id))],
                None => Vec::new(),
            },
            CommandId::NextMessage | CommandId::PrevMessage => {
                let by = if id == CommandId::NextMessage { 1 } else { -1 };
                match window.page {
                    Page::Summary => self.step_reference(by),
                    Page::List => self.step_digest_row(by),
                    Page::Email => {
                        let mut steps = self.step_digest_row(by);
                        if !steps.is_empty() {
                            steps = self.digest_email_at_focus();
                        }
                        steps
                    }
                }
            }
            CommandId::OpenMessage => match window.page {
                Page::Summary => self.open_reference(),
                Page::List => self.open_digest_row(rows),
                Page::Email => Vec::new(),
            },
            CommandId::Undo
            | CommandId::CheatSheet
            | CommandId::Search
            | CommandId::CommandPalette => return None,
            _ => Vec::new(),
        };
        Some(steps)
    }

    fn close_digest(&mut self) -> Vec<Step> {
        self.digest = None;
        self.surfaces.dismiss(SurfaceKind::Digest);
        let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Digest))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    fn step_reference(&mut self, by: i32) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(next) = words::step_reference(window.reference, by, window.statements()) else {
            return Vec::new();
        };
        window.reference = Some(next);
        window.follow_reference();
        self.draw_digest()
    }

    fn step_digest_row(&mut self, by: i32) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        if window.rows.is_empty() {
            return Vec::new();
        }
        let last = window.rows.len() as i64 - 1;
        let next = window
            .focused
            .map_or(0, |at| (at as i64 + i64::from(by)).clamp(0, last)) as usize;
        if window.focused == Some(next) {
            return Vec::new();
        }
        window.focused = Some(next);
        self.draw_digest()
    }

    fn open_reference(&mut self) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(message) = window.reference.and_then(|index| {
            window
                .summary
                .as_ref()?
                .statements
                .get(index)
                .map(|statement| statement.reference.message)
        }) else {
            return Vec::new();
        };
        window.email = Some((message, Page::Summary));
        window.page = Page::Email;
        self.show_digest_email(message)
    }

    fn open_digest_row(&mut self, _rows: &dyn Rows) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(at) = window.focused else {
            return Vec::new();
        };
        if self.policy.caps.stacking {
            return self.open_digest_message(at);
        }
        let Some(message) = window.rows.get(at).map(|row| row.id) else {
            return Vec::new();
        };
        window.email = Some((message, Page::List));
        window.page = Page::Email;
        self.show_digest_email(message)
    }

    fn digest_email_at_focus(&mut self) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(message) = window.focused_message().map(|row| row.id) else {
            return Vec::new();
        };
        let from = window.email.map_or(Page::List, |(_, from)| from);
        window.email = Some((message, from));
        self.show_digest_email(message)
    }

    fn show_digest_email(&mut self, message: MessageId) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        if let Some(at) = window.index_of(message) {
            window.focused = Some(at);
        }
        let index = window.index_of(message).unwrap_or(0) as u32;
        let total = window.rows.len() as u32;
        let mut steps = vec![Step::Show(Intent::OpenMessage {
            message,
            index,
            total,
            host: Host::Digest,
        })];
        steps.extend(self.draw_digest());
        steps
    }

    pub(crate) fn open_digest_message(&mut self, index: usize) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        let Some(row) = window.facts(index) else {
            return Vec::new();
        };
        window.focused = Some(index);
        let total = window.rows.len() as u32;
        let mut steps = self
            .surfaces
            .open(&row, index as u32, total, self.policy.caps.stacking);
        self.surfaces.set_origin(Origin::Digest);
        steps.extend(self.draw_digest());
        steps
    }

    pub(crate) fn digest_point(&mut self, index: u32) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        if index as usize >= window.rows.len() {
            return Vec::new();
        }
        window.focused = Some(index as usize);
        self.draw_digest()
    }

    pub(crate) fn digest_reference(&mut self, index: u32) -> Vec<Step> {
        let Some(window) = self.digest.as_mut() else {
            return Vec::new();
        };
        if index as usize >= window.statements() {
            return Vec::new();
        }
        window.reference = Some(index as usize);
        window.follow_reference();
        self.draw_digest()
    }

    pub(crate) fn digest_reply(&mut self, reply: Reply) -> Vec<Step> {
        match reply {
            Reply::DigestRead {
                stamp,
                rows,
                summary,
            } => {
                let Some(window) = self.digest.as_mut().filter(|it| it.stamp == stamp) else {
                    return Vec::new();
                };
                window.loading = false;
                match rows {
                    Ok(rows) => window.rows = rows,
                    Err(error) => tracing::warn!(%error, "Focus could not read a digest"),
                }
                window.summary = summary.filter(|summary| !summary.is_empty());
                let has_summary = window.has_summary();
                window.page = Page::opening(has_summary);
                window.reference = has_summary.then_some(0);
                window.focused = (!window.rows.is_empty()).then_some(0);
                window.follow_reference();
                self.draw_digest()
            }
            Reply::Unsubscribed(said) => vec![Step::Show(Intent::Toast {
                text: match said {
                    Ok(list) => postio_ui::focus_target::unsubscribed(&list),
                    Err(error) => error,
                },
                kind: ToastKind::Notice,
            })],
            _ => Vec::new(),
        }
    }

    fn draw_digest(&self) -> Vec<Step> {
        self.digest_view()
            .map(|view| vec![Step::Show(Intent::Digest(Box::new(view)))])
            .unwrap_or_default()
    }

    pub(crate) fn digest_view(&self) -> Option<DigestView> {
        let window = self.digest.as_ref()?;
        let keymap = self.keymap();
        let now = self.now();
        let mut title = words::window_title(window.cadence, &window.rule);
        let mut subtitle = words::window_subtitle(
            window.count,
            window.senders,
            window.at.with_timezone(&chrono::Local),
            now,
        );
        let email = window.email.map(|(message, _)| {
            let cited = window.cited(message);
            if let Some(row) = window.rows.iter().find(|row| row.id == message) {
                title = row.subject.clone().unwrap_or_default();
            }
            subtitle = words::source_line(window.index_of(message), window.rows.len());
            DigestEmail {
                message,
                number: cited.as_ref().map(|(_, number, _)| *number),
                excerpt: cited.as_ref().map(|(_, _, excerpt)| excerpt.clone()),
                banner: cited
                    .as_ref()
                    .map(|(_, number, _)| words::cited_banner(*number)),
            }
        });
        let statements = window
            .summary
            .as_ref()
            .map_or(&[][..], |summary| &summary.statements[..]);
        let card = window.reference.and_then(|index| {
            let statement = statements.get(index)?;
            let row = window
                .rows
                .iter()
                .find(|row| row.id == statement.reference.message)?;
            let sender = row.from.as_ref().map(|from| from.display().to_owned());
            Some(DigestCard {
                title: words::reference_title(
                    statement.reference.number,
                    sender.as_deref(),
                    row.subject.as_deref(),
                ),
                hint: words::OPEN_FULL_EMAIL.to_owned(),
                key: hints::key(keymap, CommandId::OpenMessage),
            })
        });
        let list_tab = words::list_tab(window.rows.len());
        Some(DigestView {
            delivery: window.delivery,
            page: window.page,
            title,
            subtitle,
            archive: words::archive_all(window.count),
            archive_key: hints::key(keymap, CommandId::Archive),
            rule_line: words::rule_line(window.rule_when.as_deref()),
            rule_key: hints::key(keymap, CommandId::DigestRule),
            tabs: window.has_summary(),
            list_tab: list_tab.clone(),
            tab_key: hints::key(keymap, CommandId::ToggleDigestSummary),
            rows: window
                .rows
                .iter()
                .map(|row| DigestLine {
                    message: row.id,
                    sender: row
                        .from
                        .as_ref()
                        .map(|from| from.display().to_owned())
                        .unwrap_or_default(),
                    subject: row.subject.clone().unwrap_or_default(),
                    preview: row.preview.clone(),
                    time: postio_ui::row::timestamp(row.received_at, now),
                })
                .collect(),
            focused: window.focused.map(|at| at as u32),
            topics: words::topics(statements)
                .iter()
                .map(|topic| DigestTopic {
                    heading: words::topic_heading(topic),
                    statements: topic
                        .statements
                        .iter()
                        .enumerate()
                        .map(|(at, statement)| DigestStatement {
                            index: (topic.start + at) as u32,
                            text: statement.text.clone(),
                            number: statement.reference.number,
                            message: statement.reference.message,
                        })
                        .collect(),
                })
                .collect(),
            focused_reference: window.reference.map(|at| at as u32),
            card,
            footer: window
                .summary
                .as_ref()
                .map(|summary| words::summary_footer(summary.messages)),
            email,
            back: match window.email {
                Some((_, Page::List)) => list_tab,
                _ => "Summary".to_owned(),
            },
            back_key: hints::key(keymap, CommandId::Back),
            loading: window.loading,
        })
    }

    pub(crate) fn new_rule(&mut self, rows: &dyn Rows) -> Vec<Step> {
        use postio_core::state::Selection;
        let selection = self.cursor.selection();
        let cursor = self.cursor.position().and_then(|at| rows.row(at));
        let resident: Vec<FocusRow> = match &selection {
            Selection::These(picked) => picked
                .iter()
                .filter_map(|id| rows.position_of(*id).and_then(|at| rows.row(at)))
                .collect(),
            Selection::Everything { .. } => Vec::new(),
        };
        let aimed = postio_ui::focus_target::aimed_rows(&selection, cursor.clone(), resident);
        let senders = postio_ui::focus_target::senders(&aimed);
        if senders.is_empty() {
            return Vec::new();
        }
        let like_this = postio_ui::focus_target::like_this_message(
            &selection,
            self.states
                .config
                .model_for(postio_config::model::ModelFeature::LikeThis)
                .is_some(),
            cursor.as_ref(),
        );
        let stamp = self.stamp();
        self.rule = Some(RuleDialog {
            form: RuleForm::new_rule(&senders, like_this),
            schedule: Schedule::new_rule(),
            preview: None,
            error: None,
            stamp,
        });
        self.open_rule()
    }

    fn edit_rule(&mut self, name: &str) -> Vec<Step> {
        let rule = self
            .states
            .config
            .digests
            .iter()
            .find(|rule| rule.name.trim() == name.trim())
            .cloned();
        let Some(rule) = rule else {
            return vec![Step::Show(Intent::Toast {
                text: postio_ui::focus_target::RULE_MISSING.to_owned(),
                kind: ToastKind::Notice,
            })];
        };
        let stamp = self.stamp();
        self.rule = Some(RuleDialog {
            form: RuleForm::edit(&rule),
            schedule: Schedule::of_rule(&rule),
            preview: None,
            error: None,
            stamp,
        });
        self.open_rule()
    }

    fn open_rule(&mut self) -> Vec<Step> {
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Dialog, self.policy.caps.stacking);
        if let Some(view) = self.rule_view() {
            steps.push(Step::Show(Intent::OpenRule(Box::new(view))));
        }
        steps.extend(self.read_preview());
        steps
    }

    fn read_preview(&mut self) -> Vec<Step> {
        let since = self.now().to_utc() - chrono::Duration::days(words::PREVIEW_DAYS);
        let stamp = self.stamp();
        let Some(rule) = self.rule.as_mut() else {
            return Vec::new();
        };
        rule.stamp = stamp;
        if rule.form.queries().is_empty() {
            rule.preview = None;
            return Vec::new();
        }
        vec![Step::Ask(Request::DigestPreview {
            queries: rule.form.queries().to_vec(),
            since,
            stamp,
        })]
    }

    pub(crate) fn rule_input(&mut self, input: crate::Input) -> Vec<Step> {
        use crate::Input;
        let Some(rule) = self.rule.as_mut() else {
            return Vec::new();
        };
        let mut steps = Vec::new();
        match input {
            Input::RuleQuery { text } => {
                if rule.form.query() == Some(text.as_str()) {
                    return Vec::new();
                }
                set_query(rule, &text);
                steps.extend(self.read_preview());
            }
            Input::RuleMatchInstead => {
                set_query(rule, "");
                steps.extend(self.read_preview());
            }
            Input::RuleLikeThis => {
                let Some(message) = rule.form.like_this() else {
                    return Vec::new();
                };
                let stamp = rule.stamp;
                return vec![Step::Ask(Request::DigestLikeThis { message, stamp })];
            }
            Input::RuleSchedule(schedule) => {
                rule.schedule = schedule;
                rule.error = None;
            }
            Input::RuleCreate => match DigestRuleDraft::from_form(&rule.form, &rule.schedule) {
                Ok(draft) => {
                    rule.error = None;
                    return vec![Step::Ask(Request::SaveDigestRule {
                        replacing: rule.form.replacing().map(str::to_owned),
                        draft,
                        stamp: rule.stamp,
                    })];
                }
                Err(sentence) => rule.error = Some(sentence),
            },
            _ => return Vec::new(),
        }
        let mut drawn = self.draw_rule();
        drawn.extend(steps);
        drawn
    }

    pub(crate) fn rule_reply(&mut self, reply: Reply) -> Vec<Step> {
        let stamp = match &reply {
            Reply::DigestPreview { stamp, .. }
            | Reply::LikeThis { stamp, .. }
            | Reply::RuleSaved { stamp, .. } => *stamp,
            _ => return Vec::new(),
        };
        let Some(rule) = self.rule.as_mut().filter(|rule| rule.stamp == stamp) else {
            return Vec::new();
        };
        match reply {
            Reply::DigestPreview { answer, .. } => match answer {
                Ok(preview) => {
                    rule.preview = Some(preview);
                    rule.error = None;
                }
                Err(error) => rule.error = Some(error),
            },
            Reply::LikeThis { answer, .. } => match answer {
                Ok(Some(queries)) => {
                    set_query(rule, &queries.join(", "));
                    let mut steps = self.draw_rule();
                    steps.extend(self.read_preview());
                    return steps;
                }
                Ok(None) => rule.error = Some(words::NOTHING_ALIKE.to_owned()),
                Err(error) => rule.error = Some(error),
            },
            Reply::RuleSaved { answer, .. } => match answer {
                Ok(()) => {
                    let name = rule.form.name().to_owned();
                    self.rule = None;
                    self.surfaces.dismiss(SurfaceKind::Dialog);
                    let mut steps = vec![
                        Step::Show(Intent::CloseSurface(SurfaceKind::Dialog)),
                        Step::Show(Intent::Toast {
                            text: postio_ui::focus_target::rule_saved(&name),
                            kind: ToastKind::Notice,
                        }),
                    ];
                    if !self.has_surface() {
                        steps.push(Step::Show(Intent::KeyboardHome));
                    }
                    return steps;
                }
                Err(error) => rule.error = Some(error),
            },
            _ => {}
        }
        self.draw_rule()
    }

    fn draw_rule(&self) -> Vec<Step> {
        self.rule_view()
            .map(|view| vec![Step::Show(Intent::Rule(Box::new(view)))])
            .unwrap_or_default()
    }

    pub(crate) fn rule_view(&self) -> Option<RuleView> {
        let rule = self.rule.as_ref()?;
        let now = self.now();
        let senders = !rule.form.query_mode();
        Some(RuleView {
            heading: rule.form.heading().to_owned(),
            from_label: words::FROM.to_owned(),
            from: rule.form.senders().map(str::to_owned),
            query: rule.form.query().map(str::to_owned),
            placeholder: words::QUERY_PLACEHOLDER.to_owned(),
            match_instead: senders.then(|| words::MATCH_INSTEAD.to_owned()),
            like_this: rule.form.like_this().map(|_| words::LIKE_THIS.to_owned()),
            deliver_label: words::DELIVER.to_owned(),
            schedule: rule.schedule.clone(),
            cadences: words::CADENCES
                .iter()
                .map(|(_, name)| (*name).to_owned())
                .collect(),
            weekdays: words::WEEKDAYS
                .iter()
                .map(|day| {
                    chrono::NaiveDate::from_isoywd_opt(2026, 1, *day)
                        .map(|date| date.format("%A").to_string())
                        .unwrap_or_default()
                })
                .collect(),
            note: rule.form.note().to_owned(),
            create: words::create_words(rule.form.editing()).to_owned(),
            create_key: hints::key(self.keymap(), CommandId::PickerConfirm),
            preview_heading: rule
                .preview
                .as_ref()
                .map(|preview| words::preview_heading(preview.count)),
            preview: rule
                .preview
                .as_ref()
                .map(|preview| {
                    preview
                        .first
                        .iter()
                        .map(|row| RulePreviewLine {
                            subject: row.subject.clone().unwrap_or_default(),
                            day: words::preview_day(
                                row.received_at.with_timezone(&chrono::Local),
                                now,
                            ),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            more: rule
                .preview
                .as_ref()
                .and_then(|preview| words::preview_more(preview.count, preview.first.len())),
            error: rule.error.clone(),
        })
    }
}

fn set_query(rule: &mut RuleDialog, text: &str) {
    rule.form.set_query(text);
    rule.error = None;
}
