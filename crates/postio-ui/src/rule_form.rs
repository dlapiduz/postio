//! The digest rule dialog's content and rules, shared by every frontend; controls, focus and
//! drawing stay per frontend.

use postio_config::{DigestRule, Due};
use postio_model::{EmailAddress, MessageId};

use crate::digest::{self, Schedule};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleForm {
    heading: String,
    name: String,
    queries: Vec<String>,
    replacing: Option<String>,
    like_this: Option<MessageId>,
    senders: String,
    query: Option<String>,
    note: &'static str,
}

pub type Written = (String, Vec<String>, Due);

impl RuleForm {
    /// `like_this` is the message "Digest mail like this" checks other mail against, when a model
    /// exists.
    pub fn new_rule(senders: &[EmailAddress], like_this: Option<MessageId>) -> Self {
        Self::new_rule_clean(senders, like_this, str::to_owned)
    }

    pub fn new_rule_clean(
        senders: &[EmailAddress],
        like_this: Option<MessageId>,
        clean: impl Fn(&str) -> String,
    ) -> Self {
        let names: Vec<String> = senders
            .iter()
            .map(|sender| clean(sender.display()))
            .collect();
        RuleForm {
            heading: digest::new_rule_heading(senders.len()).to_owned(),
            name: digest::rule_name(&names),
            queries: digest::sender_queries(senders),
            replacing: None,
            like_this,
            senders: clean(&digest::sender_line(senders)),
            query: None,
            note: digest::rule_note(senders.len()),
        }
    }

    /// A rule whose queries are all `from:` reads as senders. "Digest mail like this" is never
    /// offered while editing.
    pub fn edit(rule: &DigestRule) -> Self {
        Self::edit_clean(rule, str::to_owned)
    }

    pub fn edit_clean(rule: &DigestRule, clean: impl Fn(&str) -> String) -> Self {
        let text = rule.queries.join(", ");
        let senders = digest::is_sender_rule(&rule.queries);
        RuleForm {
            heading: clean(&digest::edit_rule_heading(&rule.name)),
            name: rule.name.clone(),
            queries: rule.queries.clone(),
            replacing: Some(rule.name.clone()),
            like_this: None,
            senders: clean(&text),
            query: (!senders).then_some(text),
            note: digest::rule_note(rule.queries.len()),
        }
    }

    pub fn heading(&self) -> &str {
        &self.heading
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn queries(&self) -> &[String] {
        &self.queries
    }

    pub fn replacing(&self) -> Option<&str> {
        self.replacing.as_deref()
    }

    pub fn editing(&self) -> bool {
        self.replacing.is_some()
    }

    pub fn note(&self) -> &'static str {
        self.note
    }

    pub fn senders(&self) -> Option<&str> {
        self.query.is_none().then_some(self.senders.as_str())
    }

    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    pub fn query_mode(&self) -> bool {
        self.query.is_some()
    }

    pub fn like_this(&self) -> Option<MessageId> {
        self.like_this.filter(|_| !self.query_mode())
    }

    /// One query per comma-separated piece; also the rule's name until it is given one.
    pub fn set_query(&mut self, text: &str) {
        let queries = digest::split_queries(text);
        self.name = queries.join(", ");
        self.queries = queries;
        self.query = Some(text.to_owned());
    }

    pub fn written(&self, schedule: &Schedule) -> Result<Written, String> {
        Ok((self.name.clone(), self.queries.clone(), schedule.due()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender(address: &str, name: Option<&str>) -> EmailAddress {
        EmailAddress {
            address: address.to_owned(),
            name: name.map(str::to_owned),
        }
    }

    fn rule(name: &str, queries: &[&str]) -> DigestRule {
        DigestRule {
            name: name.to_owned(),
            queries: queries.iter().map(|query| (*query).to_owned()).collect(),
            ..DigestRule::default()
        }
    }

    #[test]
    fn a_new_rule_from_one_sender_is_named_and_matched_by_them() {
        let form = RuleForm::new_rule(&[sender("News@Example.com", Some("The News"))], None);
        assert_eq!(form.heading(), "Digest this sender");
        assert_eq!(form.name(), "The News");
        assert_eq!(form.queries(), ["from:news@example.com"]);
        assert_eq!(form.senders(), Some("news@example.com"));
        assert_eq!(form.query(), None);
        assert!(!form.editing() && !form.query_mode());
    }

    #[test]
    fn several_senders_are_one_rule_with_a_query_each() {
        let form = RuleForm::new_rule(
            &[sender("a@example.com", None), sender("b@example.com", None)],
            None,
        );
        assert_eq!(form.heading(), "Digest these senders");
        assert_eq!(form.queries(), ["from:a@example.com", "from:b@example.com"]);
        assert_eq!(form.senders(), Some("a@example.com, b@example.com"));
    }

    #[test]
    fn what_a_frontend_cleans_is_cleaned_in_the_name_and_the_line() {
        let form = RuleForm::new_rule_clean(
            &[sender("a@example.com", Some("A\u{1b}da"))],
            None,
            |text| text.replace('\u{1b}', ""),
        );
        assert_eq!(form.name(), "Ada");
        assert_eq!(form.senders(), Some("a@example.com"));
    }

    #[test]
    fn like_this_is_offered_only_while_the_senders_show() {
        let message = MessageId::new(7);
        let mut form = RuleForm::new_rule(&[sender("a@example.com", None)], Some(message));
        assert_eq!(form.like_this(), Some(message));
        form.set_query("");
        assert_eq!(form.like_this(), None);
        assert_eq!(
            RuleForm::edit(&rule("N", &["from:a@example.com"])).like_this(),
            None
        );
    }

    #[test]
    fn typing_a_query_renames_the_rule_and_swaps_the_senders_out() {
        let mut form = RuleForm::new_rule(&[sender("a@example.com", None)], None);
        form.set_query("list:weekly.example.org, is:unread ,");
        assert_eq!(form.queries(), ["list:weekly.example.org", "is:unread"]);
        assert_eq!(form.name(), "list:weekly.example.org, is:unread");
        assert_eq!(form.senders(), None);
        assert_eq!(form.query(), Some("list:weekly.example.org, is:unread ,"));
        assert!(form.query_mode());
    }

    #[test]
    fn matching_instead_starts_from_nothing() {
        let mut form = RuleForm::new_rule(&[sender("a@example.com", None)], None);
        form.set_query("");
        assert!(form.queries().is_empty());
        assert_eq!(form.query(), Some(""));
    }

    #[test]
    fn editing_a_sender_rule_shows_its_senders_under_its_own_name() {
        let form = RuleForm::edit(&rule("Newsletters", &["from:a@example.com"]));
        assert_eq!(form.heading(), "Digest rule \u{b7} Newsletters");
        assert_eq!(form.replacing(), Some("Newsletters"));
        assert_eq!(form.name(), "Newsletters");
        assert_eq!(form.senders(), Some("from:a@example.com"));
        assert!(form.editing() && !form.query_mode());
    }

    #[test]
    fn editing_any_other_rule_opens_in_the_query_entry() {
        let form = RuleForm::edit(&rule("Lists", &["list:a.example.org", "is:unread"]));
        assert!(form.query_mode());
        assert_eq!(form.query(), Some("list:a.example.org, is:unread"));
        assert_eq!(form.senders(), None);
        assert_eq!(form.name(), "Lists");
    }

    #[test]
    fn written_pairs_the_rule_with_when_the_controls_say_it_is_due() {
        let form = RuleForm::new_rule(&[sender("a@example.com", None)], None);
        let (name, queries, due) = form.written(&Schedule::new_rule()).expect("a schedule");
        assert_eq!(name, form.name());
        assert_eq!(queries, form.queries());
        assert_eq!(Some(due), Schedule::new_rule().due().ok());
    }

    #[test]
    fn a_time_that_does_not_read_is_the_reason_it_cannot_be_written() {
        let form = RuleForm::new_rule(&[sender("a@example.com", None)], None);
        let mut schedule = Schedule::new_rule();
        schedule.at = "nine".to_owned();
        assert_eq!(form.written(&schedule), Err(digest::TIME_ERROR.to_owned()));
    }

    #[test]
    fn editing_a_rule_with_no_readable_schedule_starts_from_a_new_rules() {
        let broken = rule("Broken", &["from:a@example.com"]);
        assert_eq!(Schedule::of_rule(&broken), Schedule::new_rule());
        let daily = DigestRule {
            cadence: "daily".to_owned(),
            at: "08:15".to_owned(),
            ..broken
        };
        let shown = Schedule::of_rule(&daily);
        assert_eq!((shown.cadence, shown.at.as_str()), (0, "08:15"));
        assert_eq!(shown.cadence_kind(), postio_model::listing::Cadence::Daily);
    }
}
