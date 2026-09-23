//! What a message is made of: its MIME tree, and the rules for showing and
//! saving one part of it.
//!
//! Toolkit-free and frontend-free on purpose. A message is a tree of parts,
//! and a mail client that only ever shows you the one part it decided to
//! render is one you cannot check — so both frontends draw the whole
//! structure, and every judgement involved in drawing it is here rather than
//! in either of them (ADR 0019).
//!
//! The tree, the box drawing, the words each row and each detail pane says,
//! and above all [`save_name`] arrived from `postio-gtk::parts`, which is
//! where they were written and where the bugs that shaped them were found.
//! What stayed behind is the widget.
//!
//! # Structure before bytes
//!
//! Everything here comes from `BODYSTRUCTURE`, which IMAP returns without
//! transferring a single byte of any part. [`Attachment`] already carries
//! exactly that: a `part_id` like `2.1`, a `mime_type`, a declared `size`,
//! and a `blob_id` that is `None` until the bytes have actually been
//! downloaded. So the tree is a *reading* of metadata the store already has,
//! and a panel can be complete and correct for a message nothing has been
//! fetched for.
//!
//! That is the point rather than an optimisation. "Nothing downloads until
//! the user asks" is a privacy promise (`PRODUCT.md`), and the way to keep it
//! is for the surface that shows attachments to have no way to fetch one.
//! Nothing in this module can: it takes rows and returns words.
//!
//! # The tree comes from the part ids
//!
//! IMAP part ids are paths — `1`, `2`, `2.1`, `2.2` — so the nesting is
//! already in the data and [`tree`] only has to read it. Nothing here invents
//! a hierarchy or asks the store for one.

use postio_model::Attachment;
use postio_model::ids::AttachmentId;

use crate::format::human_size;

/// Resolves a `Content-ID` to the bytes it names.
///
/// The GTK reader resolves `cid:` through a registered URI scheme and a
/// `WKWebView` will resolve it through something else entirely, but *what a
/// `Content-ID` may resolve to* is a security property rather than a
/// rendering detail, so it is stated once here and both frontends are handed
/// the same answer (ADR 0019 Q6, #608).
///
/// A `Content-ID` is passed exactly as `postio_body::sanitize::percent_decode`
/// recovered it: without the `cid:` prefix and without the angle brackets some
/// senders wrap it in (`sanitize_body` already strips those before encoding).
///
/// Synchronous and local by design — an implementation is a blob-store read or
/// a lookup into whatever the caller already has in memory for the open
/// message, never a call that blocks on I/O the reader would have to await.
/// That is not only about latency: a `cid:` that could reach the network would
/// be the tracking pixel the reader spends so much effort blocking, arriving
/// through the back door.
pub trait BlobSource {
    /// The part's bytes and MIME type, or `None` if no part carries this id.
    fn resolve(&self, content_id: &str) -> Option<(Vec<u8>, String)>;

    /// The same, for a reference that names *which message* it belongs to.
    ///
    /// A single-message document does not need to: the reader knows which
    /// message is open, so `scope` is `None` and this is [`resolve`]. ADR
    /// 0032's conversation document holds a whole thread, where "whichever is
    /// open" names nothing — two messages may each carry a part called `logo`,
    /// and a sender may reference a `Content-ID` they know belongs to somebody
    /// else's message in the same thread.
    ///
    /// The default ignores the scope, which is right for a source that only
    /// ever holds one message's parts and wrong for one that holds a thread's.
    /// A thread source must override it, and a scope it does not recognise
    /// must resolve to nothing rather than to whatever it has.
    ///
    /// [`resolve`]: Self::resolve
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

/// One node of the tree, flattened into the order the keyboard walks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The IMAP part id — `2.1`. Empty for the synthetic root.
    pub part_id: String,
    /// How deep it sits; the root is 0.
    pub depth: usize,
    /// `text/html`, `image/png`, `multipart/mixed`.
    pub mime: String,
    /// The name the sender gave it, if any.
    pub filename: Option<String>,
    /// Size in bytes as the server declared it. `0` for a container.
    pub size: u64,
    /// Whether the bytes are already in the blob store.
    pub downloaded: bool,
    /// Whether this is the last child of its parent, for `└` rather than `├`.
    pub last: bool,
    /// The attachment row this came from; `None` for the synthetic root.
    pub attachment: Option<AttachmentId>,
    /// The `Content-ID` the body may reference this part by, if it has one.
    pub content_id: Option<String>,
    /// Whether the message means this part to be shown in place rather than
    /// offered as an attachment.
    ///
    /// Both halves of the question, because senders answer only one of them:
    /// a `Content-Disposition: inline`, or a `Content-ID` the markup can
    /// point at. A part with a `cid:` and no disposition is still one the
    /// body draws, and a list that called it an attachment would offer the
    /// sender's signature logo beside their invoice.
    ///
    /// It is a *label*, never a filter: an inline part is a part like any
    /// other here, with a row, a size and a save. #751's oversized inline
    /// image is exactly the case where being able to see and save it is the
    /// only explanation the user gets for the broken box in the body.
    pub inline: bool,
}

impl Node {
    /// What the row calls this part: its filename, or its type when the
    /// sender did not name it.
    pub fn label(&self) -> &str {
        match self.filename.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name,
            _ => &self.mime,
        }
    }

    /// Whether this part holds bytes worth saving, as opposed to being a
    /// container for other parts.
    pub fn is_leaf(&self) -> bool {
        !self.mime.starts_with("multipart/") && self.attachment.is_some()
    }
}

/// Reads `parts` as a tree, flattened in walk order.
///
/// `root` is the message's own content type — `multipart/mixed` — which is a
/// property of the message rather than of any part, so it is passed in rather
/// than guessed. [`root_type`] is what works it out. A message with no parts
/// still gets its root node: a tree with one entry is a true answer, where an
/// empty panel would look broken.
///
/// Parts are ordered by their id read as a path of numbers, so `2.10` sorts
/// after `2.9` rather than before it the way a string compare would.
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
        // The last child of *this* parent, not the last row overall: a
        // deeper branch that ends before its parent's next sibling still
        // needs its own `└`.
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

/// A part id read as a path: `"2.1"` becomes `[2, 1]`.
///
/// A part with no id at all, or one that is not a path of numbers, sorts as
/// nothing and is dropped — the tree draws what the server described, and a
/// row nothing can be fetched for is a row that leads nowhere.
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

/// The message's own content type — the row the parts tree hangs off.
///
/// # Read when it is there, derived otherwise
///
/// `BODYSTRUCTURE` says what it is and `postio-account` records it in
/// [`Message::content_type`] at fetch time (`postio-roj4`), so `stored` is
/// the honest answer whenever a sync has actually filled it in. `stored` is
/// `None` for a row synced before that column existed and never refetched
/// since — the composer's own in-progress drafts too — and for those this
/// falls back to reconstructing a plausible shape from what *is* recorded: a
/// message with parts is `multipart/mixed`, one with two bodies is
/// `multipart/alternative`, and one with neither is whichever body it has.
///
/// The fallback can be wrong in exactly the case the real value fixes: a
/// `multipart/related` with inline images has parts, so it reads as
/// `multipart/mixed` here. That is a label on one row rather than a wrong
/// tree, which is why it was P3 rather than a bug.
///
/// [`Message::content_type`]: postio_model::Message::content_type
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

/// The box-drawing prefix the canvas draws down the left of the tree.
///
/// Two spaces per level of nesting, then `├ ` or `└ `. The root has none.
pub fn prefix(node: &Node) -> String {
    if node.depth == 0 {
        return String::new();
    }
    let indent = "  ".repeat(node.depth - 1);
    let branch = if node.last { "└ " } else { "├ " };
    format!("{indent}{branch}")
}

/// The header line: `multipart/mixed · 4 parts · 1.2 MB`.
///
/// The count is of parts, not of nodes — the root is the message, not a part
/// of it.
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

/// What the detail pane says about one part: `text/html · 6 KB`.
pub fn detail(node: &Node) -> String {
    if node.depth == 0 || node.size == 0 {
        return node.mime.clone();
    }
    format!("{} · {}", node.mime, human_size(node.size))
}

/// Whether a part is one Postio can show inline rather than only save.
///
/// Images and PDFs, and nothing else. Everything else is bytes the
/// application has no business interpreting, and "Open with…" hands those to
/// the desktop rather than guessing.
pub fn previewable(mime: &str) -> bool {
    let mime = mime.trim().to_ascii_lowercase();
    mime.starts_with("image/") || mime == "application/pdf"
}

/// What the detail pane says about a part nothing has fetched.
pub const NOT_FETCHED: &str =
    "Described by the server, not downloaded. Nothing here has touched the network.";

/// What it says about a container.
pub const CONTAINER: &str = "A container for the parts below it. Nothing to save.";

/// The sentence beside the part the cursor is on.
///
/// Four states with four different things to say, and the ordering matters:
/// a held-back HTML part is *also* downloaded, and saying "already
/// downloaded" about it would answer a question nobody asked while leaving
/// the one they did ask — why is the body not showing — unanswered.
///
/// Shared rather than composed per frontend because this is the only place
/// several of these states are ever explained. #751's oversized inline image
/// is the case: the `cid:` request finds nothing, the pane draws a broken
/// box, and [`NOT_FETCHED`] on that part's row is the entire explanation the
/// user gets. A frontend that wrote its own three sentences would be one
/// frontend where that explanation was missing.
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

/// Why a part is being held back, or `None` when it is not.
///
/// Only the markup parts: an `image/png` attachment references nothing and
/// cannot phone home, so holding it back would be theatre. `text/html` is the
/// one that loads things.
///
/// This is also what decides whether "Render once" is offered at all — the
/// button exists exactly when there is a sentence to put above it.
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
    // "Likely", always. The count comes from a size heuristic
    // (`postio_body::sanitize`) that reads what an `<img>` declares about
    // itself and nothing else -- it cannot know a beacon from a very small
    // picture, and it deliberately under-counts rather than accuse an
    // ordinary image. The word is the difference between a signal and a
    // claim, and both kinds are blocked identically either way (#174).
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

/// How a part reads to a screen reader: what it is, how big, and whether
/// anything has actually been downloaded.
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

/// A filename safe to write `node` out under.
///
/// The sender's name when there is one, with any path separators taken out —
/// a part called `../../.bashrc` must not be able to steer where a save
/// lands. Otherwise the part id and a guess at an extension, so no caller
/// ever has to invent a name for an unnamed part.
///
/// This is where an attachment filename stops being *reported* and starts
/// being *used*. [`postio_model::mime::parse`] hands over what the sender
/// wrote, faithfully and on purpose; everything that makes it fit to name a
/// file happens here, which is why the traversal and control-character tests
/// live beside this function and not beside the parser.
///
/// It lives in this crate rather than in a frontend for the reason the whole
/// module does, and more sharply: a second sanitiser is a second chance to
/// miss a separator, and the platform that missed it would be the one with no
/// test for the message that exploited it.
pub fn save_name(node: &Node) -> String {
    if let Some(name) = node.filename.as_deref().map(str::trim)
        && !name.is_empty()
    {
        let cleaned: String = name
            .chars()
            // A separator becomes a dash: the sender meant a character to be
            // there, and eliding it silently joins two name components.
            .map(|c| if c == '/' || c == '\\' { '-' } else { c })
            // A control character is dropped rather than marked, because the
            // sender did not mean anything by it that a reader could see.
            //
            // #147, found by the `parse_message` fuzz target: a NUL reaches
            // here both from a literal `filename="a\0b.txt"` and from one
            // base64'd inside an RFC 2047 encoded word. The name then goes to
            // a file dialog, and gtk-rs converts a `&str` to a C string on
            // the way — a conversion an interior NUL has no valid answer for.
            // Pressing `s` on a message must not be how the application ends.
            .filter(|c| !c.is_control())
            .collect();
        let cleaned = cleaned.trim_matches(['.', ' '].as_slice()).to_owned();
        if !cleaned.is_empty() {
            return cleaned;
        }
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

/// A name for every one of `nodes`, no two of them the same.
///
/// [`save_name`] answers about one part in isolation, which is the right
/// answer for "save this one" and the wrong one for "save all": nothing stops
/// a message carrying two parts that both call themselves `invoice.pdf`, and
/// writing them one after another under their own names produces a directory
/// with one invoice in it and no sign that a second ever existed. A silent
/// loss of the user's mail, from the command whose whole promise is that it
/// got everything.
///
/// So a repeat is suffixed: `invoice.pdf`, `invoice-2.pdf`, `invoice-3.pdf`,
/// before the extension so the file still opens in the right application.
/// The first of a set keeps its plain name — a message with no collision at
/// all is the common case and must look untouched.
///
/// **Compared without case**, because the filesystem may be: macOS and a
/// default Windows share are case-insensitive, so `Invoice.pdf` beside
/// `invoice.pdf` is one file there and two on Linux. Taking the cautious
/// reading everywhere means "save all" writes the same number of files on
/// every platform, which is a far better property than matching each
/// filesystem's own rule.
pub fn save_names(nodes: &[Node]) -> Vec<String> {
    let mut taken: Vec<String> = Vec::with_capacity(nodes.len());
    let mut names = Vec::with_capacity(nodes.len());
    for node in nodes {
        let wanted = save_name(node);
        let folded = wanted.to_lowercase();
        let name = if taken.contains(&folded) {
            // From two, because the unsuffixed name is the first one.
            (2..)
                .map(|nth| suffixed(&wanted, nth))
                .find(|candidate| !taken.contains(&candidate.to_lowercase()))
                // `(2..)` over `u32` is not infinite, so the compiler wants
                // an answer for running out. A message would need four
                // billion parts of one name to get here; the unsuffixed
                // name is a truthful fallback and the loop above is what
                // actually runs.
                .unwrap_or(wanted)
        } else {
            wanted
        };
        taken.push(name.to_lowercase());
        names.push(name);
    }
    names
}

/// `invoice.pdf` and 2 become `invoice-2.pdf`.
///
/// Before the extension rather than after it: `invoice.pdf-2` opens in
/// nothing, and the point of the suffix is to keep both files usable.
/// A name with no extension simply takes the suffix at the end.
fn suffixed(name: &str, nth: u32) -> String {
    match name.rsplit_once('.') {
        // A leading dot is the whole name, not an extension — and
        // `save_name` has already refused to produce one, so this is
        // defensive rather than reachable.
        Some((stem, extension)) if !stem.is_empty() => format!("{stem}-{nth}.{extension}"),
        _ => format!("{name}-{nth}"),
    }
}

/// What to say when "save all" could not get every part, or `None` when it
/// could.
///
/// A count rather than which ones: a message can easily name a dozen parts,
/// and one message per failure would be worse than the save. One sentence for
/// the batch, shared, so the two frontends do not report the same partial
/// save in two different ways.
pub fn save_all_failure(failed: usize) -> Option<String> {
    if failed == 0 {
        return None;
    }
    Some(format!(
        "{failed} part{} could not be saved",
        if failed == 1 { "" } else { "s" }
    ))
}

/// Which row a freshly opened parts panel puts its cursor on.
///
/// The first *part*, not the message: the row the user came to look at is one
/// of the things inside, and starting on the container would cost a keystroke
/// every single time. A message with no parts at all has only the root, and
/// the cursor goes there because there is nowhere else.
///
/// The rule crosses; the cursor itself does not. Where the keyboard is inside
/// a panel is the panel's own state, the same way the message list's cursor
/// is — what has to be shared is what "the first part" and "the next one"
/// mean, so the two frontends do not walk the same tree differently.
pub fn first_part(count: usize) -> usize {
    if count > 1 { 1 } else { 0 }
}

/// Where the cursor goes when the walk keys are pressed.
///
/// **Clamps rather than wraps.** A tree is a thing you read down, and a `j`
/// at the bottom that jumped back to the message header would lose the
/// reader's place in a structure they were working through. The message list
/// holds at its ends for the same reason.
///
/// `count` is the number of rows including the root, so a `count` of 0 — no
/// tree drawn yet — leaves the cursor at 0 rather than moving it somewhere
/// there is no row.
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

    /// Canvas 3g's own message.
    fn message() -> Vec<Attachment> {
        vec![
            part("1", "text/plain", 2_100),
            part("2", "text/html", 6 * 1024),
            named("3", "text/x-diff", 11 * 1024, "0001-index.patch"),
            named("4", "image/png", 1_100 * 1024, "cold.png"),
        ]
    }

    /// One leaf, for the rules that are about a single node.
    fn leaf(mime: &str, filename: Option<&str>) -> Node {
        let mut row = part("1", mime, 10);
        row.filename = filename.map(str::to_owned);
        tree("multipart/mixed", &[row]).remove(1)
    }

    // -- reading the tree --------------------------------------------------

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

    // -- inline against attached -------------------------------------------

    #[test]
    fn a_part_the_body_draws_is_marked_inline_either_way_the_sender_said_so() {
        // Senders answer one of the two questions and not the other, so both
        // have to count. A `cid:` with no disposition is still a part the
        // markup points at; a `Content-Disposition: inline` with no
        // `Content-ID` is still one the message means to be shown in place.
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
        // #751's last acceptance criterion. An inline part over
        // `[sync] max_inline_bytes` stays on the payload axis, so the `cid:`
        // request 404s and the pane draws a broken box. What stops that being
        // *silent* is the parts panel: the part is listed, sized, and said to
        // be undownloaded, so there is somewhere to go and find out why.
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

    // -- the message's own type --------------------------------------------

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

    // -- the box drawing ---------------------------------------------------

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

    // -- sizes -------------------------------------------------------------

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

    // -- what a row says ---------------------------------------------------

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
        // The ordering that matters: the HTML part *is* on this machine, so
        // "already downloaded" is true and useless. What the reader wants to
        // know is why the pane is showing the plain-text version instead.
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

    // -- previewing and saving ---------------------------------------------

    #[test]
    fn only_images_and_pdfs_are_shown_rather_than_handed_over() {
        assert!(previewable("image/png"));
        assert!(previewable("IMAGE/JPEG"));
        assert!(previewable("application/pdf"));
        assert!(!previewable("text/html"));
        assert!(!previewable("application/octet-stream"));
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

    /// #147, found by the `parse_message` fuzz target within a minute of its
    /// first run. A NUL reaches `Attachment::filename` two ways -- written
    /// straight into the `filename=` parameter, or base64'd inside an RFC 2047
    /// encoded word -- and `mime::parse` reports it faithfully, which is its
    /// job. This is the layer that has to make it safe: the name goes to a
    /// file dialog, and gtk-rs converts a `&str` to a C string on the way,
    /// which an interior NUL is not a valid input for. Opening a message's
    /// parts and pressing `s` is not allowed to be how the application ends.
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

    /// The same find's other half: a slash survives a *malformed* encoded
    /// word, which is not the shape the existing traversal test used.
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

    // -- saving all of them ------------------------------------------------

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

    // -- walking -----------------------------------------------------------

    #[test]
    fn the_cursor_opens_on_the_first_part_rather_than_on_the_message() {
        assert_eq!(first_part(5), 1, "four parts and the message");
        assert_eq!(
            first_part(1),
            0,
            "a message with no parts has only the root to stand on"
        );
        assert_eq!(first_part(0), 0, "nothing drawn yet");
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
