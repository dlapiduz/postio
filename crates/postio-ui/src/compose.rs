//! What the composer says about itself, shared so both frontends' footers cannot differ.

/// The plain part is not optional: rich mail always sends a `text/plain` alternative. Plain mail is
/// wrapped at 72 columns and flowed (RFC 3676).
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
        assert!(outgoing_shape(true).contains("text/plain"));
    }

    #[test]
    fn plain_says_it_is_flowed_rather_than_just_plain() {
        assert_eq!(outgoing_shape(false), "text/plain, format=flowed");
    }
}

/// One copy, because a second one is a second dialect: a macOS script emitting `<div>`s and `<span
/// style>`s would be narrowed differently by `parse`.
pub const EDITOR_SCRIPT: &str = include_str!("../data/editor.js");

/// `EDITOR_SCRIPT` reads `POSTIO_MARKDOWN` and does not define it: the table is generated from
/// `crate::editor::markdown::SEQUENCES` so no JavaScript copy drifts.
/// Built once and kept.
pub fn editor_script() -> &'static str {
    static SCRIPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SCRIPT.get_or_init(|| format!("{}{EDITOR_SCRIPT}", markdown_table_js()))
}

/// Escaped rather than trusted to stay punctuation, since the markers are written into source.
pub fn markdown_table_js() -> String {
    use crate::editor::markdown::{Trigger, bridged};
    use std::fmt::Write as _;

    let mut out = String::from("const POSTIO_MARKDOWN = [\n");
    for (marker, command, trigger) in bridged() {
        let trigger = match trigger {
            Trigger::LinePrefix => "line_prefix",
            Trigger::Wrapping => "wrapping",
            Trigger::LineComplete | Trigger::WordComplete => {
                unreachable!("the bridge recognises no {trigger:?} row")
            }
        };
        let _ = writeln!(
            out,
            "    {{ marker: \"{}\", command: \"{command}\", trigger: \"{trigger}\" }},",
            marker.replace('\\', "\\\\").replace('"', "\\\""),
        );
    }
    out.push_str("];\n");
    out
}

pub const NO_REMOTE_IMAGE: &str = "There's no image to download in this message";

pub const NOT_ATTACHED_YET: &str = "Not attached yet";

pub const DOWNLOAD_AND_ATTACH: &str = "Download and attach";

pub const DOWNLOADING: &str = "Downloading\u{2026}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteRefusal {
    Unreachable(String),
    TooLarge,
    NotAnImage,
    Offline,
}

pub fn remote_refusal(refusal: &RemoteRefusal, host: &str, key: &str) -> String {
    match refusal {
        RemoteRefusal::Unreachable(reason) => {
            format!("Couldn't download from {host} \u{b7} {reason} \u{b7} Try again {key}")
        }
        RemoteRefusal::TooLarge => {
            "Larger than 10 MB \u{b7} not attached \u{b7} it will be sent as a link".to_owned()
        }
        RemoteRefusal::NotAnImage => {
            format!("{host} sent something that isn't a picture \u{b7} it will be sent as a link")
        }
        RemoteRefusal::Offline => "You're offline \u{b7} press again when you're online".to_owned(),
    }
}

pub fn remote_images_warning(count: usize, key: &str) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!(
            "1 image isn't attached and will be sent as a link \u{b7} {key} to attach"
        )),
        n => Some(format!(
            "{n} images aren't attached and will be sent as links \u{b7} {key} to attach"
        )),
    }
}

pub fn remote_host(href: &str) -> String {
    let rest = href.split_once("://").map_or(href, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

pub fn source_strip(key: &str, narrow: bool) -> String {
    if narrow {
        format!("Markdown \u{b7} {key}")
    } else {
        format!("Editing as Markdown \u{b7} {key} for rich text")
    }
}

pub fn in_markdown_view_for_now(title: &str, key: &str) -> String {
    format!("{title} is in the Markdown view for now \u{b7} {key}")
}

pub fn narrowed_notice(lost: postio_body::markdown::Lost) -> Option<String> {
    let mut parts = Vec::new();
    let mut say = |count: usize, one: &str, many: &str, why: &str| match count {
        0 => {}
        1 => parts.push(format!("1 {one} {why}")),
        n => parts.push(format!("{n} {many} {why}")),
    };
    say(
        lost.cells,
        "table cell was dropped:",
        "table cells were dropped:",
        "their rows are longer than the header",
    );
    say(
        lost.links,
        "link was kept as its words:",
        "links were kept as their words:",
        "mail links only to http, https and mailto",
    );
    say(
        lost.images,
        "image was kept as its words:",
        "images were kept as their words:",
        "it names neither an attachment nor the web",
    );
    (!parts.is_empty()).then(|| parts.join("; "))
}

#[cfg(test)]
mod remote_words {
    use super::*;

    #[test]
    fn each_outcome_says_what_happens_to_the_image() {
        assert_eq!(
            remote_refusal(&RemoteRefusal::TooLarge, "example.com", "ctrl+alt+i"),
            "Larger than 10 MB \u{b7} not attached \u{b7} it will be sent as a link"
        );
        assert_eq!(
            remote_refusal(&RemoteRefusal::NotAnImage, "example.com", "ctrl+alt+i"),
            "example.com sent something that isn't a picture \u{b7} it will be sent as a link"
        );
        assert_eq!(
            remote_refusal(
                &RemoteRefusal::Unreachable("it answered 404".into()),
                "example.com",
                "ctrl+alt+i"
            ),
            "Couldn't download from example.com \u{b7} it answered 404 \u{b7} Try again ctrl+alt+i"
        );
        assert_eq!(
            remote_refusal(&RemoteRefusal::Offline, "example.com", "ctrl+alt+i"),
            "You're offline \u{b7} press again when you're online"
        );
    }

    #[test]
    fn the_warning_counts_what_will_go_as_a_link() {
        assert_eq!(remote_images_warning(0, "ctrl+alt+i"), None);
        assert_eq!(
            remote_images_warning(1, "ctrl+alt+i").as_deref(),
            Some("1 image isn't attached and will be sent as a link \u{b7} ctrl+alt+i to attach")
        );
        assert_eq!(remote_host("https://example.com/x.png"), "example.com");
    }
}

#[cfg(test)]
mod source_words {
    use super::*;

    #[test]
    fn the_strip_shortens_where_it_is_narrow() {
        assert_eq!(
            source_strip("ctrl+alt+s", false),
            "Editing as Markdown \u{b7} ctrl+alt+s for rich text"
        );
        assert_eq!(
            source_strip("ctrl+alt+s", true),
            "Markdown \u{b7} ctrl+alt+s"
        );
    }

    #[test]
    fn a_command_waiting_for_the_rich_view_says_where_it_is() {
        assert_eq!(
            in_markdown_view_for_now("Strikethrough", "ctrl+alt+s"),
            "Strikethrough is in the Markdown view for now \u{b7} ctrl+alt+s"
        );
    }

    #[test]
    fn the_notice_names_what_narrowed() {
        use postio_body::markdown::Lost;
        assert_eq!(narrowed_notice(Lost::default()), None);
        assert_eq!(
            narrowed_notice(Lost {
                cells: 2,
                ..Lost::default()
            })
            .as_deref(),
            Some("2 table cells were dropped: their rows are longer than the header")
        );
    }
}

#[cfg(test)]
mod editor_script_tests {
    use super::EDITOR_SCRIPT;

    /// A frontend handed the body alone gets a `ReferenceError` on the first keystroke, and the
    /// WebView hides it.
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
    fn every_command_a_sequence_reaches_is_one_the_script_can_run_and_report() {
        // A row with no script entry consumes the marker and formats nothing; one never reflected
        // leaves a toolbar toggle off (#1485).
        for (marker, command, _) in crate::editor::markdown::bridged() {
            assert!(
                EDITOR_SCRIPT.contains(&format!("\n    {command}: [")),
                "the script cannot run {command}, which {:?} reaches",
                marker
            );
            assert!(
                EDITOR_SCRIPT.contains(&format!("active.push('{command}')")),
                "the script never reports {command} as in force"
            );
        }
    }
}

/// Written once for both frontends: different scripts for `bold` would produce different markup.
/// `None` for a command that is not a mark.
/// Every script ends by dispatching `input`, because `execCommand` does not always raise one and
/// the host's record updates only on it.
pub fn mark_script(command: &str) -> Option<String> {
    let body = match command {
        "bold" => "document.execCommand('bold');",
        "italic" => "document.execCommand('italic');",
        "bullet_list" => "document.execCommand('insertUnorderedList');",
        "numbered_list" => "document.execCommand('insertOrderedList');",
        // `formatBlock` does not toggle; the toggle is ours.
        "heading_1" => {
            "if (document.queryCommandValue('formatBlock') === 'h1') { \
                 document.execCommand('formatBlock', false, 'p'); \
             } else { \
                 document.execCommand('formatBlock', false, 'h1'); \
             }"
        }
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

/// `None` when `postio_body::Href` refuses `href`; checked here so the composer can say so rather
/// than have the link vanish at the next parse.
pub fn link_script(href: &str) -> Option<String> {
    postio_body::Href::parse(href)?;
    // Escaped although `Href::parse` refuses control characters: it is still somebody's input in a
    // script literal.
    let escaped = href.replace('\\', "\\\\").replace('\'', "\\'");
    Some(format!(
        "document.execCommand('createLink', false, '{escaped}'); \
         document.dispatchEvent(new Event('input'));"
    ))
}

/// `None` for an id `postio_body::ContentId` refuses, so only a part of this message can become an
/// image source. Written in the `postio-cid:` form, which the send path rewrites to `cid:`.
/// The caret fallback matters: a picture can be the first gesture into a fresh body, and
/// `insertHTML` does nothing without a selection.
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
        for command in [
            "bold",
            "italic",
            "heading_1",
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
        assert_eq!(mark_script("archive"), None);
        assert_eq!(mark_script("insert_link"), None, "link needs an href");
    }

    #[test]
    fn every_script_raises_the_event_the_host_records_on() {
        // `execCommand` does not always raise `input` (bold does, `insertUnorderedList` does not);
        // a mark that skipped it would be lost on save.
        for command in ["bold", "bullet_list", "heading_1", "quote_block"] {
            let script = mark_script(command).expect("a script");
            assert!(
                script.contains("new Event('input')"),
                "{command} applies without telling the host: {script}"
            );
        }
    }

    #[test]
    fn quoting_toggles_rather_than_only_applying() {
        let script = mark_script("quote_block").expect("a script");
        assert!(script.contains("queryCommandValue"), "no toggle: {script}");
        assert!(
            script.contains("'p'"),
            "nothing to toggle back to: {script}"
        );
    }

    #[test]
    fn a_heading_toggles_back_to_a_paragraph_rather_than_only_applying() {
        let script = mark_script("heading_1").expect("a script");
        assert!(
            script.contains("queryCommandValue('formatBlock') === 'h1'"),
            "no toggle: {script}"
        );
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
        let script = link_script("https://example.com/a'b").expect("a script");
        assert!(script.contains("a\\'b"), "unescaped quote: {script}");
    }

    #[test]
    fn an_image_is_inserted_in_the_editing_scheme_at_the_caret() {
        let script = image_script("0a1b@postio.invalid", "inline-image.png").expect("a script");
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
        let script = image_script("a@postio.invalid", "x").expect("a script");
        assert!(script.contains("rangeCount === 0"), "{script}");
    }

    #[test]
    fn an_id_that_could_be_read_as_a_url_has_no_script() {
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

/// `None` for the ordinary case: a banner that is always there is one nobody reads.
/// The count says which field ("1 To, 41 Cc") because a reply-all to a large list looks like a
/// reply until sent.
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
        assert_eq!(recipient_summary(1, 0, 0), None);
        assert_eq!(recipient_summary(0, 0, 0), None);
    }

    #[test]
    fn a_reply_all_to_a_list_says_which_field_the_crowd_is_on() {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulePreset {
    pub label: &'static str,
    pub when: chrono::DateTime<chrono::Local>,
}

/// Shared with the other composers: two frontends deciding what "tomorrow morning" means would send
/// mail at the wrong hour.
pub fn schedule_presets(now: chrono::DateTime<chrono::Local>) -> [SchedulePreset; 4] {
    crate::schedule::schedule_presets(now).map(|(label, when)| SchedulePreset { label, when })
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
        let now = local(2026, 3, 4, 9, 30);
        assert_eq!(schedule_presets(now)[0].when, now + Duration::hours(1));
    }

    #[test]
    fn this_evening_rolls_to_tomorrow_once_it_has_gone() {
        let before = local(2026, 3, 4, 12, 0);
        assert_eq!(schedule_presets(before)[1].when.day(), 4);

        let after = local(2026, 3, 4, 19, 0);
        let evening = schedule_presets(after)[1].when;
        assert_eq!(evening.day(), 5, "this evening stayed in the past");
        assert_eq!(evening.hour(), 18);
    }

    #[test]
    fn monday_morning_is_never_today() {
        let monday = local(2026, 3, 2, 10, 0);
        assert_eq!(monday.weekday(), chrono::Weekday::Mon);
        let next = schedule_presets(monday)[3].when;
        assert_eq!(next.weekday(), chrono::Weekday::Mon);
        assert_eq!(next.day(), 9, "Monday morning meant this morning");
    }

    #[test]
    fn every_preset_is_in_the_future() {
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

pub fn kept_note(drafts_key: Option<&str>) -> String {
    match drafts_key {
        Some(key) => format!("Draft saved to Drafts ({key})"),
        None => "Draft saved to Drafts".to_owned(),
    }
}

pub fn title(kind: postio_model::DraftKind) -> &'static str {
    match kind {
        postio_model::DraftKind::New => "New message",
        postio_model::DraftKind::Reply => "Reply",
        postio_model::DraftKind::ReplyAll => "Reply to all",
        postio_model::DraftKind::Forward => "Forward",
    }
}

/// `None` when the body is local; otherwise the person should not send believing the quote is
/// there.
pub fn unquoted_note(source: &postio_model::Message) -> Option<&'static str> {
    (!source.sync.body_state.has_body()).then_some("Original still downloading \u{2014} not quoted")
}

pub fn summary(draft: &postio_model::Draft) -> String {
    let words = draft
        .body
        .text
        .as_deref()
        .map(|text| postio_model::signature::split(text).0)
        .unwrap_or_default()
        .split_whitespace()
        .count();
    let kind = if draft.body.html.is_some() {
        "Rich text"
    } else {
        "Plain text"
    };
    let words = match words {
        1 => "1 word".to_owned(),
        count => format!("{count} words"),
    };
    format!("{kind} \u{b7} {words}")
}

pub fn subtitle(summary: &str, note: &str) -> String {
    if note.is_empty() {
        summary.to_owned()
    } else {
        format!("{summary} \u{b7} {note}")
    }
}

pub fn saved_at(at: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "Draft saved locally {}",
        at.with_timezone(&chrono::Local).format("%H:%M")
    )
}

pub const NOT_SAVED: &str = "draft is in the composer only";

pub const IN_OUTBOX: &str = "waiting in the Outbox";

/// A draft with an id is a row in Drafts or the Outbox, so it never says "in the composer only"
/// (#1759).
pub fn resumed_status(draft: &postio_model::Draft) -> String {
    use postio_model::DraftState;
    if !draft.id.is_assigned() {
        return NOT_SAVED.to_owned();
    }
    match draft.state {
        DraftState::Queued | DraftState::Sending => IN_OUTBOX.to_owned(),
        _ => saved_at(draft.updated_at),
    }
}

/// A part with no `part_id` is a file the composer is still writing, not a carried one (#1690).
pub fn carried_sections(attachments: &[postio_model::Attachment]) -> Vec<String> {
    attachments
        .iter()
        .filter(|attachment| attachment.blob_id.is_none())
        .filter_map(|attachment| attachment.part_id.clone())
        .collect()
}

pub fn attachment_note(size: &str, large: bool, downloading: bool) -> String {
    match (large, downloading) {
        (false, false) => size.to_owned(),
        (true, false) => format!("{size} \u{2014} large"),
        (false, true) => format!("{size} \u{2014} downloading"),
        (true, true) => format!("{size} \u{2014} large, downloading"),
    }
}

/// The full "if no reply" is the verb's tooltip and accessible name (`remind_meaning`); the row has
/// room for the day only.
pub fn remind_words(at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match at {
        Some(at) => format!(
            "Remind \u{b7} {}",
            at.with_timezone(&chrono::Local).format("%a %-d %b")
        ),
        None => "Remind".to_owned(),
    }
}

pub fn remind_meaning(at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match at {
        Some(at) => format!(
            "Remind if no reply \u{b7} {}",
            at.with_timezone(&chrono::Local).format("%a %-d %b")
        ),
        None => "Remind if no reply".to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteFold {
    pub written: String,
    pub quote: String,
    pub summary: String,
}

/// `None` for a body that ends in no quote.
pub fn fold_quote(body: &str) -> Option<QuoteFold> {
    let mut starts = Vec::new();
    let mut at = 0;
    for line in body.split('\n') {
        starts.push((at, line));
        at += line.len() + 1;
    }
    // Blank lines after the quote are the quote's: a body often ends in a newline.
    let trailing = starts
        .iter()
        .rev()
        .take_while(|(_, line)| line.trim().is_empty())
        .count();
    let quoted = starts
        .iter()
        .rev()
        .skip(trailing)
        .take_while(|(_, line)| line.starts_with('>'))
        .count();
    if quoted == 0 {
        return None;
    }
    let (start, attribution) = *starts.iter().rev().nth(trailing + quoted)?;
    let said = attribution.trim_end().strip_suffix(':')?;
    if !said.ends_with("wrote") {
        return None;
    }
    let lines = match quoted {
        1 => "1 quoted line".to_owned(),
        count => format!("{count} quoted lines"),
    };
    Some(QuoteFold {
        written: body[..start].to_owned(),
        quote: body[start..].to_owned(),
        summary: format!("{said} \u{b7} {lines}"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewerMessage {
    pub who: crate::author::Author,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OlderReplyFacts {
    pub number: usize,
    pub total: usize,
    pub who: crate::author::Author,
    pub at: chrono::DateTime<chrono::Utc>,
    pub newer: Vec<NewerMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarAction {
    pub label: String,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OlderReplyView {
    pub title: String,
    pub detail: String,
    pub actions: [BarAction; 2],
    pub newer_shown: bool,
}

fn newer_count_words(count: usize) -> String {
    match count {
        1 => "One newer message".to_owned(),
        2 => "Two newer messages".to_owned(),
        n => format!("{n} newer messages"),
    }
}

fn clock_and_day(at: chrono::DateTime<chrono::Utc>) -> (String, String) {
    let local = at.with_timezone(&chrono::Local);
    (
        local.format("%H:%M").to_string(),
        local.format("%A").to_string(),
    )
}

pub fn older_reply(
    facts: &OlderReplyFacts,
    newer_shown: bool,
    keymap: &postio_core::Keymap,
) -> OlderReplyView {
    let named: Vec<String> = facts
        .newer
        .iter()
        .map(|message| {
            format!(
                "{} at {}",
                message.who.mid_sentence(),
                clock_and_day(message.at).0
            )
        })
        .collect();
    let newer = match (named.len(), named.split_last()) {
        (0, _) | (_, None) => String::new(),
        (1, _) => format!("{}: {}.", newer_count_words(1), named[0]),
        (n, Some((last, rest))) if n <= 3 => {
            format!("{}: {} and {last}.", newer_count_words(n), rest.join(", "))
        }
        (n, Some((last, _))) => format!("{}, the latest {last}.", newer_count_words(n)),
    };
    let quoted = format!(
        "Your reply quotes {} {} message and threads under it.",
        facts.who.possessive(),
        clock_and_day(facts.at).1
    );
    OlderReplyView {
        title: format!(
            "You\u{2019}re replying to message {} of {}, not the latest",
            facts.number, facts.total
        ),
        detail: format!("{newer} {quoted}"),
        actions: [
            BarAction {
                label: if newer_shown {
                    "Hide them"
                } else {
                    "Show them"
                }
                .to_owned(),
                key: crate::hints::key(keymap, postio_core::CommandId::ShowNewer),
            },
            BarAction {
                label: "Reply to latest".to_owned(),
                key: crate::hints::key(keymap, postio_core::CommandId::ReplyToLatest),
            },
        ],
        newer_shown,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChooserCard {
    pub form: postio_model::ForwardForm,
    pub key: Option<String>,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChooserView {
    pub heading: String,
    pub cards: Vec<ChooserCard>,
    pub selected: postio_model::ForwardForm,
}

pub fn forward_chooser(
    selected: postio_model::ForwardForm,
    messages: usize,
    of: Option<(&crate::author::Author, chrono::DateTime<chrono::Utc>)>,
    keymap: &postio_core::Keymap,
) -> ChooserView {
    use postio_core::CommandId;
    use postio_model::ForwardForm;
    let this = match of {
        Some((who, at)) => format!(
            "Only {} {} message, quoted as usual.",
            who.possessive(),
            clock_and_day(at).1
        ),
        None => "Only this message, quoted as usual.".to_owned(),
    };
    let card = |form, command, title: &str, detail: String| ChooserCard {
        form,
        key: crate::hints::key(keymap, command),
        title: title.to_owned(),
        detail,
    };
    ChooserView {
        heading: "What to forward".to_owned(),
        cards: vec![
            card(
                ForwardForm::This,
                CommandId::ForwardWhat1,
                "This message",
                this,
            ),
            card(
                ForwardForm::Thread,
                CommandId::ForwardWhat2,
                "Whole thread",
                format!(
                    "All {messages} messages, oldest first, in one readable message. \
                     Attachments included."
                ),
            ),
            card(
                ForwardForm::Attachments,
                CommandId::ForwardWhat3,
                "As attachments",
                "Each message as an .eml file, for someone who needs the originals.".to_owned(),
            ),
        ],
        selected,
    }
}

pub fn forward_not_local(form: postio_model::ForwardForm, missing: usize) -> String {
    let which = match missing {
        1 => "1 message isn\u{2019}t".to_owned(),
        n => format!("{n} messages aren\u{2019}t"),
    };
    let what = match form {
        postio_model::ForwardForm::Attachments => "they can\u{2019}t be forwarded as attachments",
        _ => "the thread can\u{2019}t be forwarded whole",
    };
    format!("{which} on this machine yet, so {what}. Nothing was fetched.")
}

#[cfg(test)]
mod thread_words_tests {
    use crate::author::Author;
    use chrono::{Local, TimeZone, Utc};
    use postio_core::Keymap;
    use postio_model::ForwardForm;

    use super::*;

    fn at(hour: u32, minute: u32) -> chrono::DateTime<Utc> {
        Local
            .with_ymd_and_hms(2026, 9, 24, hour, minute, 0)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn the_bar_says_which_message_and_names_the_newer_ones() {
        let facts = OlderReplyFacts {
            number: 4,
            total: 6,
            who: Author::Named("Ben".into()),
            at: at(9, 0),
            newer: vec![
                NewerMessage {
                    who: Author::Named("Lena".into()),
                    at: at(15, 22),
                },
                NewerMessage {
                    who: Author::Named("Ben".into()),
                    at: at(16, 40),
                },
            ],
        };
        let view = older_reply(&facts, false, Keymap::defaults());
        assert_eq!(
            view.title,
            "You\u{2019}re replying to message 4 of 6, not the latest"
        );
        assert_eq!(
            view.detail,
            "Two newer messages: Lena at 15:22 and Ben at 16:40. Your reply quotes \
             Ben\u{2019}s Thursday message and threads under it."
        );
        assert_eq!(view.actions[0].label, "Show them");
        assert_eq!(
            view.actions[0].key,
            crate::hints::key(Keymap::defaults(), postio_core::CommandId::ShowNewer)
        );
        assert!(
            view.actions[0]
                .key
                .as_deref()
                .is_some_and(|k| k.ends_with("shift+Down"))
        );
        assert_eq!(view.actions[1].label, "Reply to latest");
        assert!(
            view.actions[1]
                .key
                .as_deref()
                .is_some_and(|k| k.ends_with("+e"))
        );
        let shown = older_reply(&facts, true, Keymap::defaults());
        assert_eq!(shown.actions[0].label, "Hide them");
    }

    #[test]
    fn the_bar_and_the_chooser_name_every_author_in_its_place() {
        let forms = [
            (Author::You, "your", "you"),
            (Author::Named("Ben".into()), "Ben\u{2019}s", "Ben"),
            (Author::Someone, "someone\u{2019}s", "someone"),
        ];
        for (who, possessive, mid) in forms {
            let facts = OlderReplyFacts {
                number: 1,
                total: 3,
                who: who.clone(),
                at: at(9, 0),
                newer: vec![NewerMessage {
                    who: who.clone(),
                    at: at(15, 22),
                }],
            };
            assert_eq!(
                older_reply(&facts, false, Keymap::defaults()).detail,
                format!(
                    "One newer message: {mid} at 15:22. Your reply quotes {possessive} \
                     Thursday message and threads under it."
                )
            );
            let chooser = forward_chooser(
                ForwardForm::This,
                3,
                Some((&who, at(9, 0))),
                Keymap::defaults(),
            );
            assert_eq!(
                chooser.cards[0].detail,
                format!("Only {possessive} Thursday message, quoted as usual.")
            );
        }
    }

    #[test]
    fn many_newer_messages_name_only_the_latest() {
        let newer = (0..5)
            .map(|n| NewerMessage {
                who: Author::Named("Ana".into()),
                at: at(10 + n, 0),
            })
            .collect();
        let facts = OlderReplyFacts {
            number: 2,
            total: 8,
            who: Author::Named("Ben".into()),
            at: at(9, 0),
            newer,
        };
        let view = older_reply(&facts, false, Keymap::defaults());
        assert!(
            view.detail
                .starts_with("5 newer messages, the latest Ana at 14:00."),
            "{}",
            view.detail
        );
    }

    #[test]
    fn the_chooser_has_three_numbered_cards_and_the_choice() {
        let view = forward_chooser(
            ForwardForm::This,
            6,
            Some((&Author::Named("Ben".into()), at(9, 0))),
            Keymap::defaults(),
        );
        assert_eq!(view.heading, "What to forward");
        let titles: Vec<_> = view.cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["This message", "Whole thread", "As attachments"]);
        let keys: Vec<_> = view.cards.iter().map(|c| c.key.as_deref()).collect();
        assert_eq!(keys, [Some("1"), Some("2"), Some("3")]);
        assert_eq!(
            view.cards[0].detail,
            "Only Ben\u{2019}s Thursday message, quoted as usual."
        );
        assert!(
            view.cards[1]
                .detail
                .starts_with("All 6 messages, oldest first")
        );
        assert_eq!(view.selected, ForwardForm::This);
    }

    #[test]
    fn a_refusal_counts_the_messages_that_are_not_here() {
        assert_eq!(
            forward_not_local(ForwardForm::Thread, 2),
            "2 messages aren\u{2019}t on this machine yet, so the thread can\u{2019}t be \
             forwarded whole. Nothing was fetched."
        );
        assert!(forward_not_local(ForwardForm::Attachments, 1).starts_with("1 message isn"));
    }
}

#[cfg(test)]
mod frame_tests {
    use chrono::{Local, TimeZone, Utc};
    use postio_model::{AccountId, Draft, DraftKind};

    use super::*;

    #[test]
    fn a_replys_quote_folds_under_what_is_written() {
        let body = "\n\nOn 2026-09-26, Lena Park wrote:\n> v3 is up.\n>\n> Comments welcome.";
        let fold = fold_quote(body).expect("a reply's quote folds");
        assert_eq!(fold.written, "\n\n");
        assert_eq!(
            fold.summary,
            "On 2026-09-26, Lena Park wrote \u{b7} 3 quoted lines"
        );
        assert_eq!(
            format!("{}{}", fold.written, fold.quote),
            body,
            "nothing lost"
        );

        let trailing = "\n\nOn 2026-09-26, Ada wrote:\n> Hi\n";
        let fold = fold_quote(trailing).expect("a trailing newline is still the quote's");
        assert_eq!(fold.written, "\n\n");
        assert!(fold.summary.ends_with("1 quoted line"), "{}", fold.summary);
        assert_eq!(format!("{}{}", fold.written, fold.quote), trailing);

        let one = fold_quote("Thanks.\nOn 2026-09-26, Ada wrote:\n> Hi").expect("folds");
        assert_eq!(one.written, "Thanks.\n");
        assert!(one.summary.ends_with("1 quoted line"), "{}", one.summary);
    }

    #[test]
    fn a_body_that_ends_in_no_quote_folds_nothing() {
        assert_eq!(fold_quote(""), None);
        assert_eq!(fold_quote("Hi Ada,\n\nSee you Monday."), None);
        assert_eq!(
            fold_quote("On 2026-09-26, Ada wrote:\n> Hi\nAnd below it."),
            None
        );
        assert_eq!(fold_quote("Hi\nOn 2026-09-26, Ada wrote:"), None);
        assert_eq!(fold_quote("> a quote\n> of my own"), None);
    }

    #[test]
    fn the_subtitle_counts_the_words_that_will_be_sent_and_says_which_kind() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        assert_eq!(summary(&draft), "Plain text \u{b7} 0 words");
        draft.body.text = Some("Hi Ada,\n\nThe sheet is attached.".to_owned());
        assert_eq!(summary(&draft), "Plain text \u{b7} 6 words");
        draft.body.text = Some("One".to_owned());
        assert_eq!(summary(&draft), "Plain text \u{b7} 1 word");
        draft.body.html = Some("<p><b>One</b></p>".to_owned());
        assert_eq!(summary(&draft), "Rich text \u{b7} 1 word");
    }

    #[test]
    fn a_message_without_its_body_says_it_is_not_quoted() {
        use postio_model::{BodyState, Message};
        let mut source = Message::new(
            AccountId::UNASSIGNED,
            postio_model::ids::MailboxId::UNASSIGNED,
            chrono::Utc::now(),
        );
        for state in [BodyState::NotFetched, BodyState::HeadersOnly] {
            source.sync.body_state = state;
            assert!(unquoted_note(&source).is_some(), "{state:?} said nothing");
        }
        for state in [BodyState::Partial, BodyState::Full] {
            source.sync.body_state = state;
            assert_eq!(unquoted_note(&source), None, "{state:?} has a body");
        }
    }

    #[test]
    fn the_signature_is_not_counted_as_written() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.body.text = Some("Looks good.\n\n-- \nAda Norwood\nExample Corp".to_owned());
        assert_eq!(summary(&draft), "Plain text \u{b7} 2 words");
    }

    #[test]
    fn the_subtitle_says_what_will_be_sent_then_what_happened() {
        assert_eq!(
            subtitle("Plain text \u{b7} 0 words", ""),
            "Plain text \u{b7} 0 words"
        );
        assert_eq!(
            subtitle("Plain text \u{b7} 58 words", "Draft saved locally 16:12"),
            "Plain text \u{b7} 58 words \u{b7} Draft saved locally 16:12"
        );
    }

    #[test]
    fn remind_is_the_message_dialogs_word_and_says_its_day() {
        assert_eq!(remind_words(None), "Remind");
        assert_eq!(remind_meaning(None), "Remind if no reply");
        let at = Local
            .with_ymd_and_hms(2026, 9, 29, 9, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(remind_words(Some(at)), "Remind \u{b7} Tue 29 Sep");
        assert_eq!(
            remind_meaning(Some(at)),
            "Remind if no reply \u{b7} Tue 29 Sep"
        );
    }

    #[test]
    fn the_header_names_a_composition_as_the_screens_do() {
        assert_eq!(title(DraftKind::New), "New message");
        assert_eq!(title(DraftKind::ReplyAll), "Reply to all");
    }

    #[test]
    fn a_reopened_draft_says_where_it_lives_not_that_it_is_unsaved() {
        use postio_model::{DraftId, DraftState};
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        assert_eq!(resumed_status(&draft), "draft is in the composer only");

        draft.id = DraftId::new(7);
        draft.updated_at = Utc.with_ymd_and_hms(2026, 3, 4, 16, 12, 0).unwrap();
        assert_eq!(resumed_status(&draft), saved_at(draft.updated_at));
        assert!(resumed_status(&draft).starts_with("Draft saved locally "));

        for state in [DraftState::Queued, DraftState::Sending] {
            draft.state = state;
            assert_eq!(resumed_status(&draft), "waiting in the Outbox");
        }
    }

    #[test]
    fn a_forward_asks_only_for_the_carried_parts_it_does_not_have() {
        use postio_model::{Attachment, MessageId};
        let part = |id: Option<&str>, local: bool| {
            let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 9);
            attachment.part_id = id.map(str::to_owned);
            if local {
                attachment.blob_id = Some(postio_model::ids::BlobId::new("0".repeat(64)));
            }
            attachment
        };
        let attachments = [
            part(Some("2"), false),
            part(Some("3"), true),
            part(None, false),
            part(Some("4"), false),
        ];
        assert_eq!(carried_sections(&attachments), vec!["2", "4"]);
        assert!(carried_sections(&[]).is_empty());
    }

    #[test]
    fn an_attachment_note_says_large_and_downloading_after_the_size() {
        assert_eq!(attachment_note("12 KB", false, false), "12 KB");
        assert_eq!(
            attachment_note("30 MB", true, false),
            "30 MB \u{2014} large"
        );
        assert_eq!(
            attachment_note("12 KB", false, true),
            "12 KB \u{2014} downloading"
        );
        assert_eq!(
            attachment_note("30 MB", true, true),
            "30 MB \u{2014} large, downloading"
        );
    }

    #[test]
    fn the_kept_note_names_where_the_draft_went_and_the_key_there() {
        assert_eq!(kept_note(Some("g d")), "Draft saved to Drafts (g d)");
        assert_eq!(kept_note(None), "Draft saved to Drafts");
    }
}
