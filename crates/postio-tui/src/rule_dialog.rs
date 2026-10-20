//! The rule dialog's state (terminal.md, "Digests"; screen 24): "Digest this
//! sender" from `d`, or a rule edited from its digest or from the rules list.
//!
//! The senders, how often and when, what the rule would have caught in the
//! last 90 days, and Create. "Match a list or a search instead…" swaps the
//! senders for typed queries, previewed the same way. The words and the
//! rules of the schedule are `postio_ui::digest`'s; the drawing is
//! `view::rule_dialog`.

use postio_client::protocol::{DigestPreview, DigestRuleDraft};
use postio_config::DigestRule;
use postio_model::listing::Cadence;
use postio_model::{EmailAddress, MessageId};
use postio_ui::digest::{self, Schedule};
use postio_ui::rule_form::RuleForm;
use postio_ui::terminal::SafeText;
use tui_input::Input;

/// One control of the dialog the keyboard can be on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// How often.
    Cadence,
    /// The weekday or the day of the month.
    Day,
    /// The time of day.
    Time,
    /// The typed queries, once "match instead" has been chosen.
    Query,
    /// "Match a list or a search instead…".
    MatchInstead,
    /// "Digest mail like this".
    LikeThis,
    /// Create, or Save.
    Create,
}

/// What a rule would have caught, as the dialog shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// How many messages it matches in the window.
    pub count: u32,
    /// The newest few: subject and when.
    pub first: Vec<(SafeText, chrono::DateTime<chrono::Utc>)>,
}

/// The form. See the module.
#[derive(Debug, Clone)]
pub struct Form {
    rule: RuleForm,
    query: Input,
    schedule: Schedule,
    time: Input,
    preview: Option<Preview>,
    generation: u64,
    error: Option<String>,
    field: Field,
}

impl Form {
    /// A new rule holding mail from `senders`. `like_this` is the message
    /// "Digest mail like this" would check other mail against, when the
    /// person has a model for it.
    pub fn new_rule(senders: &[EmailAddress], like_this: Option<MessageId>) -> Form {
        let schedule = Schedule::new_rule();
        Form {
            rule: RuleForm::new_rule_clean(senders, like_this, clean),
            query: Input::default(),
            time: Input::new(schedule.at.clone()),
            schedule,
            preview: None,
            generation: 0,
            error: None,
            field: Field::Cadence,
        }
    }

    /// `rule` edited where it stands in `config.toml`.
    pub fn edit(rule: &DigestRule) -> Form {
        let schedule = Schedule::of_rule(rule);
        let rule = RuleForm::edit_clean(rule, clean);
        let mut form = Form {
            query: Input::new(rule.query().unwrap_or_default().to_owned()),
            rule,
            time: Input::new(schedule.at.clone()),
            schedule,
            preview: None,
            generation: 0,
            error: None,
            field: Field::Cadence,
        };
        if form.query_mode() {
            form.field = Field::Query;
        }
        form
    }

    /// The heading: "Digest this sender", or the rule's name.
    pub fn heading(&self) -> &str {
        self.rule.heading()
    }

    /// Whether a rule is being edited rather than made.
    pub fn editing(&self) -> bool {
        self.rule.editing()
    }

    /// The senders, as one line.
    pub fn senders(&self) -> &str {
        self.rule.senders().unwrap_or_default()
    }

    /// Whether the rule is typed queries rather than senders.
    pub fn query_mode(&self) -> bool {
        self.rule.query_mode()
    }

    /// The typed queries.
    pub fn query(&self) -> &Input {
        &self.query
    }

    /// The time of day as typed.
    pub fn time(&self) -> &Input {
        &self.time
    }

    /// The cadence, weekday and day of the month chosen.
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// The note under the preview.
    pub fn note(&self) -> &'static str {
        self.rule.note()
    }

    /// What the rule would have caught, once read.
    pub fn preview(&self) -> Option<&Preview> {
        self.preview.as_ref()
    }

    /// Why the rule cannot be written, or what went wrong.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Say why not.
    pub fn say(&mut self, sentence: Option<String>) {
        self.error = sentence;
    }

    /// The control the keyboard is on.
    pub fn field(&self) -> Field {
        self.field
    }

    /// The message "Digest mail like this" would check, when it is offered.
    pub fn like_this(&self) -> Option<MessageId> {
        self.rule.like_this()
    }

    /// The controls, in the order Tab walks them.
    pub fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Cadence];
        if self.schedule.cadence_kind() != Cadence::Daily {
            fields.push(Field::Day);
        }
        fields.push(Field::Time);
        if self.query_mode() {
            fields.push(Field::Query);
        } else {
            fields.push(Field::MatchInstead);
            if self.like_this().is_some() {
                fields.push(Field::LikeThis);
            }
        }
        fields.push(Field::Create);
        fields
    }

    /// Whether the keyboard is in a field that takes typing.
    pub fn typing(&self) -> bool {
        matches!(self.field, Field::Time | Field::Query)
    }

    /// Move the keyboard to the next control, or the previous.
    pub fn walk(&mut self, by: isize) {
        let fields = self.fields();
        let at = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        let next = (at as isize + by).rem_euclid(fields.len() as isize) as usize;
        self.field = fields[next];
    }

    /// Put the keyboard on `field` when the dialog has it.
    pub fn focus(&mut self, field: Field) {
        if self.fields().contains(&field) {
            self.field = field;
        }
    }

    /// Change the cadence or the day by `by`, wrapping.
    pub fn change(&mut self, by: isize) {
        match self.field {
            Field::Cadence => {
                let at = self.schedule.cadence as isize + by;
                self.schedule.cadence = at.rem_euclid(digest::CADENCES.len() as isize) as usize;
            }
            Field::Day if self.schedule.cadence_kind() == Cadence::Weekly => {
                let at = self.schedule.weekday as isize + by;
                self.schedule.weekday = at.rem_euclid(digest::WEEKDAYS.len() as isize) as usize;
            }
            Field::Day => {
                let at = self.schedule.month_day as isize + by;
                self.schedule.month_day = at.rem_euclid(28) as usize;
            }
            _ => {}
        }
    }

    /// "Match a list or a search instead…": the senders give way to typed
    /// queries, starting from `text`. Answers whether the preview is to be
    /// read again.
    pub fn match_instead(&mut self, text: &str) {
        self.query = Input::new(text.to_owned());
        self.field = Field::Query;
        self.apply_query();
    }

    /// What was typed into the query field: one query per comma-separated
    /// piece is the rule's `match`, and its name until it is given one.
    pub fn apply_query(&mut self) {
        self.rule.set_query(self.query.value());
    }

    /// The preview to ask for: the rule's queries over the last 90 days.
    pub fn preview_ask(&mut self) -> crate::ask::Ask {
        self.generation += 1;
        crate::ask::Ask::Preview {
            generation: self.generation,
            queries: self.rule.queries().to_vec(),
            since: chrono::Utc::now() - chrono::Duration::days(digest::PREVIEW_DAYS),
        }
    }

    /// The preview arrived. Answers whether it was for this reading.
    pub fn previewed(&mut self, generation: u64, preview: DigestPreview) -> bool {
        if generation != self.generation {
            return false;
        }
        self.preview = Some(Preview {
            count: preview.count,
            first: preview
                .first
                .into_iter()
                .map(|row| {
                    (
                        SafeText::new(row.subject.as_deref().unwrap_or("")),
                        row.received_at,
                    )
                })
                .collect(),
        });
        true
    }

    /// The rule as the dialog says it, or why it cannot be one.
    pub fn draft(&self) -> Result<DigestRuleDraft, String> {
        let mut schedule = self.schedule.clone();
        schedule.at = self.time.value().to_owned();
        DigestRuleDraft::from_form(&self.rule, &schedule)
    }

    /// The rule being edited, by the name it had.
    pub fn replacing(&self) -> Option<String> {
        self.rule.replacing().map(str::to_owned)
    }

    /// The name the rule will have.
    pub fn name(&self) -> &str {
        self.rule.name()
    }

    /// The text inputs, for typing.
    pub fn input(&mut self) -> Option<&mut Input> {
        match self.field {
            Field::Time => Some(&mut self.time),
            Field::Query => Some(&mut self.query),
            _ => None,
        }
    }
}

/// What the terminal shows of text that came from outside: no controls.
fn clean(text: &str) -> String {
    SafeText::new(text).as_str().to_owned()
}
