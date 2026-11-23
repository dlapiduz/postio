//! A message's MIME tree and the rules for showing and saving one part, read from `BODYSTRUCTURE`
//! metadata only (ADR 0019).
//! Nothing here can fetch a part: nothing downloads until the user asks.

use postio_model::Attachment;
use postio_model::ids::AttachmentId;

use crate::format::human_size;

/// Resolves a `Content-ID` (no `cid:` prefix, no angle brackets) to bytes and MIME type.
/// Synchronous and local: a `cid:` that reached the network would be a tracking pixel through the
/// back door (ADR 0019).
pub trait BlobSource {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)>;

    /// Names which message the reference belongs to. The default ignores `scope`; a thread source
    /// must override it, and an unknown scope must resolve to nothing.
    fn resolve_in(&self, scope: Option<&str>, content_id: &str) -> Option<(Vec<u8>, String)> {
        let _ = scope;
        self.resolve(content_id)
    }
}

impl<F: Fn(&str) -> Option<(Vec<u8>, String)>> BlobSource for F {
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)> {
        self(content_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The IMAP part id — `2.1`. Empty for the synthetic root.
    pub part_id: String,
    pub depth: usize,
    pub mime: String,
    pub filename: Option<String>,
    /// Size in bytes as the server declared it. `0` for a container.
    pub size: u64,
    pub downloaded: bool,
    pub last: bool,
    /// The attachment row this came from; `None` for the synthetic root.
    pub attachment: Option<AttachmentId>,
    pub content_id: Option<String>,
    /// Shown in place rather than offered: `Content-Disposition: inline` or a `Content-ID`.
    /// A label, never a filter, so an inline part can still be seen and saved (#751).
    pub inline: bool,
}

impl Node {
    pub fn label(&self) -> &str {
        match self.filename.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name,
            _ => &self.mime,
        }
    }

    pub fn is_leaf(&self) -> bool {
        !self.mime.starts_with("multipart/") && self.attachment.is_some()
    }
}

/// Flattened in walk order; ids sort as number paths, so `2.10` follows `2.9`.
/// `root` is passed in, and a message with no parts still gets its root node.
pub fn tree(root: &str, parts: &[Attachment]) -> Vec<Node> {
    let mut ordered: Vec<(Vec<u32>, &Attachment)> = parts
        .iter()
        .map(|part| (path_of(part), part))
        .filter(|(path, _)| !path.is_empty())
        .collect();
    ordered.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut nodes = Vec::with_capacity(ordered.len() + 1);
    nodes.push(Node {
        part_id: String::new(),
        depth: 0,
        mime: root.to_owned(),
        filename: None,
        size: parts.iter().map(|part| part.size).sum(),
        downloaded: false,
        last: ordered.is_empty(),
        attachment: None,
        content_id: None,
        inline: false,
    });

    for (index, (path, part)) in ordered.iter().enumerate() {
        // The last child of this parent, not the last row overall.
        let last = ordered.get(index + 1).is_none_or(|(next, _)| {
            next.len() < path.len() || next[..path.len() - 1] != path[..path.len() - 1]
        });
        nodes.push(Node {
            part_id: part.part_id.clone().unwrap_or_default(),
            depth: path.len(),
            mime: part.mime_type.clone(),
            filename: part.filename.clone(),
            size: part.size,
            downloaded: part.blob_id.is_some(),
            last,
            attachment: Some(part.id),
            content_id: part.content_id.clone(),
            inline: part.disposition == postio_model::Disposition::Inline
                || part.content_id.is_some(),
        });
    }
    nodes
}

/// A part with no numeric id is dropped: nothing can be fetched for it.
fn path_of(part: &Attachment) -> Vec<u32> {
    let Some(id) = part.part_id.as_deref() else {
        return Vec::new();
    };
    let mut path = Vec::new();
    for segment in id.split('.') {
        match segment.parse::<u32>() {
            Ok(number) => path.push(number),
            Err(_) => return Vec::new(),
        }
    }
    path
}

/// `stored` is `None` for rows synced before the column existed and for drafts; the fallback
/// guesses from parts and bodies, so a `multipart/related` reads as `multipart/mixed`.
pub fn root_type(
    stored: Option<&str>,
    body: &postio_model::MessageBody,
    parts: &[Attachment],
) -> String {
    if let Some(content_type) = stored {
        return content_type.to_owned();
    }
    match (parts.is_empty(), body.text.is_some(), body.html.is_some()) {
        (false, _, _) => "multipart/mixed".to_owned(),
        (true, true, true) => "multipart/alternative".to_owned(),
        (true, false, true) => "text/html".to_owned(),
        _ => "text/plain".to_owned(),
    }
}

pub fn prefix(node: &Node) -> String {
    if node.depth == 0 {
        return String::new();
    }
    let indent = "  ".repeat(node.depth - 1);
    let branch = if node.last { "└ " } else { "├ " };
    format!("{indent}{branch}")
}

/// Counts parts, not nodes: the root is the message.
pub fn summary(nodes: &[Node]) -> String {
    let Some(root) = nodes.first() else {
        return String::new();
    };
    let parts = nodes.len().saturating_sub(1);
    let count = match parts {
        1 => "1 part".to_string(),
        many => format!("{many} parts"),
    };
    format!("{} · {count} · {}", root.mime, human_size(root.size))
}

pub fn detail(node: &Node) -> String {
    if node.depth == 0 || node.size == 0 {
        return node.mime.clone();
    }
    format!("{} · {}", node.mime, human_size(node.size))
}

pub const NOT_FETCHED: &str =
    "Described by the server, not downloaded. Nothing here has touched the network.";

pub const CONTAINER: &str = "A container for the parts below it. Nothing to save.";

/// A held-back HTML part is also downloaded; say why it is held back, not "already downloaded".
/// Shared so every frontend explains the oversized inline image (#751).
pub fn note(node: &Node, remote_images: u32, trackers: u32) -> String {
    if let Some(held_back) = held_back_note(&node.mime, remote_images, trackers) {
        return held_back;
    }
    if !node.is_leaf() {
        return CONTAINER.to_string();
    }
    if !node.downloaded {
        return NOT_FETCHED.to_string();
    }
    format!(
        "{} · already downloaded",
        node.filename.clone().unwrap_or_else(|| node.mime.clone())
    )
}

/// Only markup parts: an image cannot phone home. "Render once" is offered exactly when this is
/// `Some`.
pub fn held_back_note(mime: &str, remote_images: u32, trackers: u32) -> Option<String> {
    if !mime.trim().eq_ignore_ascii_case("text/html") {
        return None;
    }
    if remote_images + trackers == 0 {
        return None;
    }
    let images = match remote_images {
        0 => String::new(),
        1 => "1 remote image".to_string(),
        many => format!("{many} remote images"),
    };
    // "Likely": a size heuristic cannot tell a beacon from a small picture, and it under-counts on
    // purpose (#174).
    let trackers = match trackers {
        0 => String::new(),
        1 => "1 likely tracker".to_string(),
        many => format!("{many} likely trackers"),
    };
    let what = match (images.is_empty(), trackers.is_empty()) {
        (false, false) => format!("{images} and {trackers}"),
        (false, true) => images,
        (true, false) => trackers,
        (true, true) => return None,
    };
    Some(format!(
        "HTML part held back — {what} would load. The plain-text part is showing instead."
    ))
}

pub fn spoken(node: &Node) -> String {
    if node.depth == 0 {
        return format!("{}, the whole message", node.mime);
    }
    let name = match node.filename.as_deref() {
        Some(name) if !name.trim().is_empty() => format!("{name}, {}", node.mime),
        _ => node.mime.clone(),
    };
    if !node.is_leaf() {
        return format!("{name}, a container");
    }
    let fetched = if node.downloaded {
        "downloaded"
    } else {
        "not downloaded"
    };
    format!("{name}, {}, {fetched}", human_size(node.size))
}

/// `None` when nothing is left. A sender wrote the name, so nothing in it is trusted.
pub fn clean_file_name(name: &str) -> Option<String> {
    let name = name.trim();
    let cleaned: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' { '-' } else { c })
        // #147: an interior NUL (literal or inside an encoded word) panics gtk-rs's C-string
        // conversion in the file dialog.
        .filter(|c| !c.is_control())
        .collect();
    let cleaned = cleaned.trim_matches(['.', ' '].as_slice()).to_owned();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// The sender's name with separators removed so `../../.bashrc` cannot steer a save; else the part
/// id and a guessed extension.
/// The one sanitiser both frontends share, so no platform misses a separator.
pub fn save_name(node: &Node) -> String {
    if let Some(cleaned) = node.filename.as_deref().and_then(clean_file_name) {
        return cleaned;
    }
    let extension = node
        .mime
        .rsplit_once('/')
        .map(|(_, sub)| sub)
        .unwrap_or("bin");
    let part = if node.part_id.is_empty() {
        "message"
    } else {
        &node.part_id
    };
    format!("part-{part}.{extension}")
}

/// Repeats get `-2`, `-3` before the extension, so "save all" never overwrites a part.
/// Compared without case, so every platform writes the same number of files.
pub fn save_names(nodes: &[Node]) -> Vec<String> {
    let mut taken: Vec<String> = Vec::with_capacity(nodes.len());
    let mut names = Vec::with_capacity(nodes.len());
    for node in nodes {
        let wanted = save_name(node);
        let folded = wanted.to_lowercase();
        let name = if taken.contains(&folded) {
            (2..)
                .map(|nth| suffixed(&wanted, nth))
                .find(|candidate| !taken.contains(&candidate.to_lowercase()))
                // Unreachable: it takes four billion parts of one name.
                .unwrap_or(wanted)
        } else {
            wanted
        };
        taken.push(name.to_lowercase());
        names.push(name);
    }
    names
}

/// The suffix goes before the extension so both files still open.
fn suffixed(name: &str, nth: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem}-{nth}.{extension}"),
        _ => format!("{name}-{nth}"),
    }
}

/// A count, shared so both frontends report a partial save the same way.
pub fn save_all_failure(failed: usize) -> Option<String> {
    if failed == 0 {
        return None;
    }
    Some(format!(
        "{failed} part{} could not be saved",
        if failed == 1 { "" } else { "s" }
    ))
}

pub const SAVE: &str = "Save";

pub const SAVE_ALL: &str = "Save all";

pub fn saved(name: &str) -> String {
    format!("Saved {name}")
}

pub fn saved_all(count: usize, failed: usize) -> String {
    save_all_failure(failed).unwrap_or_else(|| {
        format!(
            "Saved {count} attachment{}",
            if count == 1 { "" } else { "s" }
        )
    })
}

/// Clamps rather than wraps, so the reader keeps their place. A `count` of 0 leaves the cursor at
/// 0.
pub fn step(current: usize, forward: bool, count: usize) -> usize {
    let Some(last) = count.checked_sub(1) else {
        return 0;
    };
    if forward {
        current.saturating_add(1).min(last)
    } else {
        current.saturating_sub(1).min(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::BlobId;
    use postio_model::ids::MessageId;

    fn part(id: &str, mime: &str, size: u64) -> Attachment {
        let mut part = Attachment::new(MessageId::new(1), mime, size);
        part.part_id = Some(id.to_owned());
        part
    }

    fn named(id: &str, mime: &str, size: u64, filename: &str) -> Attachment {
        let mut part = part(id, mime, size);
        part.filename = Some(filename.to_owned());
        part
    }

    fn message() -> Vec<Attachment> {
        vec![
            part("1", "text/plain", 2_100),
            part("2", "text/html", 6 * 1024),
            named("3", "text/x-diff", 11 * 1024, "0001-index.patch"),
            named("4", "image/png", 1_100 * 1024, "cold.png"),
        ]
    }

    fn leaf(mime: &str, filename: Option<&str>) -> Node {
        let mut row = part("1", mime, 10);
        row.filename = filename.map(str::to_owned);
        tree("multipart/mixed", &[row]).remove(1)
    }

    #[test]
    fn the_message_is_the_root_and_the_parts_hang_off_it() {
        let nodes = tree("multipart/mixed", &message());

        assert_eq!(nodes.len(), 5, "four parts and the message itself");
        assert_eq!(nodes[0].mime, "multipart/mixed");
        assert_eq!(nodes[0].depth, 0);
        assert!(nodes[0].attachment.is_none(), "the root is not a part");
        assert!(nodes[1..].iter().all(|node| node.depth == 1));
    }

    #[test]
    fn nesting_comes_out_of_the_part_ids() {
        let nodes = tree(
            "multipart/mixed",
            &[
                part("1", "text/plain", 10),
                part("2", "multipart/alternative", 0),
                part("2.1", "text/plain", 20),
                part("2.2", "text/html", 30),
            ],
        );

        let depths: Vec<usize> = nodes.iter().map(|node| node.depth).collect();
        assert_eq!(depths, [0, 1, 1, 2, 2]);
    }

    #[test]
    fn parts_are_ordered_as_paths_of_numbers_not_as_strings() {
        let nodes = tree(
            "multipart/mixed",
            &[
                part("2.10", "text/plain", 1),
                part("2.9", "text/plain", 1),
                part("2.2", "text/plain", 1),
            ],
        );

        let ids: Vec<&str> = nodes[1..]
            .iter()
            .map(|node| node.part_id.as_str())
            .collect();
        assert_eq!(ids, ["2.2", "2.9", "2.10"], "`2.10` comes after `2.9`");
    }

    #[test]
    fn a_part_with_no_usable_id_is_not_drawn() {
        let mut orphan = part("1", "text/plain", 10);
        orphan.part_id = None;
        let mut nonsense = part("TEXT", "text/plain", 10);
        nonsense.part_id = Some("TEXT".to_owned());

        let nodes = tree("multipart/mixed", &[orphan, nonsense]);
        assert_eq!(nodes.len(), 1, "nothing to fetch means nothing to offer");
    }

    #[test]
    fn a_message_with_no_parts_still_says_what_it_is() {
        let nodes = tree("text/plain", &[]);

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].mime, "text/plain");
        assert_eq!(summary(&nodes), "text/plain · 0 parts · 0 B");
    }

    #[test]
    fn downloaded_is_whether_the_bytes_are_actually_here() {
        let mut fetched = part("1", "image/png", 100);
        fetched.blob_id = Some(BlobId::new("abc"));
        let nodes = tree("multipart/mixed", &[fetched, part("2", "image/png", 100)]);

        assert!(nodes[1].downloaded);
        assert!(!nodes[2].downloaded, "described, not fetched");
    }

    #[test]
    fn a_part_the_body_draws_is_marked_inline_either_way_the_sender_said_so() {
        let mut by_cid = part("1", "image/png", 10);
        by_cid.content_id = Some("logo@example.com".to_owned());
        let mut by_disposition = part("2", "image/png", 10);
        by_disposition.disposition = postio_model::Disposition::Inline;
        let attached = named("3", "application/pdf", 10, "invoice.pdf");

        let nodes = tree("multipart/related", &[by_cid, by_disposition, attached]);

        assert!(nodes[1].inline);
        assert_eq!(nodes[1].content_id.as_deref(), Some("logo@example.com"));
        assert!(nodes[2].inline, "a disposition alone is enough");
        assert!(!nodes[3].inline, "an attachment is not inline");
        assert_eq!(nodes[3].content_id, None);
    }

    #[test]
    fn an_inline_image_too_big_for_the_text_axis_is_a_part_the_panel_explains() {
        // #751: an oversized inline part is still listed, sized and undownloaded, which explains
        // the broken box.
        let mut banner = part("2", "image/png", 4 * 1024 * 1024);
        banner.content_id = Some("banner@example.com".to_owned());
        banner.disposition = postio_model::Disposition::Inline;

        let nodes = tree("multipart/related", &[part("1", "text/html", 900), banner]);
        let node = &nodes[2];

        assert!(
            node.is_leaf(),
            "an inline part is a part like any other: it gets a row and a save"
        );
        assert!(
            node.inline,
            "and it is still labelled as one the body draws"
        );
        assert!(!node.downloaded);
        assert_eq!(detail(node), "image/png · 4.0 MB");
        assert_eq!(spoken(node), "image/png, 4.0 MB, not downloaded");
        assert_eq!(
            note(node, 0, 0),
            NOT_FETCHED,
            "the sentence is the whole explanation for the broken box in the \
             body -- changing the words is fine, leaving the user without \
             them is not"
        );
    }

    #[test]
    fn the_root_type_is_read_when_the_sync_recorded_one() {
        assert_eq!(
            root_type(
                Some("multipart/related"),
                &postio_model::MessageBody::default(),
                &message(),
            ),
            "multipart/related",
            "a stored BODYSTRUCTURE beats any derivation from the columns"
        );
    }

    #[test]
    fn the_root_type_falls_back_to_derivation_when_nothing_is_stored() {
        let with_html = postio_model::MessageBody {
            text: None,
            html: Some("<p>hi</p>".to_owned()),
        };
        assert_eq!(root_type(None, &with_html, &[]), "text/html");
        assert_eq!(
            root_type(None, &postio_model::MessageBody::default(), &[]),
            "text/plain"
        );
        assert_eq!(
            root_type(None, &postio_model::MessageBody::default(), &message()),
            "multipart/mixed",
            "parts mean a multipart, whatever the bodies say"
        );
    }

    #[test]
    fn the_last_child_of_a_branch_closes_it() {
        let nodes = tree("multipart/mixed", &message());
        let drawn: Vec<String> = nodes.iter().map(prefix).collect();

        assert_eq!(drawn, ["", "├ ", "├ ", "├ ", "└ "]);
    }

    #[test]
    fn a_nested_branch_closes_before_its_parents_next_sibling() {
        let nodes = tree(
            "multipart/mixed",
            &[
                part("1", "multipart/alternative", 0),
                part("1.1", "text/plain", 1),
                part("1.2", "text/html", 1),
                part("2", "image/png", 1),
            ],
        );
        let drawn: Vec<String> = nodes.iter().map(prefix).collect();

        assert_eq!(
            drawn,
            ["", "├ ", "  ├ ", "  └ ", "└ "],
            "`1.2` ends its branch even though `2` follows it"
        );
    }

    #[test]
    fn the_summary_counts_parts_and_not_the_message() {
        let nodes = tree("multipart/mixed", &message());
        assert_eq!(summary(&nodes), "multipart/mixed · 4 parts · 1.1 MB");
    }

    #[test]
    fn one_part_is_not_plural() {
        let nodes = tree("multipart/mixed", &[part("1", "text/plain", 10)]);
        assert!(summary(&nodes).contains("1 part ·"));
    }

    #[test]
    fn a_part_is_called_by_its_name_when_it_has_one() {
        let nodes = tree("multipart/mixed", &message());

        assert_eq!(nodes[1].label(), "text/plain", "unnamed, so its type");
        assert_eq!(nodes[3].label(), "0001-index.patch");
    }

    #[test]
    fn the_detail_line_pairs_the_type_with_the_size() {
        let nodes = tree("multipart/mixed", &message());
        assert_eq!(detail(&nodes[2]), "text/html · 6.0 KB");
        assert_eq!(
            detail(&nodes[0]),
            "multipart/mixed",
            "the root is not sized"
        );
    }

    #[test]
    fn a_container_is_not_something_to_save() {
        let nodes = tree(
            "multipart/mixed",
            &[
                part("1", "multipart/alternative", 0),
                part("1.1", "text/plain", 10),
            ],
        );
        assert!(!nodes[0].is_leaf(), "the message is not a part");
        assert!(
            !nodes[1].is_leaf(),
            "a container holds parts, it is not one"
        );
        assert!(nodes[2].is_leaf());
        assert_eq!(note(&nodes[1], 0, 0), CONTAINER);
    }

    #[test]
    fn a_held_back_html_part_is_explained_before_it_is_called_downloaded() {
        let mut html = part("1", "text/html", 900);
        html.blob_id = Some(BlobId::new("abc"));
        let nodes = tree("multipart/alternative", &[html]);

        assert_eq!(
            note(&nodes[1], 6, 1),
            held_back_note("text/html", 6, 1).expect("a held-back sentence")
        );
        assert!(
            note(&nodes[1], 0, 0).contains("already downloaded"),
            "with nothing held back it is an ordinary downloaded part"
        );
    }

    #[test]
    fn only_the_markup_part_is_ever_held_back() {
        assert!(held_back_note("image/png", 6, 1).is_none(), "theatre");
        assert!(
            held_back_note("TEXT/HTML", 6, 0).is_some(),
            "case is not it"
        );
        assert!(held_back_note("text/html", 0, 0).is_none());
        assert_eq!(
            held_back_note("text/html", 1, 1).as_deref(),
            Some(
                "HTML part held back — 1 remote image and 1 likely tracker would load. \
                 The plain-text part is showing instead."
            )
        );
    }

    #[test]
    fn a_save_name_comes_from_the_sender_when_it_is_usable() {
        let nodes = tree("multipart/mixed", &message());
        assert_eq!(save_name(&nodes[3]), "0001-index.patch");
    }

    #[test]
    fn a_save_name_cannot_steer_a_save_out_of_its_folder() {
        let name = save_name(&leaf("text/plain", Some("../../.bashrc")));

        assert!(!name.contains('/'), "{name}");
        assert!(!name.starts_with('.'), "{name}");
    }

    #[test]
    fn a_part_the_sender_did_not_name_still_gets_one() {
        let nodes = tree("multipart/mixed", &[part("2.1", "image/png", 10)]);
        assert_eq!(save_name(&nodes[1]), "part-2.1.png");
    }

    /// #147: an interior NUL in a filename must not reach gtk-rs's C-string conversion.
    #[test]
    fn a_save_name_carries_no_control_characters() {
        for hostile in ["a\0b.txt", "a\nb.txt", "a\rb\tc.txt", "\u{7}bell.txt"] {
            let name = save_name(&leaf("text/plain", Some(hostile)));
            assert!(
                !name.chars().any(char::is_control),
                "{hostile:?} produced a name with a control character: {name:?}"
            );
            assert!(!name.is_empty(), "{hostile:?} produced an empty name");
        }
    }

    #[test]
    fn a_save_name_launders_a_slash_out_of_an_undecoded_encoded_word() {
        let name = save_name(&leaf("text/plain", Some("=?utf-8Qa/b.txt")));
        assert!(!name.contains('/'), "{name}");
    }

    #[test]
    fn a_name_that_is_nothing_but_punctuation_falls_back() {
        let name = save_name(&leaf("image/png", Some("  ...  ")));
        assert_eq!(name, "part-1.png");
    }

    #[test]
    fn two_parts_claiming_one_name_do_not_become_one_file() {
        let nodes = tree(
            "multipart/mixed",
            &[
                named("1", "application/pdf", 10, "invoice.pdf"),
                named("2", "application/pdf", 10, "invoice.pdf"),
                named("3", "application/pdf", 10, "invoice.pdf"),
            ],
        );
        let names = save_names(&nodes[1..]);

        assert_eq!(names, ["invoice.pdf", "invoice-2.pdf", "invoice-3.pdf"]);
    }

    #[test]
    fn a_message_with_no_collisions_keeps_every_name_untouched() {
        let nodes = tree("multipart/mixed", &message());
        let names = save_names(&nodes[1..]);

        assert_eq!(
            names,
            [
                "part-1.plain",
                "part-2.html",
                "0001-index.patch",
                "cold.png"
            ],
            "a suffix on a name that did not need one is noise in every \
             directory the user saves into"
        );
    }

    #[test]
    fn a_collision_is_judged_without_case_because_the_filesystem_may_be() {
        let nodes = tree(
            "multipart/mixed",
            &[
                named("1", "application/pdf", 10, "Invoice.pdf"),
                named("2", "application/pdf", 10, "invoice.pdf"),
            ],
        );
        let names = save_names(&nodes[1..]);

        assert_eq!(
            names,
            ["Invoice.pdf", "invoice-2.pdf"],
            "on a case-insensitive filesystem these are one file, and \
             'save all' must write the same number of files everywhere"
        );
    }

    #[test]
    fn the_suffix_goes_before_the_extension_so_the_file_still_opens() {
        assert_eq!(suffixed("invoice.pdf", 2), "invoice-2.pdf");
        assert_eq!(suffixed("archive.tar.gz", 3), "archive.tar-3.gz");
        assert_eq!(suffixed("README", 2), "README-2");
    }

    #[test]
    fn a_partial_save_says_how_many_it_could_not_get() {
        assert_eq!(save_all_failure(0), None, "silence when nothing failed");
        assert_eq!(
            save_all_failure(1).as_deref(),
            Some("1 part could not be saved")
        );
        assert_eq!(
            save_all_failure(3).as_deref(),
            Some("3 parts could not be saved")
        );
    }

    #[test]
    fn a_save_says_what_it_saved_and_one_part_is_one_attachment() {
        assert_eq!(saved("report.pdf"), "Saved report.pdf");
        assert_eq!(saved_all(1, 0), "Saved 1 attachment");
        assert_eq!(saved_all(3, 0), "Saved 3 attachments");
        assert_eq!(
            saved_all(3, 1),
            "1 part could not be saved",
            "a partial save says what went wrong, not what went right"
        );
    }

    #[test]
    fn walking_holds_at_both_ends_rather_than_wrapping() {
        assert_eq!(step(1, true, 3), 2);
        assert_eq!(step(2, true, 3), 2, "the bottom holds");
        assert_eq!(step(1, false, 3), 0);
        assert_eq!(step(0, false, 3), 0, "and so does the top");
    }

    #[test]
    fn walking_an_empty_tree_moves_nowhere() {
        assert_eq!(step(0, true, 0), 0);
        assert_eq!(step(7, false, 0), 0);
        assert_eq!(
            step(7, true, 3),
            2,
            "a cursor left over from a longer tree is brought back in range"
        );
    }
}

#[cfg(test)]
mod blob_source_tests {
    use super::BlobSource;

    #[test]
    fn a_closure_resolves_and_ignores_the_scope_by_default() {
        let source =
            |id: &str| (id == "logo@example.com").then(|| (vec![1, 2], "image/png".to_owned()));
        assert_eq!(
            source.resolve("logo@example.com"),
            Some((vec![1, 2], "image/png".to_owned()))
        );
        assert_eq!(source.resolve("other@example.com"), None);
        assert_eq!(
            source.resolve_in(Some("7"), "logo@example.com"),
            source.resolve("logo@example.com")
        );
        assert_eq!(source.resolve_in(None, "other@example.com"), None);
    }
}
