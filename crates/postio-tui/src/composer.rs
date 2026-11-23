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

/// The desktop composer's formatting commands, as the Markdown they stand
/// for in this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `**…**`.
    Bold,
    /// `*…*`.
    Italic,
    /// `~~…~~`.
    Strikethrough,
    /// A backtick run around the words.
    InlineCode,
    /// One to six `#`s at the start of the line.
    Heading(u8),
    /// The line's `#`s taken off.
    NormalText,
    /// Fences around the lines.
    CodeBlock,
    /// `[ ] ` after each item's marker.
    TaskList,
    /// `[ ]` and `[x]` swapped.
    ToggleTask,
    /// The item nested under the one above.
    Indent,
    /// The item out one level.
    Outdent,
    /// `---` below the block.
    Divider,
    /// A two-by-two table template.
    Table,
    /// The marks taken off the words.
    ClearFormatting,
    /// `- ` at the start of the line.
    BulletList,
    /// `1. ` at the start of the line.
    NumberedList,
    /// `> ` at the start of the line.
    Quote,
    /// `[…](…)`.
    Link,
}

impl Format {
    /// The formatting command `id` names, if it is one the body answers.
    pub fn for_command(id: &str) -> Option<Self> {
        Some(match id {
            "bold" => Format::Bold,
            "italic" => Format::Italic,
            "strikethrough" => Format::Strikethrough,
            "inline_code" => Format::InlineCode,
            "heading_1" => Format::Heading(1),
            "heading_2" => Format::Heading(2),
            "heading_3" => Format::Heading(3),
            "heading_4" => Format::Heading(4),
            "heading_5" => Format::Heading(5),
            "heading_6" => Format::Heading(6),
            "normal_text" => Format::NormalText,
            "bullet_list" => Format::BulletList,
            "numbered_list" => Format::NumberedList,
            "quote_block" => Format::Quote,
            "insert_link" => Format::Link,
            "code_block" => Format::CodeBlock,
            "task_list" => Format::TaskList,
            "toggle_task" => Format::ToggleTask,
            "indent" => Format::Indent,
            "outdent" => Format::Outdent,
            "divider" => Format::Divider,
            "table" => Format::Table,
            "clear_formatting" => Format::ClearFormatting,
            _ => return None,
        })
    }
}

/// Where a remote image's download stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Remote {
    /// Running.
    Downloading,
    /// Refused, and why.
    Refused(postio_ui::compose::RemoteRefusal),
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
    /// Remote images whose download is running, or why one was refused,
    /// by address (spec 014 US8).
    remote: std::collections::HashMap<String, Remote>,
    /// The first body line the last frame showed: the textarea's own
    /// scroll, mirrored, since it does not say where it scrolled to.
    body_top: std::cell::Cell<u16>,
    /// How many lines the last frame showed of it.
    body_height: std::cell::Cell<u16>,
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
            remote: std::collections::HashMap::new(),
            body_top: std::cell::Cell::new(0),
            body_height: std::cell::Cell::new(0),
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

    pub fn quote(&self) -> String {
        quote_text(&self.quote)
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

    /// `copy_fields`: raise `Cc` and `Bcc`, or put them away again -- only
    /// while both are empty, since a hidden row still holding an address
    /// would still send to it, unseen. Refused, the keyboard goes to `Cc`,
    /// so the refusal shows.
    pub fn toggle_copy_fields(&mut self) {
        let empty = self.cc.value().trim().is_empty() && self.bcc.value().trim().is_empty();
        if !self.extra_recipients || !empty {
            self.show_extra_recipients();
            return;
        }
        self.extra_recipients = false;
        if matches!(self.field, Field::Cc | Field::Bcc) {
            self.field = Field::To;
        }
    }

    /// Apply `format` in the body: around the selection, or as a pair to
    /// type into, or to the line the caret is on. Answers whether anything
    /// changed; outside the body nothing does.
    pub fn format(&mut self, format: Format) -> bool {
        if self.field != Field::Body {
            return false;
        }
        use postio_ui::editor::markdown_edit as edit;
        match format {
            Format::Bold => self.apply(|text, at| edit::wrap(text, at, "**", "**")),
            Format::Italic => self.apply(|text, at| edit::wrap(text, at, "*", "*")),
            Format::Link => self.apply(edit::link),
            Format::Strikethrough => self.apply(edit::strikethrough),
            Format::InlineCode => self.apply(edit::inline_code),
            Format::Heading(level) => self.apply(|text, at| edit::heading(text, at, level)),
            Format::NormalText => self.apply(edit::normal_text),
            Format::CodeBlock => self.apply(edit::code_block),
            Format::TaskList => self.apply(edit::task_list),
            Format::ToggleTask => self.apply(edit::toggle_task),
            Format::Indent => self.apply(edit::indent),
            Format::Outdent => self.apply(edit::outdent),
            Format::Divider => self.apply(edit::divider),
            Format::Table => self.apply(edit::table),
            Format::ClearFormatting => self.apply(edit::clear_formatting),
            Format::BulletList => self.apply(|text, at| edit::toggle_prefix(text, at, "- ")),
            Format::NumberedList => self.apply(|text, at| edit::toggle_prefix(text, at, "1. ")),
            Format::Quote => self.apply(|text, at| edit::toggle_prefix(text, at, "> ")),
        }
        self.edits += 1;
        true
    }

    /// Run a Markdown edit (`postio_ui::editor::markdown_edit`) over the
    /// body: the edit decides, and the text area is brought to match.
    ///
    /// Only the span that changed is replaced, so the text area's own undo
    /// takes the edit back rather than the whole body.
    fn apply(
        &mut self,
        edit: impl FnOnce(&str, std::ops::Range<usize>) -> postio_ui::editor::markdown_edit::Edit,
    ) {
        let before = self.body.lines().join("\n");
        let offset = |row: usize, column: usize| -> usize {
            let lines = self.body.lines();
            let above: usize = lines.iter().take(row).map(|line| line.len() + 1).sum();
            let line = lines.get(row).map_or("", String::as_str);
            above
                + line
                    .char_indices()
                    .nth(column)
                    .map_or(line.len(), |(at, _)| at)
        };
        let selection = match self.body.selection_range() {
            Some(((r0, c0), (r1, c1))) => offset(r0, c0)..offset(r1, c1),
            None => {
                let ratatui_textarea::DataCursor(row, column) = self.body.cursor();
                let at = offset(row, column);
                at..at
            }
        };
        let after = edit(&before, selection);

        // The changed span: what is left once the common start and end are
        // taken off, on character boundaries.
        let mut head = before
            .bytes()
            .zip(after.text.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !before.is_char_boundary(head) || !after.text.is_char_boundary(head) {
            head -= 1;
        }
        let mut tail = before[head..]
            .bytes()
            .rev()
            .zip(after.text[head..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !before.is_char_boundary(before.len() - tail)
            || !after.text.is_char_boundary(after.text.len() - tail)
        {
            tail -= 1;
        }
        let jump = |text: &str, at: usize| {
            let row = text[..at].matches('\n').count();
            let column = text[text[..at].rfind('\n').map_or(0, |n| n + 1)..at]
                .chars()
                .count();
            ratatui_textarea::CursorMove::Jump(
                u16::try_from(row).unwrap_or(u16::MAX),
                u16::try_from(column).unwrap_or(u16::MAX),
            )
        };
        self.body.cancel_selection();
        self.body.move_cursor(jump(&before, head));
        if before.len() - tail > head {
            self.body.start_selection();
            self.body.move_cursor(jump(&before, before.len() - tail));
            let yanked = self.body.yank_text();
            self.body.cut();
            self.body.set_yank_text(yanked);
        }
        self.body
            .insert_str(&after.text[head..after.text.len() - tail]);
        if after.selection.is_empty() {
            self.body
                .move_cursor(jump(&after.text, after.selection.start));
        } else {
            self.body
                .move_cursor(jump(&after.text, after.selection.start));
            self.body.start_selection();
            self.body
                .move_cursor(jump(&after.text, after.selection.end));
        }
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

    /// Put `markdown` in place of the body: what the external editor saved.
    /// Everything else about the draft stays as it was.
    pub fn replace_body(&mut self, markdown: &str) {
        let lines: Vec<String> = markdown
            .split('\n')
            .map(|line| SafeText::new(line).as_str().to_owned())
            .collect();
        let mut body = TextArea::new(lines);
        body.set_cursor_line_style(ratatui::style::Style::default());
        self.body = body;
        self.edits += 1;
    }

    /// The message as it will arrive, drawn by the reader: the HTML part
    /// when there is one, else the text part (FR-021).
    pub fn preview(&self) -> crate::reader::Rendered {
        let body = body_of(&self.markdown(), &self.quote);
        match (body.html, body.text) {
            (Some(html), _) => crate::reader::from_html(&html),
            (None, Some(text)) => crate::reader::from_text(&text),
            (None, None) => crate::reader::Rendered::default(),
        }
    }

    /// The body is being drawn `height` lines tall: which of its lines is at
    /// the top. The textarea's own rule -- it scrolls only when the cursor
    /// would leave the view -- mirrored, so a click can be placed.
    pub fn body_top(&self, height: u16) -> u16 {
        self.body_height.set(height);
        let row = u16::try_from(self.body.cursor().0).unwrap_or(u16::MAX);
        let top = self.body_top.get();
        let top = if row < top {
            row
        } else if height > 0 && row >= top + height {
            row + 1 - height
        } else {
            top
        };
        self.body_top.set(top);
        top
    }

    /// Scroll the body by `lines`, down when positive, as far as its first
    /// and last lines allow; the cursor stays in view.
    pub fn scroll(&mut self, lines: isize) {
        let top = i64::from(self.body_top.get());
        let last = i64::try_from(self.body.lines().len())
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::from(self.body_height.get()))
            .max(0);
        let by = (top + lines as i64).clamp(0, last.max(top)) - top;
        let rows = i16::try_from(by).unwrap_or(0);
        self.body.scroll((rows, 0));
        self.body_top
            .set(self.body_top.get().saturating_add_signed(rows));
    }

    /// Put the cursor where a click landed in the body: `row` lines and
    /// `column` columns into what was drawn.
    pub fn click_body(&mut self, row: u16, column: u16) {
        self.field = Field::Body;
        let row = self.body_top.get().saturating_add(row);
        self.body
            .move_cursor(ratatui_textarea::CursorMove::Jump(row, column));
    }

    /// Put the keyboard in `field`.
    pub fn focus_field(&mut self, field: Field) {
        self.field = field;
    }

    /// The files this draft carries.
    pub fn attachments(&self) -> &[postio_model::Attachment] {
        &self.draft.attachments
    }

    /// The remote images the body names and has not attached, with their
    /// words (spec 014 US8).
    ///
    /// Asked on every frame, so a body that cannot name one -- no `](http`
    /// in it -- is not read as Markdown at all.
    pub fn remote_images(&self) -> Vec<(postio_body::WebHref, String)> {
        let text = self.markdown();
        if !text.to_ascii_lowercase().contains("](http") {
            return Vec::new();
        }
        markdown::to_document(&text).remote_images()
    }

    /// Each remote image's row: its words, and its host or what became of
    /// its download.
    pub fn remote_rows(&self, key: &str) -> Vec<(String, String)> {
        self.remote_images()
            .into_iter()
            .map(|(href, alt)| {
                let host = postio_ui::compose::remote_host(href.as_str());
                let words = match self.remote.get(href.as_str()) {
                    Some(Remote::Downloading) => postio_ui::compose::DOWNLOADING.to_owned(),
                    Some(Remote::Refused(refusal)) => {
                        postio_ui::compose::remote_refusal(refusal, &host, key)
                    }
                    None => host,
                };
                (alt, words)
            })
            .collect()
    }

    /// The first remote image not already downloading, marked as downloading:
    /// what one Download and attach press asks for.
    pub fn start_remote(&mut self) -> Option<postio_body::WebHref> {
        let href = self
            .remote_images()
            .into_iter()
            .map(|(href, _)| href)
            .find(|href| self.remote.get(href.as_str()) != Some(&Remote::Downloading))?;
        self.remote
            .insert(href.as_str().to_owned(), Remote::Downloading);
        Some(href)
    }

    /// A download came back: the picture in the image's place, by its
    /// content id, or the row saying why it stays a link.
    pub fn settle_remote(
        &mut self,
        href: &str,
        outcome: Result<postio_model::Attachment, postio_ui::compose::RemoteRefusal>,
    ) {
        match outcome {
            Ok(attachment) => {
                self.remote.remove(href);
                if let Some(id) = attachment.content_id.clone() {
                    let from = format!("]({href})");
                    let to = format!("](cid:{id})");
                    self.apply(|text, at| {
                        let mut edited = text.to_owned();
                        if let Some(found) = text.find(&from) {
                            edited.replace_range(found..found + from.len(), &to);
                        }
                        postio_ui::editor::markdown_edit::Edit {
                            text: edited,
                            selection: at,
                        }
                    });
                }
                self.attach(attachment);
            }
            Err(refusal) => {
                self.remote
                    .insert(href.to_owned(), Remote::Refused(refusal));
            }
        }
    }

    /// Carry `attachment`, already stored by the host.
    pub fn attach(&mut self, attachment: postio_model::Attachment) {
        self.draft.attachments.push(attachment);
        self.edits += 1;
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

    /// What the draft is: new, a reply, a reply to all, or a forward.
    pub fn kind(&self) -> postio_model::DraftKind {
        self.draft.kind
    }

    /// When a reminder is asked for if nobody replies, once one is chosen.
    pub fn remind_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.draft.remind_at
    }

    /// Ask for a reminder at `at` if nobody replies, or for none.
    pub fn set_remind_at(&mut self, at: Option<chrono::DateTime<chrono::Utc>>) {
        self.draft.remind_at = at;
        self.edits += 1;
    }

    /// How many words the body holds.
    pub fn words(&self) -> usize {
        self.body
            .lines()
            .iter()
            .map(|line| line.split_whitespace().count())
            .sum()
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

/// What a body of Markdown, followed by `quote`, sends: the written
/// document's plain alternative and then the quote's text as the text part,
/// and the rendering of both as the HTML part when there is anything to
/// style.
///
/// The plain alternative, not the Markdown as typed: `**bold**` in a text
/// part is the doubled marker spec 002 FR-072 forbids, and spec 014 D6 keeps
/// exactly the markers that carry meaning, the same from every composer. A
/// document of plain paragraphs reads as the words typed either way.
pub fn body_of(markdown: &str, quote: &[Block]) -> MessageBody {
    if markdown.trim().is_empty() && quote.is_empty() {
        return MessageBody::default();
    }
    let written = markdown::to_document(markdown);
    let written_text = written.to_flowed_text();
    let mut document = written;
    document.blocks.extend(quote.iter().cloned());
    let quoted = quote_text(quote);
    let text = match (markdown.trim().is_empty(), quoted.is_empty()) {
        (_, true) => written_text,
        (true, false) => quoted,
        (false, false) => format!("{}\n\n{quoted}", written_text.trim_end()),
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

    fn in_body(text: &str) -> Composer {
        let mut draft = Draft::new(AccountId::new(1));
        draft.body_markdown = Some(text.into());
        let mut composer = Composer::new(1, draft);
        composer.focus_field(Field::Body);
        composer
    }

    #[test]
    fn bold_and_italic_wrap_the_selection_or_open_a_pair_to_type_into() {
        let mut composer = in_body("tide gate");
        composer.body.select_all();
        assert!(composer.format(Format::Bold));
        assert_eq!(composer.markdown(), "**tide gate**");

        let mut composer = in_body("");
        assert!(composer.format(Format::Italic));
        typed(&mut composer, "now");
        assert_eq!(
            composer.markdown(),
            "*now*",
            "the caret sits between the pair"
        );
    }

    #[test]
    fn a_link_wraps_its_words_and_waits_for_the_address() {
        let mut composer = in_body("the docs");
        composer.body.select_all();
        assert!(composer.format(Format::Link));
        typed(&mut composer, "https://example.com");
        assert_eq!(composer.markdown(), "[the docs](https://example.com)");

        let mut composer = in_body("");
        composer.format(Format::Link);
        typed(&mut composer, "words");
        assert_eq!(composer.markdown(), "[words]()");
    }

    #[test]
    fn headings_lists_and_quotes_prefix_the_line_and_a_second_press_takes_it_off() {
        for (format, prefix) in [
            (Format::Heading(1), "# "),
            (Format::BulletList, "- "),
            (Format::NumberedList, "1. "),
            (Format::Quote, "> "),
        ] {
            let mut composer = in_body("one");
            assert!(composer.format(format));
            assert_eq!(composer.markdown(), format!("{prefix}one"));
            assert!(composer.format(format));
            assert_eq!(composer.markdown(), "one", "{format:?} again takes it off");
        }
    }

    #[test]
    fn every_now_phase_formatting_command_writes_its_markdown() {
        // Spec 014 US6 scenario 4: the terminal authors every construct.
        // The edits are `markdown_edit`'s; this is that the terminal runs
        // the right one for each command, over the whole line selected.
        for (format, before, after) in [
            (Format::Strikethrough, "gone", "~~gone~~"),
            (Format::InlineCode, "x", "`x`"),
            (Format::Heading(1), "Plan", "# Plan"),
            (Format::Heading(4), "Plan", "#### Plan"),
            (Format::Heading(6), "Plan", "###### Plan"),
            (Format::NormalText, "## Plan", "Plan"),
            (Format::CodeBlock, "let x", "```\nlet x\n```"),
            (Format::TaskList, "- one", "- [ ] one"),
            (Format::ToggleTask, "- [ ] one", "- [x] one"),
            (Format::Indent, "- a\n- b", "- a\n  - b"),
            (Format::Outdent, "- a\n  - b", "- a\n- b"),
            (Format::Divider, "above", "above\n\n---\n"),
            (
                Format::Table,
                "",
                "| Column 1 | Column 2 |\n|---|---|\n|  |  |",
            ),
            (Format::ClearFormatting, "**b** and *i*", "b and i"),
        ] {
            let mut composer = in_body(before);
            // The last line, selected: what a person would mark.
            composer
                .body
                .move_cursor(ratatui_textarea::CursorMove::Bottom);
            composer
                .body
                .move_cursor(ratatui_textarea::CursorMove::Head);
            composer.body.start_selection();
            composer.body.move_cursor(ratatui_textarea::CursorMove::End);
            assert!(composer.format(format), "{format:?}");
            assert_eq!(composer.markdown(), after, "{format:?} over {before:?}");
        }
    }

    #[test]
    fn a_remote_image_is_a_row_downloaded_on_a_press_and_put_in_place() {
        let mut composer = in_body("See ![chart](https://example.com/x.png).");
        assert_eq!(
            composer.remote_rows("alt+I"),
            vec![("chart".to_owned(), "example.com".to_owned())]
        );
        let href = composer.start_remote().expect("one to download");
        assert_eq!(href.as_str(), "https://example.com/x.png");
        assert_eq!(composer.remote_rows("alt+I")[0].1, "Downloading\u{2026}");
        assert_eq!(
            composer.start_remote(),
            None,
            "one request at a time for one image"
        );
        let mut part = postio_model::Attachment::new(
            postio_model::ids::MessageId::UNASSIGNED,
            "image/png",
            10,
        );
        part.content_id = Some("x1@postio.invalid".to_owned());
        composer.settle_remote(href.as_str(), Ok(part));
        assert_eq!(composer.markdown(), "See ![chart](cid:x1@postio.invalid).");
        assert!(composer.remote_rows("alt+I").is_empty());
        assert_eq!(composer.attachments().len(), 1);
    }

    #[test]
    fn a_refused_download_says_why_and_stays_a_link() {
        let mut composer = in_body("![chart](https://example.com/x.png)");
        let href = composer.start_remote().expect("one");
        composer.settle_remote(
            href.as_str(),
            Err(postio_ui::compose::RemoteRefusal::TooLarge),
        );
        assert_eq!(
            composer.remote_rows("alt+I")[0].1,
            "Larger than 10 MB \u{b7} not attached \u{b7} it will be sent as a link"
        );
        assert_eq!(composer.markdown(), "![chart](https://example.com/x.png)");
        assert!(
            composer.start_remote().is_some(),
            "a press again asks again"
        );
    }

    #[test]
    fn formatting_is_the_bodys_and_counts_as_an_edit() {
        let mut composer = fresh();
        assert_eq!(composer.field(), Field::To);
        assert!(!composer.format(Format::Bold), "not in a header field");
        let mut composer = in_body("x");
        let before = composer.edits();
        composer.format(Format::Quote);
        assert!(composer.edits() > before, "so it is autosaved");
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
        // The text part is the plain alternative every composer sends
        // (spec 014 D6): bold's stars are dropped, as spec 002 FR-072 has it.
        assert_eq!(draft.body.text.as_deref(), Some("Some bold words"));
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
    fn cc_and_bcc_go_away_only_while_empty() {
        // `copy_fields`' asymmetric half: a hidden row still holding an
        // address would still send to it, unseen.
        let mut composer = fresh();
        composer.toggle_copy_fields();
        assert!(composer.shows_extra_recipients());
        assert_eq!(composer.field(), Field::Cc);
        composer.toggle_copy_fields();
        assert!(!composer.shows_extra_recipients(), "empty, so they go");
        assert_eq!(composer.field(), Field::To, "and the keyboard leaves them");

        composer.toggle_copy_fields();
        composer.type_key(key(KeyCode::Tab, KeyModifiers::NONE));
        typed(&mut composer, "archive@example.com");
        composer.toggle_copy_fields();
        assert!(composer.shows_extra_recipients(), "Bcc holds an address");
        assert_eq!(composer.field(), Field::Cc, "the refusal is visible");
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
    fn a_click_in_a_scrolled_body_lands_on_the_line_shown_there() {
        let mut draft = Draft::new(AccountId::new(1));
        let lines: Vec<String> = (0..50).map(|line| format!("line {line}")).collect();
        draft.body_markdown = Some(lines.join("\n"));
        let mut composer = Composer::new(1, draft);
        composer.click_body(0, 0);
        for _ in 0..45 {
            composer.type_key(key(KeyCode::Down, KeyModifiers::NONE));
        }
        // Drawn ten lines tall with the cursor on line 45: lines 36 to 45.
        assert_eq!(composer.body_top(10), 36);
        composer.click_body(2, 3);
        assert_eq!(composer.body().cursor(), (38, 3));
    }

    #[test]
    fn the_wheel_scrolls_the_body_and_stops_at_either_end() {
        let mut draft = Draft::new(AccountId::new(1));
        let lines: Vec<String> = (0..50).map(|line| format!("line {line}")).collect();
        draft.body_markdown = Some(lines.join("\n"));
        let mut composer = Composer::new(1, draft);
        composer.click_body(0, 0);
        assert_eq!(composer.body_top(10), 0);
        composer.scroll(3);
        assert_eq!(composer.body_top(10), 3, "three lines down");
        composer.click_body(0, 0);
        assert_eq!(
            composer.body().cursor().0,
            3,
            "a click lands on the top line shown"
        );
        composer.scroll(500);
        assert_eq!(composer.body_top(10), 40, "the last line is at the bottom");
        composer.scroll(-500);
        assert_eq!(composer.body_top(10), 0);
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
