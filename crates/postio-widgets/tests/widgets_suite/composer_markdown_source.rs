//! The Markdown source view (spec 014 US7), at the composer: one case per
//! row of `contracts/source-view.md`'s surface table, the Mac's aside.
//!
//! Commands are dispatched as the host's keys dispatch them; typing goes
//! through the view's own buffer, as a key press does.

use crate::support_compose::{Window, app, fonts, settle, style};
use gtk::gdk;
use postio_body::markdown::{DocPosition, to_document};
use postio_core::CommandId;
use postio_model::{AccountId, Draft};
use postio_widgets::composer::Composer;

fn ready() -> Option<(Window, Composer)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return None;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);
    let window = Window::default();
    let composer = window.composer();
    window.present();
    Some((window, composer))
}

fn holding(composer: &Composer, document: &postio_body::Document) {
    let mut draft = Draft::new(AccountId::new(1));
    draft.body = postio_body::message_body(document);
    composer.resume(draft);
    settle();
}

/// The switch, waited out: the rich view says where its caret is before
/// the Markdown view opens.
fn toggle(composer: &Composer) {
    let was = composer.test_source_text().is_some();
    composer.dispatch(CommandId::ToggleMarkdownSource);
    crate::support_compose::settle_until("the view to switch", || {
        composer.test_source_text().is_some() != was
    });
    settle();
}

/// The golden document, switched to Markdown and back with no edit, is the
/// same document, and the draft's undo history has no new step.
pub fn switching_there_and_back_changes_nothing() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    let golden = postio_body::golden::golden();
    holding(&composer, &golden);
    let could_undo = composer.test_can_undo();
    toggle(&composer);
    assert!(
        composer.test_source_text().is_some(),
        "the view did not open"
    );
    toggle(&composer);
    assert_eq!(composer.test_source_text(), None, "the view did not close");
    assert_eq!(
        composer.document(),
        golden,
        "the round trip changed the draft"
    );
    assert_eq!(
        composer.test_can_undo(),
        could_undo,
        "an unedited session left an undo step"
    );
}

/// The caret in a struck word is on the same character in `~~…~~`.
pub fn the_caret_keeps_its_character_in_a_struck_word() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    holding(&composer, &to_document("Thanks, ~~Wednesday~~ works."));
    // "Thanks, " is the first text node, "Wednesday" the second: after "Wed".
    composer.test_select_body(1, 3, 3);
    toggle(&composer);
    let text = composer.test_source_text().expect("the view");
    let caret = composer.test_source_caret();
    assert_eq!(
        &text[..caret],
        "Thanks, ~~Wed",
        "the caret moved: {text:?} at {caret}"
    );
    toggle(&composer);
    assert_eq!(
        composer.test_caret(),
        Some(DocPosition {
            path: vec![0],
            offset: 11
        }),
        "the caret did not come back to the same character"
    );
}

/// In the view, bold over a selection wraps it in `**`.
pub fn bold_in_the_view_wraps_the_selection() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    holding(&composer, &to_document("make this loud"));
    toggle(&composer);
    composer.test_source_select(10, 14);
    composer.dispatch(CommandId::Bold);
    settle();
    assert_eq!(
        composer.test_source_text().as_deref(),
        Some("make this **loud**")
    );
}

/// Every now-phase formatting command edits the Markdown in the view.
pub fn every_now_phase_command_edits_the_markdown_in_the_view() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    for (id, after) in [
        (CommandId::Strikethrough, "~~word~~"),
        (CommandId::InlineCode, "`word`"),
        (CommandId::Heading3, "### word"),
        (CommandId::TaskList, "- [ ] word"),
        (CommandId::CodeBlock, "```\nword\n```"),
        (CommandId::Divider, "word\n\n---\n"),
    ] {
        holding(&composer, &to_document("word"));
        toggle(&composer);
        composer.test_source_select(0, 4);
        composer.dispatch(id);
        settle();
        assert_eq!(composer.test_source_text().as_deref(), Some(after), "{id}");
        toggle(&composer);
        composer.discard();
        settle();
    }
}

fn reply() -> postio_body::Document {
    let mut document = to_document("Thanks.");
    document
        .blocks
        .push(postio_body::Block::Quoted(postio_body::quote_of(
            Some("<p>they wrote</p>"),
            "they wrote",
            "quote",
        )));
    document
}

/// The quote is one chip line: Backspace takes it whole, and undo brings
/// it back.
pub fn the_quote_chip_goes_whole_and_comes_back() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    holding(&composer, &reply());
    toggle(&composer);
    let text = composer.test_source_text().expect("the view");
    let line = postio_ui::editor::source::quote_line();
    assert!(text.ends_with(&line), "{text:?}");
    let end = text.len();
    composer.test_source_select(end, end);
    composer.test_source_key(gdk::Key::BackSpace);
    let deleted = composer.test_source_text().expect("the view");
    assert!(
        !deleted.contains(postio_ui::editor::source::QUOTE_LINE),
        "{deleted:?}"
    );
    assert!(
        !deleted.contains("Quoted"),
        "only part of the chip went: {deleted:?}"
    );
    composer.test_source_undo();
    assert_eq!(
        composer.test_source_text(),
        Some(text),
        "undo did not bring the chip back"
    );
    toggle(&composer);
    assert_eq!(
        composer.document(),
        reply(),
        "the quote did not come back where it was"
    );
}

/// The quote line's private character cannot be pasted in.
pub fn the_quote_mark_cannot_be_pasted() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    holding(&composer, &to_document("x"));
    toggle(&composer);
    composer.test_source_select(1, 1);
    composer.test_source_paste("a\u{E001}b");
    assert_eq!(composer.test_source_text().as_deref(), Some("xab"));
}

/// A ragged table, on return: a notice names the dropped cells, and its
/// Undo restores the text as typed.
pub fn a_ragged_table_is_reported_with_undo() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    holding(&composer, &to_document("x"));
    toggle(&composer);
    let typed = "| a |\n|---|\n| b | c |";
    composer.test_source_set(typed);
    toggle(&composer);
    assert_eq!(
        composer.test_source_notice().as_deref(),
        Some("1 table cell was dropped: their rows are longer than the header")
    );
    composer.test_press_source_notice_undo();
    settle();
    assert_eq!(composer.test_source_text().as_deref(), Some(typed));
}

/// A session of edits in the view is one undo step in the draft.
pub fn a_session_in_the_view_is_one_undo_step() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    holding(&composer, &to_document("before"));
    let before = composer.document();
    toggle(&composer);
    composer.test_source_set("after, **much** edited\n\n- [ ] task");
    toggle(&composer);
    assert_ne!(composer.document(), before);
    composer.test_undo();
    settle();
    assert_eq!(
        composer.document(),
        before,
        "one undo did not return to before the switch"
    );
}

/// Saved in the view, a draft keeps the text as typed, and reopens in the
/// view with it.
pub fn a_draft_saved_in_the_view_reopens_in_it() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    holding(&composer, &to_document("x"));
    toggle(&composer);
    let typed = "* starred bullet\n\n__strong__";
    composer.test_source_set(typed);
    let draft = composer.draft();
    assert_eq!(
        draft.body_markdown.as_deref(),
        Some(typed),
        "the spelling was not kept"
    );
    assert_eq!(draft.body, postio_body::message_body(&to_document(typed)));
    composer.discard();
    settle();
    composer.resume(draft);
    settle();
    assert_eq!(composer.test_source_text().as_deref(), Some(typed));
}

/// In the rich view, a command only the Markdown view makes says so.
pub fn the_rich_view_says_where_a_waiting_command_is() {
    let Some((_window, composer)) = ready() else {
        return;
    };
    let document = to_document("words");
    holding(&composer, &document);
    composer.dispatch(CommandId::Strikethrough);
    settle();
    assert_eq!(
        composer.status(),
        "Strikethrough is in the Markdown view for now \u{b7} ctrl+alt+s"
    );
    assert_eq!(composer.document(), document, "something changed");
    assert_eq!(composer.test_source_text(), None);
}

/// The strip says what mode this is, shorter when the column is narrow.
pub fn the_strip_says_markdown() {
    use gtk::prelude::*;
    let Some((_window, composer)) = ready() else {
        return;
    };
    holding(&composer, &to_document("x"));
    toggle(&composer);
    assert_eq!(
        composer.test_source_strip().as_deref(),
        Some("Editing as Markdown \u{b7} ctrl+alt+s for rich text")
    );
    toggle(&composer);

    let narrow = Window::default();
    narrow.gtk().set_default_size(420, 600);
    let composer = narrow.composer();
    narrow.present();
    holding(&composer, &to_document("x"));
    toggle(&composer);
    assert_eq!(
        composer.test_source_strip().as_deref(),
        Some("Markdown \u{b7} ctrl+alt+s")
    );
}
