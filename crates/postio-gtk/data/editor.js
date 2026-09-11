/* The composer's editing bridge — the one script Postio runs in a WebView.
 *
 * ADR 0003: this is the host's own code, shipped in the GResource bundle,
 * never message content; the profile it runs under keeps markup-borne
 * script inert (enable_javascript_markup off) and the shell CSP names no
 * remote origin. Its whole job is two things:
 *
 *   1. Pin the dialect the contract test (gtk_editable_dialect.rs) proves:
 *      <p> paragraphs and element-form bold/italic, set before any gesture
 *      can run.
 *   2. Report every edit to the host as the DOM's innerHTML, which the host
 *      parses back into the canonical Document — the DOM is a working copy,
 *      never the record (ADR 0004 Q3).
 *
 * No timers, no network, no state beyond the document it edits.
 */
document.execCommand('defaultParagraphSeparator', false, 'p');
document.execCommand('styleWithCSS', false, 'false');

/* Markdown as input (spec 002, FR-067 to FR-072). Not a dialect and not a
 * format: a typed sequence runs one of the formatting commands the toolbar
 * and the palette already offer, and the draft is rich text either way. The
 * table is `POSTIO_MARKDOWN`, generated from `postio_ui::editor::markdown`
 * and prepended to this file, so both frontends support the same set rather
 * than each inventing one.
 *
 * Everything here goes through `execCommand`, which is what keeps FR-070
 * true without any bookkeeping: the browser's own undo stack sees one
 * grouped edit, so a single ctrl+z puts the literal characters back. Writing
 * the DOM directly would take the undo entry with it. */

/* The command each id runs, and the argument it needs. */
const MARKDOWN_COMMANDS = {
    bold: ['bold', null],
    italic: ['italic', null],
    bullet_list: ['insertUnorderedList', null],
    numbered_list: ['insertOrderedList', null],
    quote_block: ['formatBlock', 'blockquote'],
};

function runFormatting(id) {
    const entry = MARKDOWN_COMMANDS[id];
    if (!entry) return false;
    return document.execCommand(entry[0], false, entry[1]);
}

/* The caret, when it sits in a text node with nothing selected. Every
 * transformation below needs exactly this and none of them are meaningful
 * over a selection -- a sequence is something you *type*. */
function caretInText() {
    const selection = window.getSelection();
    if (!selection || !selection.isCollapsed || selection.rangeCount === 0) return null;
    const node = selection.anchorNode;
    if (!node || node.nodeType !== Node.TEXT_NODE) return null;
    return { node: node, offset: selection.anchorOffset };
}

/* `- ` and friends: the marker plus a space, at the very start of a block.
 * Anywhere else a hyphen is a hyphen. */
function applyLinePrefix() {
    const caret = caretInText();
    if (!caret) return false;
    /* A trailing space in a `contenteditable` is inserted as U+00A0, because
     * an ordinary one at the end of a line would collapse and leave nothing
     * visible under the caret. So the space the user typed is not the space
     * this would be comparing against -- which is exactly the sort of thing
     * that makes a feature work when poked from the console and never when
     * typed. */
    const before = caret.node.data.slice(0, caret.offset).replace(/\u00a0/g, ' ');
    for (const sequence of POSTIO_MARKDOWN) {
        if (sequence.trigger !== 'line_prefix') continue;
        const typed = sequence.marker + ' ';
        if (before !== typed) continue;
        /* Only at the start of its block: the text node must be the block's
         * first, or this is mid-sentence and the user typed a hyphen. */
        const block = caret.node.parentNode;
        if (!block || block.firstChild !== caret.node) continue;

        const range = document.createRange();
        range.setStart(caret.node, 0);
        range.setEnd(caret.node, caret.offset);
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        document.execCommand('delete');
        return runFormatting(sequence.command);
    }
    return false;
}

/* How many of `character` sit immediately before `end` in `text`. */
function runLengthBack(text, end, character) {
    let run = 0;
    while (end - run > 0 && text[end - run - 1] === character) run += 1;
    return run;
}

/* The last position in `text` holding a run of `marker` that is exactly
 * `marker.length` long -- not part of a longer run of the same character.
 * `-1` when there is none. */
function openingRun(text, marker) {
    const character = marker[0];
    let at = text.lastIndexOf(marker);
    while (at >= 0) {
        const before = runLengthBack(text, at, character);
        const after = runLengthBack(text, at + marker.length, character) - marker.length;
        if (before === 0 && after === 0) return at;
        at = text.lastIndexOf(marker, at - 1);
    }
    return -1;
}

/* `**bold**`: the closing marker has just been typed, and the opening one is
 * somewhere earlier in the same text node with content between them. */
function applyWrapping() {
    const caret = caretInText();
    if (!caret) return false;
    const before = caret.node.data.slice(0, caret.offset);
    for (const sequence of POSTIO_MARKDOWN) {
        if (sequence.trigger !== 'wrapping') continue;
        const marker = sequence.marker;
        if (!before.endsWith(marker)) continue;

        /* Both ends must be runs of *exactly* the marker's length, and that
         * is the whole subtlety of `**` living beside `*`. Typing
         * `**loudly*` -- the first of the two closing asterisks -- ends the
         * text in a single `*`, and a naive `lastIndexOf('*')` finds the
         * second asterisk of the *opening* pair and makes `loudly` italic on
         * the spot, so the bold the user was halfway through typing never
         * happens. A run of two must never satisfy a marker of one. */
        if (runLengthBack(before, before.length, marker[0]) !== marker.length) continue;

        const inner = before.slice(0, before.length - marker.length);
        const open = openingRun(inner, marker);
        if (open < 0) continue;
        const content = inner.slice(open + marker.length);
        /* Empty content means `****`, which is four asterisks and not a
         * formatting request. Content with a marker in it means the user is
         * typing about markers. */
        if (content.length === 0 || content.includes(marker)) continue;

        const selection = window.getSelection();
        /* Take the closing marker off first, then the opening one, so the
         * offsets of what is between them do not move under us. */
        const closing = document.createRange();
        closing.setStart(caret.node, caret.offset - marker.length);
        closing.setEnd(caret.node, caret.offset);
        selection.removeAllRanges();
        selection.addRange(closing);
        document.execCommand('delete');

        const opening = document.createRange();
        opening.setStart(caret.node, open);
        opening.setEnd(caret.node, open + marker.length);
        selection.removeAllRanges();
        selection.addRange(opening);
        document.execCommand('delete');

        const run = document.createRange();
        run.setStart(caret.node, open);
        run.setEnd(caret.node, open + content.length);
        selection.removeAllRanges();
        selection.addRange(run);
        const applied = runFormatting(sequence.command);

        /* The caret goes back to the end of what was just formatted, with
         * the formatting off -- otherwise the next character typed joins the
         * bold run the user just closed. */
        selection.collapseToEnd();
        if (applied && document.queryCommandState(MARKDOWN_COMMANDS[sequence.command][0])) {
            runFormatting(sequence.command);
        }
        return applied;
    }
    return false;
}

function applyMarkdown(event) {
    if (event.inputType !== 'insertText') return false;
    /* Both, rather than branching on `event.data`: WebKit does not fill it
     * in for text inserted through `execCommand`, so a check for `' '` sends
     * every space down the wrong path and no line prefix ever fires. Each
     * recogniser is self-guarding anyway -- a line prefix needs the block to
     * read exactly `marker + space`, a wrapping needs a closing marker just
     * typed -- so trying both costs a string compare and asks nothing of a
     * field that may not be there. */
    return applyLinePrefix() || applyWrapping();
}

document.addEventListener('input', (event) => {
    /* Captured before the transformation, because this is the state one undo
     * has to be able to return to: the literal `**loudly**` the user typed,
     * markers and all (FR-070). Reported first so the typing run ends there,
     * and the conversion then arrives on its own channel as a step of its
     * own. */
    const literal = document.body.innerHTML;
    if (applyMarkdown(event)) {
        window.webkit.messageHandlers.postioEdited.postMessage(literal);
        window.webkit.messageHandlers.postioConverted.postMessage(
            document.body.innerHTML
        );
        return;
    }
    window.webkit.messageHandlers.postioEdited.postMessage(
        document.body.innerHTML
    );
});

/* The reflection channel: the formatting in force where the caret sits, as
 * a space-joined list of the registry command ids it maps to. Reported on
 * both selection movement and edits, because either one can move the caret
 * in or out of a Strong run; the host dedups. */
function reportFormat() {
    const active = [];
    if (document.queryCommandState('bold')) active.push('bold');
    if (document.queryCommandState('italic')) active.push('italic');
    if (document.queryCommandState('insertUnorderedList')) active.push('bullet_list');
    if (document.queryCommandState('insertOrderedList')) active.push('numbered_list');
    if (document.queryCommandValue('formatBlock') === 'blockquote') active.push('quote_block');
    window.webkit.messageHandlers.postioFormat.postMessage(active.join(' '));
}
document.addEventListener('selectionchange', reportFormat);
document.addEventListener('input', reportFormat);
