//! The composer: one draft being written, in Markdown (US3).
//!
//! Modeless, like every text box the person has used before (FR-022a): a
//! key that is not a composer command is typed. Commands still resolve --
//! the keymap is asked with `in_text_entry`, which lets a chord with a
//! modifier through and hands a plain key back -- so `a` is a letter here,
//! never Archive.
//!
//! The body is kept as the Markdown typed. What is sent is derived from it
//! on every save (data-model.md, Composer): the text part is that Markdown
//! (FR-021), the HTML part is its rendering, and a body with nothing to
//! style sends no HTML at all.

use crossterm::event::{KeyCode, KeyEvent};
use postio_body::{Block, Document, Inline, Presentation, markdown};
use postio_model::contact_group::RecipientCandidate;
use postio_model::{Draft, MessageBody};
use postio_ui::terminal::SafeText;
use ratatui_textarea::TextArea;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

/// Which part of the composer the keyboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// Which of the account's addresses it goes from, when it has several.
    From,
    /// `To`.
    To,
    /// `Cc`, once shown.
    Cc,
    /// `Bcc`, once shown.
    Bcc,
    /// The subject.
    Subject,
    /// The body.
    Body,
}

/// One draft being written.
pub struct Composer {
    /// Which composition this is, for the host's draft writer: a save made
    /// before the previous one's id came back still updates the same row.
    generation: u64,
    /// The draft as it was opened; what the fields do not hold comes from
    /// here (account, kind, what it answers, its id once saved).
    draft: Draft,
    to: Input,
    cc: Input,
    bcc: Input,
    subject: Input,
    body: TextArea<'static>,
    field: Field,
    /// Whether `Cc` and `Bcc` are shown. Shown from the start when the draft
    /// already has some, so reopening one never hides a recipient.
    extra_recipients: bool,
    /// A reply's quote or a forward's message, sent after what is typed.
    quote: Vec<Block>,
    /// The addresses the account can send as.
    identities: Vec<postio_model::Identity>,
    /// Which of them this goes from.
    identity: usize,
    /// How many times what would be saved has changed, so a save that was
    /// asked for before the last edit knows it is stale.
    edits: u64,
    /// Who the recipient being typed could be, best first.
    suggestions: Vec<RecipientCandidate>,
    /// The suggestion the keyboard is on.
    suggestion: usize,
    /// What was last looked up, so an answer for anything else is dropped.
    asking: Option<String>,
}

impl std::fmt::Debug for Composer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Composer")
            .field("generation", &self.generation)
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl Composer {
    /// A composer holding `draft`, as composition `generation`.
    ///
    /// The body reopens from the Markdown it was typed in when there is some,
    /// and otherwise from its HTML, translated (data-model.md,
    /// `drafts.body_markdown`), and otherwise from its text.
    pub fn new(generation: u64, draft: Draft) -> Self {
        // A reply's quote -- and a forward's message -- is kept whole and
        // apart from what is typed (data-model.md, Composer): it is somebody
        // else's HTML, carried as the desktop carries it (ADR 0033), and
        // Markdown would narrow it. Everything from the first quote on is it.
        let document = draft.body.html.as_deref().map(postio_body::parse);
        let (head, quote) = match document {
            Some(document) => {
                let split = document
                    .blocks
                    .iter()
                    .position(|block| matches!(block, Block::Quoted(_)))
                    .unwrap_or(document.blocks.len());
                let mut blocks = document.blocks;
                let quote = blocks.split_off(split);
                (Some(Document { blocks }), quote)
            }
            None => (None, Vec::new()),
        };
        let markdown = match (&draft.body_markdown, head, &draft.body.text) {
            (Some(markdown), _, _) => markdown.clone(),
            (None, Some(head), _) => markdown_of(head),
            (None, None, Some(text)) => text.clone(),
            (None, None, None) => String::new(),
        };
        // Through `SafeText`, every one: a reply's subject and quoted
        // addresses are copied from received mail, and whatever a composer
        // holds is drawn (FR-043). A control character has no business in a
        // draft, so it is not kept to be sent either.
        let safe = |text: &str| SafeText::new(text).as_str().to_owned();
        let lines: Vec<String> = markdown.split('\n').map(safe).collect();
        let line = |addresses: &[postio_model::EmailAddress]| {
            Input::default().with_value(safe(&postio_model::address::format_list(addresses)))
        };
        let extra_recipients = !draft.cc.is_empty() || !draft.bcc.is_empty();
        // Where a person picks up: an address first when there is none, the
        // body when the message is already addressed -- a reply is.
        let field = if draft.to.is_empty() {
            Field::To
        } else {
            Field::Body
        };
        Composer {
            generation,
            to: line(&draft.to),
            cc: line(&draft.cc),
            bcc: line(&draft.bcc),
            subject: Input::default().with_value(safe(&draft.subject)),
            body: {
                let mut body = TextArea::new(lines);
                // No underline under the cursor's line: a message body is not
                // a code editor.
                body.set_cursor_line_style(ratatui::style::Style::default());
                body
            },
            field,
            extra_recipients,
            quote,
            identities: Vec::new(),
            identity: 0,
            edits: 0,
            suggestions: Vec::new(),
            suggestion: 0,
            asking: None,
            draft,
        }
    }

    /// The same composer, sending as one of `identities`: the one the draft
    /// names (a reply answers as the address it was sent to), else the
    /// account's default. With more than one, a new message starts by
    /// showing which.
    pub fn with_identities(mut self, identities: Vec<postio_model::Identity>) -> Self {
        self.identity = self
            .draft
            .identity_id
            .and_then(|wanted| identities.iter().position(|identity| identity.id == wanted))
            .or_else(|| identities.iter().position(|identity| identity.is_default))
            .unwrap_or(0);
        self.identities = identities;
        if self.shows_identities() && self.field == Field::To {
            self.field = Field::From;
        }
        self
    }

    /// Whether there is a choice of address to send as.
    pub fn shows_identities(&self) -> bool {
        self.identities.len() > 1
    }

    /// How many lines the kept quote is, for its folded summary; none when
    /// there is no quote.
    pub fn quote_lines(&self) -> usize {
        quote_text(&self.quote).lines().count()
    }

    /// Which composition this is.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Where the keyboard is.
    pub fn field(&self) -> Field {
        self.field
    }

    /// Whether `Cc` and `Bcc` are shown.
    pub fn shows_extra_recipients(&self) -> bool {
        self.extra_recipients
    }

    /// Show `Cc` and `Bcc`, and put the keyboard in `Cc`.
    pub fn show_extra_recipients(&mut self) {
        self.extra_recipients = true;
        self.field = Field::Cc;
    }

    /// Type `key` into the field the keyboard is in. `Tab` and `Shift+Tab`
    /// move between fields. Answers whether anything changed.
    pub fn type_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Tab => {
                self.field = self.step(1);
                return true;
            }
            KeyCode::BackTab => {
                self.field = self.step(-1);
                return true;
            }
            // Enter in a one-line field moves on, as it does in a form; in
            // the body it is a new line.
            KeyCode::Enter if self.field != Field::Body && key.modifiers.is_empty() => {
                self.field = self.step(1);
                return true;
            }
            // In From there is nothing to type: the arrows and space choose.
            KeyCode::Left | KeyCode::Up if self.field == Field::From => {
                self.identity = self
                    .identity
                    .checked_sub(1)
                    .unwrap_or(self.identities.len().saturating_sub(1));
                self.edits += 1;
                return true;
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Char(' ') if self.field == Field::From => {
                self.identity = (self.identity + 1) % self.identities.len().max(1);
                self.edits += 1;
                return true;
            }
            _ => {}
        }
        let (changed, moved) = match self.field {
            Field::From => (false, false),
            Field::Body => {
                let before = self.body.cursor();
                let changed = self.body.input(key);
                (changed, self.body.cursor() != before)
            }
            field => {
                let event = crossterm::event::Event::Key(key);
                self.line_mut(field)
                    .and_then(|input| input.handle_event(&event))
                    .map_or((false, false), |state| (state.value, state.cursor))
            }
        };
        if changed {
            self.edits += 1;
        }
        changed || moved
    }

    /// How many times what would be saved has changed.
    pub fn edits(&self) -> u64 {
        self.edits
    }

    /// Who the recipient being typed could be, best first.
    pub fn suggestions(&self) -> &[RecipientCandidate] {
        &self.suggestions
    }

    /// The suggestion the keyboard is on.
    pub fn suggestion(&self) -> usize {
        self.suggestion
    }

    /// What to look up now, if the recipient being typed has changed and
    /// there is enough of it (the desktop's threshold, #424). Anything
    /// shorter puts the suggestions away.
    pub fn wants_completion(&mut self) -> Option<String> {
        let wanted = match self.field {
            Field::To | Field::Cc | Field::Bcc => {
                postio_ui::recipients::prefix(self.value(self.field)).map(str::to_owned)
            }
            _ => None,
        };
        if wanted.is_none() {
            self.asking = None;
            self.suggestions.clear();
            return None;
        }
        if wanted == self.asking {
            return None;
        }
        self.asking.clone_from(&wanted);
        wanted
    }

    /// The answer to a lookup, offered only if it is still what is typed.
    pub fn offer(&mut self, prefix: &str, found: Vec<RecipientCandidate>) {
        if self.asking.as_deref() == Some(prefix) {
            self.suggestions = found;
            self.suggestion = 0;
        }
    }

    /// A key while suggestions are showing: the arrows choose, Tab or Enter
    /// accepts, Escape puts them away. Answers whether it was one of those.
    pub fn completion_key(&mut self, key: KeyEvent) -> bool {
        if self.suggestions.is_empty() {
            return false;
        }
        let count = self.suggestions.len();
        match key.code {
            KeyCode::Down => self.suggestion = (self.suggestion + 1) % count,
            KeyCode::Up => self.suggestion = (self.suggestion + count - 1) % count,
            KeyCode::Tab | KeyCode::Enter => {
                let candidate = self.suggestions[self.suggestion].clone();
                let field = self.field;
                let replaced = postio_ui::recipients::accepted(self.value(field), &candidate);
                if let Some(input) = self.line_mut(field) {
                    *input = Input::default().with_value(replaced);
                }
                self.suggestions.clear();
                self.asking = None;
                self.edits += 1;
            }
            KeyCode::Esc => self.suggestions.clear(),
            _ => return false,
        }
        true
    }

    /// The id the host gave this draft's first save.
    pub fn adopt_id(&mut self, id: postio_model::DraftId) {
        self.draft.id = id;
    }

    /// The field `by` steps from the one the keyboard is in, over the
    /// fields that are shown, wrapping.
    fn step(&self, by: isize) -> Field {
        let mut shown = Vec::with_capacity(6);
        if self.shows_identities() {
            shown.push(Field::From);
        }
        shown.push(Field::To);
        if self.extra_recipients {
            shown.extend([Field::Cc, Field::Bcc]);
        }
        shown.extend([Field::Subject, Field::Body]);
        let at = shown
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0) as isize;
        shown[(at + by).rem_euclid(shown.len() as isize) as usize]
    }

    fn line_mut(&mut self, field: Field) -> Option<&mut Input> {
        match field {
            Field::To => Some(&mut self.to),
            Field::Cc => Some(&mut self.cc),
            Field::Bcc => Some(&mut self.bcc),
            Field::Subject => Some(&mut self.subject),
            Field::From | Field::Body => None,
        }
    }

    /// Put `text` where the cursor is: a paste of words.
    pub fn insert(&mut self, text: &str) {
        // A paste is whatever the clipboard held, from anywhere: made safe
        // as mail text is, after folding a Windows line end into a newline.
        let text = SafeText::new(&text.replace("\r\n", "\n"));
        let text = text.as_str();
        match self.field {
            Field::Body => {
                self.body.insert_str(text);
            }
            field => {
                // A one-line field keeps one line: a pasted list of
                // addresses arrives with newlines between them.
                let flat = text.split(['\r', '\n']).filter(|part| !part.is_empty());
                let flat = flat.collect::<Vec<_>>().join(", ");
                if let Some(input) = self.line_mut(field) {
                    for c in flat.chars() {
                        input.handle(tui_input::InputRequest::InsertChar(c));
                    }
                }
            }
        }
        self.edits += 1;
    }

    /// The Markdown in the body, as typed.
    pub fn markdown(&self) -> String {
        self.body.lines().join("\n")
    }

    /// One field's text, for drawing.
    pub fn value(&self, field: Field) -> &str {
        match field {
            Field::To => self.to.value(),
            Field::Cc => self.cc.value(),
            Field::Bcc => self.bcc.value(),
            Field::Subject => self.subject.value(),
            Field::From => self
                .identities
                .get(self.identity)
                .map_or("", |identity| identity.address.address.as_str()),
            Field::Body => "",
        }
    }

    /// Where the cursor is in a one-line field, in columns.
    pub fn cursor_in(&self, field: Field) -> usize {
        match field {
            Field::To => self.to.visual_cursor(),
            Field::Cc => self.cc.visual_cursor(),
            Field::Bcc => self.bcc.visual_cursor(),
            Field::Subject => self.subject.visual_cursor(),
            Field::From | Field::Body => 0,
        }
    }

    /// The body's editor, for drawing.
    pub fn body(&self) -> &TextArea<'static> {
        &self.body
    }

    /// The draft as it stands, with what is sent derived from the Markdown.
    pub fn draft(&self) -> Draft {
        let mut draft = self.draft.clone();
        draft.to = postio_model::address::parse_list(self.to.value());
        draft.cc = postio_model::address::parse_list(self.cc.value());
        draft.bcc = postio_model::address::parse_list(self.bcc.value());
        draft.subject = self.subject.value().to_owned();
        if let Some(identity) = self.identities.get(self.identity) {
            draft.identity_id = Some(identity.id);
        }
        let markdown = self.markdown();
        draft.body = body_of(&markdown, &self.quote);
        draft.body_markdown = Some(markdown);
        draft
    }
}

/// What a body of Markdown, followed by `quote`, sends: the Markdown and
/// then the quote's text as the text part (FR-021), and the rendering of
/// both as the HTML part when there is anything to style.
pub fn body_of(markdown: &str, quote: &[Block]) -> MessageBody {
    if markdown.trim().is_empty() && quote.is_empty() {
        return MessageBody::default();
    }
    let mut document = markdown::to_document(markdown);
    document.blocks.extend(quote.iter().cloned());
    let quoted = quote_text(quote);
    let text = match (markdown.trim().is_empty(), quoted.is_empty()) {
        (_, true) => markdown.to_owned(),
        (true, false) => quoted,
        (false, false) => format!("{}\n\n{quoted}", markdown.trim_end()),
    };
    MessageBody {
        text: Some(text),
        html: (!document.is_plain_text()).then(|| postio_body::render(&document).1),
    }
}

/// The text half of a kept quote: a reply's with `> ` before every line, as
/// every mail client expects; a forward's as it is, since it is not a
/// quotation but the message itself.
fn quote_text(quote: &[Block]) -> String {
    let mut out = Vec::new();
    for block in quote {
        match block {
            Block::Quoted(quoted) if quoted.presentation() == Presentation::Quote => {
                out.push(
                    quoted
                        .text()
                        .lines()
                        .map(|line| {
                            if line.is_empty() {
                                ">".to_owned()
                            } else {
                                format!("> {line}")
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            Block::Quoted(quoted) => out.push(quoted.text().to_owned()),
            other => out.push(
                Document {
                    blocks: vec![other.clone()],
                }
                .to_text(),
            ),
        }
    }
    out.join("\n\n")
}

/// The Markdown for what comes before a quote.
///
/// A reply opens with an empty paragraph for the caret above the
/// attribution; here that is an empty first line, so what is typed goes
/// above the attribution rather than into it.
fn markdown_of(head: Document) -> String {
    let caret = matches!(
        head.blocks.first(),
        Some(Block::Paragraph(inlines)) if inlines.as_slice() == [Inline::Break]
    );
    let rest = Document {
        blocks: head.blocks.into_iter().skip(usize::from(caret)).collect(),
    };
    // Trimmed: a document's Markdown ends its last block with a line break,
    // which the editor would show as an empty last line.
    let markdown = markdown::from_document(&rest);
    let markdown = markdown.trim_end_matches('\n');
    if caret {
        format!("\n\n{markdown}")
    } else {
        markdown.to_owned()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use postio_model::{AccountId, EmailAddress};

    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn typed(composer: &mut Composer, text: &str) {
        for c in text.chars() {
            composer.type_key(key(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn tab(composer: &mut Composer) {
        composer.type_key(key(KeyCode::Tab, KeyModifiers::NONE));
    }

    fn fresh() -> Composer {
        Composer::new(1, Draft::new(AccountId::new(1)))
    }

    #[test]
    fn a_new_message_starts_in_to_and_tab_walks_to_the_body() {
        let mut composer = fresh();
        assert_eq!(composer.field(), Field::To);
        tab(&mut composer);
        assert_eq!(composer.field(), Field::Subject, "Cc and Bcc are hidden");
        tab(&mut composer);
        assert_eq!(composer.field(), Field::Body);
        composer.type_key(key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(composer.field(), Field::Subject);
    }

    #[test]
    fn what_is_typed_becomes_the_draft() {
        let mut composer = fresh();
        typed(&mut composer, "grace@example.net, ada@example.com");
        tab(&mut composer);
        typed(&mut composer, "Tide gate");
        tab(&mut composer);
        typed(&mut composer, "Some **bold** words");

        let draft = composer.draft();
        assert_eq!(
            draft.to,
            vec![
                EmailAddress::new(None::<String>, "grace@example.net"),
                EmailAddress::new(None::<String>, "ada@example.com"),
            ]
        );
        assert_eq!(draft.subject, "Tide gate");
        assert_eq!(draft.body_markdown.as_deref(), Some("Some **bold** words"));
        assert_eq!(draft.body.text.as_deref(), Some("Some **bold** words"));
        assert!(
            draft
                .body
                .html
                .as_deref()
                .is_some_and(|html| html.contains("<strong>bold</strong>")),
            "{:?}",
            draft.body.html
        );
    }

    #[test]
    fn plain_words_send_no_html() {
        // US3 scenario 3.
        let body = body_of("Just words.\n\nTwo paragraphs of them.", &[]);
        assert!(body.html.is_none(), "{:?}", body.html);
        assert_eq!(
            body.text.as_deref(),
            Some("Just words.\n\nTwo paragraphs of them.")
        );
    }

    #[test]
    fn an_empty_body_sends_nothing() {
        assert_eq!(body_of("", &[]), MessageBody::default());
    }

    #[test]
    fn enter_in_the_body_is_a_new_line() {
        let mut composer = fresh();
        tab(&mut composer);
        tab(&mut composer);
        typed(&mut composer, "one");
        composer.type_key(key(KeyCode::Enter, KeyModifiers::NONE));
        typed(&mut composer, "two");
        assert_eq!(composer.markdown(), "one\ntwo");
    }

    #[test]
    fn cc_and_bcc_appear_on_demand_and_are_shown_when_the_draft_has_them() {
        let mut composer = fresh();
        assert!(!composer.shows_extra_recipients());
        composer.show_extra_recipients();
        assert_eq!(composer.field(), Field::Cc);
        typed(&mut composer, "ada@example.com");
        assert_eq!(composer.draft().cc.len(), 1);

        let mut draft = Draft::new(AccountId::new(1));
        draft.bcc = vec![EmailAddress::new(None::<String>, "archive@example.com")];
        let reopened = Composer::new(2, draft);
        assert!(reopened.shows_extra_recipients());
        assert_eq!(reopened.value(Field::Bcc), "archive@example.com");
    }

    #[test]
    fn a_draft_reopens_from_the_markdown_it_was_typed_in() {
        let mut draft = Draft::new(AccountId::new(1));
        draft.body_markdown = Some("- one\n- two".to_owned());
        draft.body.html = Some("<ul><li>one</li><li>two</li></ul>".to_owned());
        assert_eq!(Composer::new(1, draft).markdown(), "- one\n- two");
    }

    #[test]
    fn a_draft_saved_by_the_desktop_reopens_as_markdown() {
        let mut draft = Draft::new(AccountId::new(1));
        draft.body.text = Some("Some bold words".to_owned());
        draft.body.html = Some("<p>Some <strong>bold</strong> words</p>".to_owned());
        assert_eq!(Composer::new(1, draft).markdown(), "Some **bold** words");
    }

    #[test]
    fn a_pasted_escape_is_kept_harmless_and_windows_line_ends_are_not_kept() {
        let mut composer = fresh();
        tab(&mut composer);
        tab(&mut composer);
        composer.insert("one\r\ntwo\u{1b}[2J");
        let markdown = composer.markdown();
        assert!(
            !markdown.contains('\u{1b}') && !markdown.contains('\r'),
            "{markdown:?}"
        );
        assert!(markdown.starts_with("one\ntwo"), "{markdown:?}");
    }

    /// A message from Ada saying "Hello **there**", and the account it came to.
    pub(crate) fn a_message_and_its_account() -> (postio_model::Message, postio_model::Account) {
        let mut account = postio_model::Account::new(
            "grace",
            EmailAddress::new(Some("Grace"), "grace@example.net"),
        );
        account.id = AccountId::new(1);
        let mut message = postio_model::Message::new(
            account.id,
            postio_model::MailboxId::new(1),
            chrono::Utc::now(),
        );
        message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
        message.to = vec![EmailAddress::new(Some("Grace"), "grace@example.net")];
        message.subject = Some("Tide gate".to_owned());
        message.body.text = Some("Hello there".to_owned());
        message.body.html = Some("<p>Hello <b>there</b></p>".to_owned());
        (message, account)
    }

    fn a_reply() -> Draft {
        let (message, account) = a_message_and_its_account();
        postio_body::replying::reply_draft(
            postio_body::replying::ReplyKind::Reply,
            &message,
            &account,
        )
    }

    #[test]
    fn a_reply_keeps_the_quote_apart_from_what_is_typed_and_sends_it() {
        let mut composer = Composer::new(1, a_reply());
        assert_eq!(
            composer.field(),
            Field::Body,
            "a reply is already addressed"
        );
        assert!(
            !composer.markdown().contains("Hello"),
            "the quote is not in the editor: {:?}",
            composer.markdown()
        );
        assert!(composer.quote_lines() > 0, "but it is kept");

        composer.insert("Thanks!");
        let draft = composer.draft();
        let html = draft.body.html.expect("a quote is HTML");
        assert!(
            html.contains("<blockquote") && html.contains("there"),
            "{html}"
        );
        let text = draft.body.text.expect("a text part");
        assert!(text.starts_with("Thanks!"), "{text:?}");
        assert!(text.contains("> Hello there"), "{text:?}");
        assert_eq!(draft.to[0].address, "ada@example.com");
        assert_eq!(draft.subject, "Re: Tide gate");
    }

    #[test]
    fn a_reply_written_here_reopens_with_its_quote() {
        let mut composer = Composer::new(1, a_reply());
        composer.insert("Thanks!");
        let saved = composer.draft();

        let reopened = Composer::new(2, saved.clone());
        assert_eq!(reopened.markdown(), composer.markdown());
        assert_eq!(reopened.draft().body, saved.body);
    }

    fn identity(id: i64, address: &str, default: bool) -> postio_model::Identity {
        let mut identity = postio_model::Identity::new(
            AccountId::new(1),
            EmailAddress::new(None::<String>, address),
        );
        identity.id = postio_model::ids::IdentityId::new(id);
        identity.is_default = default;
        identity
    }

    #[test]
    fn an_account_with_several_addresses_asks_which_one_to_send_as() {
        let mut composer = fresh().with_identities(vec![
            identity(1, "grace@example.net", true),
            identity(2, "gh@example.org", false),
        ]);
        assert_eq!(composer.field(), Field::From);
        assert_eq!(composer.value(Field::From), "grace@example.net");
        composer.type_key(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(composer.value(Field::From), "gh@example.org");
        assert_eq!(
            composer.draft().identity_id,
            Some(postio_model::ids::IdentityId::new(2))
        );
        tab(&mut composer);
        assert_eq!(composer.field(), Field::To);
    }

    #[test]
    fn one_address_needs_no_asking() {
        let composer = fresh().with_identities(vec![identity(1, "grace@example.net", true)]);
        assert_eq!(composer.field(), Field::To);
        assert!(!composer.shows_identities());
    }

    #[test]
    fn a_reply_keeps_the_identity_it_was_answered_as() {
        let mut draft = Draft::new(AccountId::new(1));
        draft.identity_id = Some(postio_model::ids::IdentityId::new(2));
        let composer = Composer::new(1, draft).with_identities(vec![
            identity(1, "grace@example.net", true),
            identity(2, "gh@example.org", false),
        ]);
        assert_eq!(composer.value(Field::From), "gh@example.org");
    }

    #[test]
    fn a_paste_of_words_lands_at_the_cursor() {
        let mut composer = fresh();
        tab(&mut composer);
        tab(&mut composer);
        typed(&mut composer, "a c");
        composer.type_key(key(KeyCode::Left, KeyModifiers::NONE));
        composer.insert("b ");
        assert_eq!(composer.markdown(), "a b c");
    }
}
