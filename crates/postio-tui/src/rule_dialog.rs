//! The rule dialog's state (terminal.md, "Digests"; screen 24): "Digest this
//! sender" from `d`, or a rule edited from its digest or from the rules list.
//!
//! The senders, how often and when, what the rule would have caught in the
//! last 90 days, and Create. "Match a list or a search instead…" swaps the
//! senders for typed queries, previewed the same way. The words and the
//! rules of the schedule are `postio_ui::digest`'s; the drawing is
//! `view::rule_dialog`.

use postio_client::protocol::{DigestPreview, DigestRuleDraft, RuleDay};
use postio_config::{DigestRule, Due};
use postio_model::listing::Cadence;
use postio_model::{EmailAddress, MessageId};
use postio_ui::digest::{self, Schedule};
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
    replacing: Option<String>,
    heading: String,
    name: String,
    queries: Vec<String>,
    senders: String,
    query_mode: bool,
    query: Input,
    schedule: Schedule,
    time: Input,
    like_this: Option<MessageId>,
    note: &'static str,
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
        let names: Vec<String> = senders
            .iter()
            .map(|sender| SafeText::new(sender.display()).as_str().to_owned())
            .collect();
        let schedule = Schedule::new_rule();
        Form {
            replacing: None,
            heading: digest::new_rule_heading(senders.len()).to_owned(),
            name: digest::rule_name(&names),
            queries: digest::sender_queries(senders),
            senders: SafeText::new(&digest::sender_line(senders))
                .as_str()
                .to_owned(),
            query_mode: false,
            query: Input::default(),
            time: Input::new(schedule.at.clone()),
            schedule,
            like_this,
            note: digest::rule_note(senders.len()),
            preview: None,
            generation: 0,
            error: None,
            field: Field::Cadence,
        }
    }

    /// `rule` edited where it stands in `config.toml`.
    pub fn edit(rule: &DigestRule) -> Form {
        let schedule = rule
            .due()
            .map(|due| Schedule::of(&due))
            .unwrap_or_else(|_| Schedule::new_rule());
        let senders = digest::is_sender_rule(&rule.queries);
        let text = rule.queries.join(", ");
        let mut form = Form {
            replacing: Some(rule.name.clone()),
            heading: SafeText::new(&digest::edit_rule_heading(&rule.name))
                .as_str()
                .to_owned(),
            name: rule.name.clone(),
            queries: rule.queries.clone(),
            senders: SafeText::new(&text).as_str().to_owned(),
            query_mode: !senders,
            query: Input::new(text),
            time: Input::new(schedule.at.clone()),
            schedule,
            like_this: None,
            note: digest::rule_note(rule.queries.len()),
            preview: None,
            generation: 0,
            error: None,
            field: Field::Cadence,
        };
        if form.query_mode {
            form.field = Field::Query;
        }
        form
    }

    /// The heading: "Digest this sender", or the rule's name.
    pub fn heading(&self) -> &str {
        &self.heading
    }

    /// Whether a rule is being edited rather than made.
    pub fn editing(&self) -> bool {
        self.replacing.is_some()
    }

    /// The senders, as one line.
    pub fn senders(&self) -> &str {
        &self.senders
    }

    /// Whether the rule is typed queries rather than senders.
    pub fn query_mode(&self) -> bool {
        self.query_mode
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
        self.note
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
        self.like_this.filter(|_| !self.query_mode)
    }

    /// The controls, in the order Tab walks them.
    pub fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Cadence];
        if CadenceKind::of(&self.schedule) != CadenceKind::Daily {
            fields.push(Field::Day);
        }
        fields.push(Field::Time);
        if self.query_mode {
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
            Field::Day if CadenceKind::of(&self.schedule) == CadenceKind::Weekly => {
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
        self.query_mode = true;
        self.query = Input::new(text.to_owned());
        self.field = Field::Query;
        self.apply_query();
    }

    /// What was typed into the query field: one query per comma-separated
    /// piece is the rule's `match`, and its name until it is given one.
    pub fn apply_query(&mut self) {
        let queries = digest::split_queries(self.query.value());
        self.name = queries.join(", ");
        self.queries = queries;
    }

    /// The preview to ask for: the rule's queries over the last 90 days.
    pub fn preview_ask(&mut self) -> crate::ask::Ask {
        self.generation += 1;
        crate::ask::Ask::Preview {
            generation: self.generation,
            queries: self.queries.clone(),
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
        let (cadence, day, at) = match schedule.due()? {
            Due::Daily { at } => (Cadence::Daily, None, at),
            Due::Weekly { day, at } => (Cadence::Weekly, Some(RuleDay::Weekday(day)), at),
            Due::Monthly { day, at } => (Cadence::Monthly, Some(RuleDay::OfMonth(day)), at),
        };
        Ok(DigestRuleDraft {
            name: self.name.clone(),
            queries: self.queries.clone(),
            cadence,
            day,
            at,
        })
    }

    /// The rule being edited, by the name it had.
    pub fn replacing(&self) -> Option<String> {
        self.replacing.clone()
    }

    /// The name the rule will have.
    pub fn name(&self) -> &str {
        &self.name
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

/// Which of the three cadences the schedule says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CadenceKind {
    Daily,
    Weekly,
    Monthly,
}

impl CadenceKind {
    fn of(schedule: &Schedule) -> CadenceKind {
        match digest::CADENCES[schedule.cadence.min(2)].0 {
            Cadence::Daily => CadenceKind::Daily,
            Cadence::Weekly => CadenceKind::Weekly,
            Cadence::Monthly => CadenceKind::Monthly,
        }
    }
}
