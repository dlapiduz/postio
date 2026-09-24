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
use postio_body::markdown;
use postio_model::{Draft, MessageBody};
use postio_ui::terminal::SafeText;
use ratatui_textarea::TextArea;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

/// Which part of the composer the keyboard is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
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
        let markdown = match (&draft.body_markdown, &draft.body.html, &draft.body.text) {
            (Some(markdown), _, _) => markdown.clone(),
            // Trimmed: a document's Markdown ends its last block with a line
            // break, which the editor would show as an empty last line.
            (None, Some(html), _) => markdown::from_document(&postio_body::parse(html))
                .trim_end_matches('\n')
                .to_owned(),
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
            draft,
        }
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
            _ => {}
        }
        match self.field {
            Field::Body => self.body.input(key),
            field => {
                let event = crossterm::event::Event::Key(key);
                self.line_mut(field)
                    .and_then(|input| input.handle_event(&event))
                    .is_some_and(|changed| changed.value)
            }
        }
    }

    /// The field `by` steps from the one the keyboard is in, over the
    /// fields that are shown, wrapping.
    fn step(&self, by: isize) -> Field {
        let shown: &[Field] = if self.extra_recipients {
            &[
                Field::To,
                Field::Cc,
                Field::Bcc,
                Field::Subject,
                Field::Body,
            ]
        } else {
            &[Field::To, Field::Subject, Field::Body]
        };
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
            Field::Body => None,
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
            Field::Body => 0,
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
        let markdown = self.markdown();
        draft.body = body_of(&markdown);
        draft.body_markdown = Some(markdown);
        draft
    }
}

/// What a body of Markdown sends: the Markdown as its text part (FR-021),
/// and its rendering as the HTML part when there is anything to style.
pub fn body_of(markdown: &str) -> MessageBody {
    if markdown.trim().is_empty() {
        return MessageBody::default();
    }
    let document = markdown::to_document(markdown);
    MessageBody {
        text: Some(markdown.to_owned()),
        html: (!document.is_plain_text()).then(|| postio_body::render(&document).1),
    }
}

#[cfg(test)]
mod tests {
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
        let body = body_of("Just words.\n\nTwo paragraphs of them.");
        assert!(body.html.is_none(), "{:?}", body.html);
        assert_eq!(
            body.text.as_deref(),
            Some("Just words.\n\nTwo paragraphs of them.")
        );
    }

    #[test]
    fn an_empty_body_sends_nothing() {
        assert_eq!(body_of(""), MessageBody::default());
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
