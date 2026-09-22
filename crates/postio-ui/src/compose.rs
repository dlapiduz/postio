//! What the composer says about itself.
//!
//! Two frontends write mail and both have a footer that names what will
//! leave. The words are here so they cannot differ, for the same reason the
//! reader's document assembly is shared: a claim about what Postio puts on
//! the wire is one it should be willing to make identically everywhere.

/// The MIME shape an outgoing message will have: `html + text/plain`, or
/// `text/plain, format=flowed`.
///
/// **The plain part is not optional.** Rich mail sends `text/html` *and* a
/// `text/plain` alternative, always — a recipient reading in a terminal, a
/// screen reader, or a client that refuses HTML gets the message rather than
/// an apology. Plain mail is wrapped at 72 columns and flowed (RFC 3676), so
/// it reads correctly whether the receiving client rewraps it or not.
pub fn outgoing_shape(rich: bool) -> String {
    if rich {
        "html + text/plain".to_owned()
    } else {
        "text/plain, format=flowed".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_always_carries_a_plain_alternative() {
        // The half that is easy to drop and expensive to notice: it is
        // invisible to the sender and it is the whole message to some
        // recipients.
        assert!(outgoing_shape(true).contains("text/plain"));
    }

    #[test]
    fn plain_says_it_is_flowed_rather_than_just_plain() {
        // `format=flowed` is the difference between a paragraph that rewraps
        // in a narrow window and one that arrives with a ragged 72-column
        // edge, and the footer is where a person can see which they chose.
        assert_eq!(outgoing_shape(false), "text/plain, format=flowed");
    }
}

/// The composer's editing bridge — the one script Postio runs in a web view.
///
/// Both frontends put a `contenteditable` document inside a web view and
/// both need the same three things from it: the paragraph separator and
/// `styleWithCSS` settings that pin the dialect [`postio_body::parse()`] reads
/// back, an edit channel carrying `innerHTML`, and a reflection channel
/// saying what formatting is in force at the caret.
///
/// One copy, because a second one is a second dialect. The GTK reader's
/// `gtk_editable_dialect.rs` proves the surface emits `<p>` paragraphs and
/// element-form bold/italic; a macOS surface running a *different* script
/// would emit `<div>`s and `<span style>`s, `parse` would narrow them to
/// something else, and the two composers would disagree about what the same
/// keystrokes wrote — invisibly, because both would still round-trip
/// through a `Document`.
///
/// `window.webkit.messageHandlers` is the same API on WebKitGTK and on
/// `WKWebView`, which is why one file can serve both without a shim.
pub const EDITOR_SCRIPT: &str = include_str!("../data/editor.js");

/// The bridge as a frontend should run it: the markdown table, then the body.
///
/// [`EDITOR_SCRIPT`] alone is not runnable. It reads `POSTIO_MARKDOWN` and
/// does not define it, because the set of supported sequences is
/// [`crate::editor::markdown::SEQUENCES`] and a hand-written copy in
/// JavaScript is a copy that drifts from the contract both frontends
/// implement. So the table is generated and prepended here, once, rather
/// than once per frontend.
///
/// Built on first use and kept: the table is a few hundred bytes derived
/// from a `&'static` table, so it cannot change while the process runs, and
/// the composer asks for it every time a window opens.
pub fn editor_script() -> &'static str {
    static SCRIPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SCRIPT.get_or_init(|| format!("{}{EDITOR_SCRIPT}", markdown_table_js()))
}

/// `POSTIO_MARKDOWN`, as a JavaScript literal.
///
/// The markers are `&'static str` from a table in this workspace, not
/// anybody's input, and every one of them is punctuation — but they are
/// being written into source, so they are escaped rather than trusted to
/// stay that way.
pub fn markdown_table_js() -> String {
    use crate::editor::markdown::{SEQUENCES, Trigger};
    use std::fmt::Write as _;

    let mut out = String::from("const POSTIO_MARKDOWN = [\n");
    for sequence in SEQUENCES {
        let trigger = match sequence.trigger {
            Trigger::LinePrefix => "line_prefix",
            Trigger::Wrapping => "wrapping",
        };
        let _ = writeln!(
            out,
            "    {{ marker: \"{}\", command: \"{}\", trigger: \"{trigger}\" }},",
            sequence.marker.replace('\\', "\\\\").replace('"', "\\\""),
            sequence.command,
        );
    }
    out.push_str("];\n");
    out
}

#[cfg(test)]
mod editor_script_tests {
    use super::EDITOR_SCRIPT;

    /// `postio-gtk` still `include_str!`s its own copy, because this branch
    /// is worked from a Mac and `issue-land.sh` will not land a crate whose
    /// gates cannot run there. Until a Linux session points it here, the two
    /// files are pinned to each other: this fails the moment either is
    /// edited alone, which is the only thing that makes a temporary
    /// duplicate safe.
    /// The body names `POSTIO_MARKDOWN` and does not define it — the table
    /// is generated from [`crate::editor::markdown::SEQUENCES`] so the set of
    /// supported sequences has one source. A frontend handed the body alone
    /// gets a `ReferenceError` on the first keystroke, and nothing on either
    /// side of the boundary would say so: the script is loaded into a WebView
    /// and its failures stay there.
    #[test]
    fn the_assembled_script_defines_the_table_before_it_reads_it() {
        let script = super::editor_script();
        let defined = script
            .find("const POSTIO_MARKDOWN")
            .expect("the table is defined");
        let read = script
            .find("for (const sequence of POSTIO_MARKDOWN)")
            .expect("the body reads the table");
        assert!(
            defined < read,
            "the table has to be in scope before the recognisers run"
        );
        assert!(
            script.contains(r#"marker: "**", command: "bold""#),
            "the table is the real one, not an empty literal: {}",
            &script[defined..read.min(defined + 400)]
        );
    }

    #[test]
    fn the_gtk_copy_of_the_bridge_has_not_drifted_from_this_one() {
        let gtk =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../postio-gtk/data/editor.js");
        let theirs = std::fs::read_to_string(&gtk)
            .unwrap_or_else(|error| panic!("reading {}: {error}", gtk.display()));
        assert_eq!(
            theirs, EDITOR_SCRIPT,
            "the two copies of the editing bridge have diverged, which is two \
             dialects: make them equal, or finish the move and delete the GTK one"
        );
    }
}

/// The script that applies one of the composer's marks to the selection.
///
/// The mapping from a registry command to what a `contenteditable` document
/// does about it is a *decision* — `quote_block` is the interesting one,
/// because `formatBlock` does not toggle and the toggle is Postio's. Writing
/// it once here rather than once per frontend is the same argument as
/// [`EDITOR_SCRIPT`]: two hosts running different scripts for `bold` would
/// produce different markup, `postio_body::parse` would narrow it
/// differently, and the two composers would disagree about what the same
/// button did.
///
/// `None` for a command that is not one of the marks, which is how a caller
/// tells "this button does something else" from "this button is broken".
///
/// Every script ends by dispatching `input`, because `execCommand` does not
/// always raise one (bold yes, `insertUnorderedList` no) and the host's
/// record is only updated by that event. A duplicate event is absorbed as a
/// no-change; a missing one loses the edit.
pub fn mark_script(command: &str) -> Option<String> {
    let body = match command {
        "bold" => "document.execCommand('bold');",
        "italic" => "document.execCommand('italic');",
        "bullet_list" => "document.execCommand('insertUnorderedList');",
        "numbered_list" => "document.execCommand('insertOrderedList');",
        // `formatBlock` toggles nothing on its own; the toggle is ours.
        "quote_block" => {
            "if (document.queryCommandValue('formatBlock') === 'blockquote') { \
                 document.execCommand('formatBlock', false, 'p'); \
             } else { \
                 document.execCommand('formatBlock', false, 'blockquote'); \
             }"
        }
        _ => return None,
    };
    Some(format!(
        "{body} document.dispatchEvent(new Event('input'));"
    ))
}

/// The script that turns the selection into a link to `href`.
///
/// `None` when `href` is not something a message may link to. The gate is
/// the canonical subset's — `postio_body::Href` refuses anything but http,
/// https and mailto — and it is applied *here*, before the document is
/// touched, so the composer can say so rather than have the link silently
/// disappear at the next parse.
pub fn link_script(href: &str) -> Option<String> {
    postio_body::Href::parse(href)?;
    // Single quotes and backslashes escaped: the href has been through
    // `Href::parse`, which refuses control characters, but it is still
    // somebody's input going into a script literal.
    let escaped = href.replace('\\', "\\\\").replace('\'', "\\'");
    Some(format!(
        "document.execCommand('createLink', false, '{escaped}'); \
         document.dispatchEvent(new Event('input'));"
    ))
}

/// The script that puts an inline image at the caret (#1571).
///
/// `content_id` is the part's `Content-ID`, as
/// `postio_session::attaching::inline_image` minted it; the source written is
/// the editing shell's `postio-cid:` form, which the composer's scheme handler
/// resolves while the draft is open and the send path rewrites to the wire's
/// `cid:`. `None` for an id [`postio_body::ContentId`] refuses -- anything that
/// could be read as a URL -- so nothing but a part of this message can ever
/// become an image source here.
///
/// The same shape as [`link_script`], and for its reason: both composers run
/// this, so an image inserted on a Mac and one inserted on Linux are the same
/// edit. The caret fallback is the one `postio-gtk`'s editor learned: a
/// picture can be the first gesture into a fresh body, and `insertHTML`
/// silently does nothing without a selection.
pub fn image_script(content_id: &str, alt: &str) -> Option<String> {
    let content_id = postio_body::ContentId::parse(content_id)?;
    let mut img = String::from("<img src=\"");
    img.push_str(&postio_body::editor_image_src(&content_id));
    img.push_str("\" alt=\"");
    for c in alt.chars() {
        match c {
            '&' => img.push_str("&amp;"),
            '<' => img.push_str("&lt;"),
            '>' => img.push_str("&gt;"),
            '"' => img.push_str("&quot;"),
            other => img.push(other),
        }
    }
    img.push_str("\">");
    // Into a single-quoted script literal: its own escapes first, then the
    // quote, and line breaks, which a literal cannot hold.
    let escaped = img
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    Some(format!(
        "(() => {{ const sel = window.getSelection(); \
           if (sel.rangeCount === 0) {{ \
             const range = document.createRange(); \
             range.selectNodeContents(document.body); \
             range.collapse(false); \
             sel.addRange(range); \
           }} \
           document.execCommand('insertHTML', false, '{escaped}'); \
           document.dispatchEvent(new Event('input')); }})()"
    ))
}

#[cfg(test)]
mod mark_tests {
    use super::*;

    #[test]
    fn every_mark_the_bar_offers_has_a_script() {
        // The bar is built from the registry ids; a mark with no script is a
        // button that does nothing, which is what #1271 reported.
        for command in [
            "bold",
            "italic",
            "bullet_list",
            "numbered_list",
            "quote_block",
        ] {
            assert!(
                mark_script(command).is_some(),
                "{command} has no script, so its button is decoration"
            );
        }
    }

    #[test]
    fn a_command_that_is_not_a_mark_has_none() {
        // So a caller can tell "does something else" from "broken".
        assert_eq!(mark_script("archive"), None);
        assert_eq!(mark_script("insert_link"), None, "link needs an href");
    }

    #[test]
    fn every_script_raises_the_event_the_host_records_on() {
        // `execCommand` does not always raise `input` -- bold does,
        // `insertUnorderedList` does not -- and the host's record is only
        // updated by that event. A mark that skipped it would apply on
        // screen and be lost on save.
        for command in ["bold", "bullet_list", "quote_block"] {
            let script = mark_script(command).expect("a script");
            assert!(
                script.contains("new Event('input')"),
                "{command} applies without telling the host: {script}"
            );
        }
    }

    #[test]
    fn quoting_toggles_rather_than_only_applying() {
        // `formatBlock` has no toggle of its own, so pressing Quote twice
        // would otherwise nest rather than undo.
        let script = mark_script("quote_block").expect("a script");
        assert!(script.contains("queryCommandValue"), "no toggle: {script}");
        assert!(
            script.contains("'p'"),
            "nothing to toggle back to: {script}"
        );
    }

    #[test]
    fn a_link_to_somewhere_a_message_may_point_gets_a_script() {
        assert!(link_script("https://example.com").is_some());
        assert!(link_script("mailto:ada@example.com").is_some());
    }

    #[test]
    fn a_link_the_subset_refuses_is_refused_here_rather_than_at_the_next_parse() {
        // Refused up front so the composer can say so. Left to the parse, the
        // link would be created, look right, and vanish on save.
        assert_eq!(link_script("javascript:alert(1)"), None);
        assert_eq!(link_script("file:///etc/passwd"), None);
        assert_eq!(
            link_script("example.com"),
            None,
            "relative means nothing in mail"
        );
    }

    #[test]
    fn a_quote_in_an_href_cannot_close_the_script_literal() {
        // `Href::parse` refuses control characters, not quotes.
        let script = link_script("https://example.com/a'b").expect("a script");
        assert!(script.contains("a\\'b"), "unescaped quote: {script}");
    }

    // -- an inline image (#1571) ---------------------------------------------

    #[test]
    fn an_image_is_inserted_in_the_editing_scheme_at_the_caret() {
        let script = image_script("0a1b@postio.invalid", "inline-image.png").expect("a script");
        // The editing shell's scheme, the one its handler resolves -- the
        // wire's `cid:` is what the *sent* message says, and a composer that
        // wrote it would draw a broken picture while the draft was open.
        assert!(
            script.contains("src=\"postio-cid:0a1b%40postio.invalid\""),
            "{script}"
        );
        assert!(script.contains("insertHTML"), "{script}");
        assert!(
            script.contains("new Event('input')"),
            "an insertion the host is not told about is lost on save: {script}"
        );
    }

    #[test]
    fn a_fresh_body_with_no_caret_still_takes_the_image() {
        // A picture can be the first gesture into a new message, before any
        // click has given the document a caret, and `insertHTML` silently
        // does nothing without one.
        let script = image_script("a@postio.invalid", "x").expect("a script");
        assert!(script.contains("rangeCount === 0"), "{script}");
    }

    #[test]
    fn an_id_that_could_be_read_as_a_url_has_no_script() {
        // `ContentId`'s rule: a colon or a slash could make a source of it.
        assert_eq!(image_script("//evil.example/pixel", "x"), None);
        assert_eq!(image_script("", "x"), None);
    }

    #[test]
    fn the_alt_text_is_markup_escaped_and_cannot_close_the_literal() {
        let script = image_script("a@postio.invalid", "Ada's <b>\"logo\"</b>").expect("a script");
        assert!(!script.contains("<b>"), "markup in the alt text: {script}");
        assert!(script.contains("&lt;b&gt;"), "{script}");
        assert!(script.contains("&quot;logo&quot;"), "{script}");
        assert!(
            script.contains("Ada\\'s"),
            "an unescaped quote ends the literal: {script}"
        );
    }
}

/// How many recipients this message has, and on which field.
///
/// FR-023, and the surprise it exists to prevent: **a reply-to-all to a large
/// list looks exactly like a reply until it is sent.** The count says *which
/// field* because "42 recipients" reads very differently from "1 To, 41 Cc".
///
/// `None` for the ordinary case, deliberately. A banner that is always there
/// is a banner nobody reads, so this says nothing until there is more than
/// one person on the message.
///
/// Shared because both composers need it and the macOS one had neither this
/// nor the Bcc field it counts — so a reply-all there showed one address and
/// silently addressed everybody else. Counts rather than the addresses
/// themselves: this is a reassurance about scale, and the fields beside it
/// are where the names are.
pub fn recipient_summary(to: usize, cc: usize, bcc: usize) -> Option<String> {
    if to + cc + bcc <= 1 {
        return None;
    }
    let counted: Vec<String> = [("To", to), ("Cc", cc), ("Bcc", bcc)]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(name, count)| format!("{count} {name}"))
        .collect();
    Some(counted.join(", "))
}

#[cfg(test)]
mod recipient_tests {
    use super::recipient_summary;

    #[test]
    fn one_person_gets_no_banner_at_all() {
        // A banner that is always there is a banner nobody reads.
        assert_eq!(recipient_summary(1, 0, 0), None);
        assert_eq!(recipient_summary(0, 0, 0), None);
    }

    #[test]
    fn a_reply_all_to_a_list_says_which_field_the_crowd_is_on() {
        // "42 recipients" reads very differently from "1 To, 41 Cc", and the
        // second is the one that tells somebody what they are about to do.
        assert_eq!(recipient_summary(1, 41, 0).as_deref(), Some("1 To, 41 Cc"));
    }

    #[test]
    fn an_empty_field_is_not_counted_at_zero() {
        assert_eq!(recipient_summary(2, 0, 0).as_deref(), Some("2 To"));
        assert_eq!(recipient_summary(1, 0, 3).as_deref(), Some("1 To, 3 Bcc"));
    }

    #[test]
    fn the_fields_are_named_in_the_order_they_are_drawn() {
        assert_eq!(
            recipient_summary(1, 2, 3).as_deref(),
            Some("1 To, 2 Cc, 3 Bcc")
        );
    }
}

/// A preset must land at least this far ahead of `now` to be offered as
/// "today" rather than rolling to tomorrow — a picker opened one minute
/// before 6pm must not offer "this evening" for an instant already gone.
const MIN_SCHEDULE_LEAD: chrono::Duration = chrono::Duration::minutes(5);

/// One thing *Schedule send…* offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulePreset {
    /// What the row says — "Tomorrow morning".
    pub label: &'static str,
    /// When that is, in the local zone.
    pub when: chrono::DateTime<chrono::Local>,
}

/// `day` at the given wall-clock hour and minute, in `day`'s own local zone.
///
/// A DST transition can make a wall-clock time ambiguous or nonexistent;
/// falling back to `day` itself rather than panicking keeps a schedule-send
/// picker from crashing the composer on the two days a year this can happen,
/// at the cost of an odd-looking preset on exactly those days.
fn at_local_time(
    day: chrono::DateTime<chrono::Local>,
    hour: u32,
    minute: u32,
) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    day.date_naive()
        .and_hms_opt(hour, minute, 0)
        .and_then(|naive| chrono::Local.from_local_datetime(&naive).single())
        .unwrap_or(day)
}

/// The four times *Schedule send…* offers, computed against `now`.
///
/// Recomputed every time the picker opens rather than once, since "in 1 hour"
/// a picker opened yesterday is not "in 1 hour" today.
///
/// "This evening" rolls to tomorrow once 6pm today is behind `now`. "Monday
/// morning" always means a Monday strictly after today: opening the picker on
/// a Monday offers next week's, not the one already underway.
///
/// Shared because the four times *are* the feature. Two frontends each
/// deciding what "tomorrow morning" means is two products, and the one that
/// is wrong sends somebody's mail at the wrong hour without ever saying so.
pub fn schedule_presets(now: chrono::DateTime<chrono::Local>) -> [SchedulePreset; 4] {
    use chrono::{Datelike, Duration};

    let in_one_hour = now + Duration::hours(1);

    let mut evening = at_local_time(now, 18, 0);
    if evening < now + MIN_SCHEDULE_LEAD {
        evening = at_local_time(now + Duration::days(1), 18, 0);
    }

    let tomorrow_morning = at_local_time(now + Duration::days(1), 8, 0);

    let days_from_monday = i64::from(now.weekday().num_days_from_monday());
    let days_until_monday = if days_from_monday == 0 {
        7
    } else {
        7 - days_from_monday
    };
    let monday_morning = at_local_time(now + Duration::days(days_until_monday), 8, 0);

    [
        SchedulePreset {
            label: "In 1 hour",
            when: in_one_hour,
        },
        SchedulePreset {
            label: "This evening",
            when: evening,
        },
        SchedulePreset {
            label: "Tomorrow morning",
            when: tomorrow_morning,
        },
        SchedulePreset {
            label: "Monday morning",
            when: monday_morning,
        },
    ]
}

#[cfg(test)]
mod schedule_tests {
    use super::*;
    use chrono::{Datelike, Duration, TimeZone, Timelike};

    fn local(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
    ) -> chrono::DateTime<chrono::Local> {
        chrono::Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("an unambiguous local time")
    }

    #[test]
    fn an_hour_from_now_is_an_hour_from_now() {
        // The one preset that is relative rather than a wall-clock time, and
        // the reason the whole set is recomputed when the picker opens.
        let now = local(2026, 3, 4, 9, 30);
        assert_eq!(schedule_presets(now)[0].when, now + Duration::hours(1));
    }

    #[test]
    fn this_evening_rolls_to_tomorrow_once_it_has_gone() {
        // A picker opened at 5:58pm must not offer "this evening" for an
        // instant two minutes away — and one opened at 7pm must not offer a
        // time already an hour behind.
        let before = local(2026, 3, 4, 12, 0);
        assert_eq!(schedule_presets(before)[1].when.day(), 4);

        let after = local(2026, 3, 4, 19, 0);
        let evening = schedule_presets(after)[1].when;
        assert_eq!(evening.day(), 5, "this evening stayed in the past");
        assert_eq!(evening.hour(), 18);
    }

    #[test]
    fn monday_morning_is_never_today() {
        // Opening the picker on a Monday offers next week's Monday, not the
        // one already underway — scheduling into a morning that has started
        // is scheduling into the past.
        let monday = local(2026, 3, 2, 10, 0);
        assert_eq!(monday.weekday(), chrono::Weekday::Mon);
        let next = schedule_presets(monday)[3].when;
        assert_eq!(next.weekday(), chrono::Weekday::Mon);
        assert_eq!(next.day(), 9, "Monday morning meant this morning");
    }

    #[test]
    fn every_preset_is_in_the_future() {
        // The invariant under all four: a picker that offers a time already
        // gone hands the send queue something it can only fire immediately,
        // which is not what anybody chose.
        for hour in [0, 7, 8, 12, 17, 18, 23] {
            let now = local(2026, 3, 4, hour, 0);
            for preset in schedule_presets(now) {
                assert!(
                    preset.when > now,
                    "{} is not in the future at {hour}:00",
                    preset.label
                );
            }
        }
    }
}
