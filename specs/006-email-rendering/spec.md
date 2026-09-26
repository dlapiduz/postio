# Feature Specification: Faithful, Readable Email Rendering

**Feature Branch**: `feature/email-rendering`

**Created**: 2026-09-26

**Status**: Draft

**Input**: User description: "The reading of emails right now is buggy, especially in dark mode, where I see black text in the dark background. I think we need to have a spec for great email rendering. We have over indexed in privacy and removing layout but it hasnt worked that well. Maybe we can take a look at the blitz spike and see if we can start with just a disconnected renderer that doesnt process javascript and loads images from the attached email."

## Context

This spec is about **what a message body looks like once it is on screen**:
whether it is legible, whether it looks the way its sender built it, and
whether it does both in light and dark mode. The reading pane around the body
(header, actions, thread, rail) is `specs/001-conversation-reading-pane`, and
this spec does not restate it. 001's Rendering requirements (FR-019 to FR-027)
still apply and are **inherited**: sender layout honoured, sender styling
confined to its message, nothing fetched without per-sender consent, no script.
What 001 left out, and this spec adds, is dark mode, legacy email markup, a
renderer that cannot reach the network at all, and the reading affordances that
renderer has to supply itself.

### Why reading is broken today

The user's report is **dark text on a dark background in dark mode**. The code
explains it:

| Cause | Where |
|---|---|
| A sender's text colours reach the screen, but their page canvas does not. Mail sets its white canvas on `<body>` (`bgcolor`, `style`, `text`). The sanitizer cleans the body as a fragment, which drops `<body>` and its attributes. So `color:#333` is painted on the dark theme ground. | `postio-body/src/sanitize.rs`; `postio-ui/data/reader.css` (`.postio-body` has no background) |
| A sender's own dark-mode rules cannot match. `@media (prefers-color-scheme: dark)` is admitted, but it selects on classes, and `class` is stripped. | #1545 |
| Nothing checks contrast. No sender colour is adapted, clamped or checked against what is actually behind it. | `postio-ui/src/reader/document.rs` |
| The page and the app can disagree about which theme is on. The document takes its scheme from the web engine's own setting, and the app takes it from libadwaita. The editor already had this bug and was fixed; the reader was not. | `postio-gtk/src/reader/editor.rs:185` |
| Legacy colour markup is lost. `<font color>` is not an admitted tag, so part of an old message's colour vanishes and part survives. | `sanitize.rs` |
| Bulk mail opens in Reader view by default. That view strips `style`, `bgcolor`, `width` and `class`, so newsletters open flattened. | `document.rs` `suits_reader_view` |
| The dark palette for the body was derived mechanically, not designed. | #1588 |

The user's summary is fair: **the posture over-indexed on removing things.**
Each removal was defensible alone. Together they produce a body that is neither
safe to read (it can be illegible) nor faithful (the layout is gone). This spec
turns the posture around. **Legibility and fidelity are the product.** Privacy
stays absolute, but it is enforced by what the renderer is able to do, not by
deleting markup.

### The direction: a disconnected renderer

The Blitz spike (#1543, branch `spike/blitz-reader`) rendered message bodies
in-process, painted on the CPU into a texture, with no web process and no
network stack compiled in. Its findings are the evidence behind this spec's
direction:

- **All nine HTML fixtures in the corpus lay out correctly, and so do all eight
  hand-built legacy fixtures.** Those cover nested `%`/`px` tables, `bgcolor`,
  `align`, `valign` and `cellpadding` on tables, scoped `<style>`, `@media`,
  Outlook conditional comments and VML (ignored), CSS Grid, and a full
  promotional mail. Two gaps remain, both cosmetic and both upstream:
  `border="1"` draws no border, and `vertical-align` on an inline-block is
  ignored.
- **Resources are answered from a closed table.** The table holds the
  message's own inline parts and the app's embedded fonts, and nothing else.
  "Cannot reach the network" is a property of the build, not of eighteen
  settings someone has to keep switched off.
- **Speed is not the argument.** Steady-state renders cost about the same as
  today (4–8 ms against a 9 ms median). What changes is the removal of the web
  process and its ~408 MB of helpers, and of the black frame composited between
  documents.
- **The spike did not build these:** text selection, find in page, an
  accessibility tree, painting only what is visible (the whole document went
  into one texture capped at 30,000 px), and remote images once a sender is
  allowed. #1547 lists them. They are requirements below, because a renderer
  that lacks them is a regression, however well it lays out a table.

This spec names that direction and not the engine. The plan chooses the engine
and is measured against the requirements here. If the engine replaces the
current one, it replaces it: the spike's own conclusion was that shipping two
reading engines is worse than either one, and ADR 0032's rendering half is then
amended or superseded on this branch.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Every message is legible in dark mode (Priority: P1)

A user with the app in dark mode opens any message: a colleague's reply, a
receipt, a newsletter, a message written in a twenty-year-old client. They can
read every word of it. No text is ever dark on dark or light on light.

**Why this priority**: This is the defect the user reported, and it defeats the
point of a mail client. A message that cannot be read is worse than one that
looks plain.

**Independent Test**: Render every message in the rendering corpus in dark
mode and measure contrast between each text run and the colour actually
painted behind it. Delivers value with no change to how any engine lays
anything out.

**Acceptance Scenarios**:

1. **Given** dark mode and a message whose sender set dark text and a white
   page canvas, **When** it renders, **Then** the text is legible against what
   is painted behind it. The sender's canvas is either kept, or text and
   canvas are adapted together. Never one without the other.
2. **Given** dark mode and a message that sets a text colour and no background
   at all, **When** it renders, **Then** that text meets the legibility
   threshold (FR-012) against the dark ground.
3. **Given** dark mode and a message whose sender supplied their own
   dark-mode styling, **When** it renders, **Then** the sender's dark-mode
   styling is what the user sees.
4. **Given** a message open while the user switches the app between light and
   dark, **When** the switch happens, **Then** the body follows within one
   frame, keeps its scroll position, and agrees with the app's theme.
5. **Given** plain-text mail in either theme, **When** it renders, **Then** it
   uses the app's own reading colours, exactly as today.

---

### User Story 2 - See the message the way its sender built it (Priority: P1)

A user opens a newsletter, a receipt, a shipping notice, a calendar invite, a
formatted announcement. The columns, colours, buttons, logos and spacing are
where the sender put them, as they would be in a mainstream mail client.

**Why this priority**: The user named the other half of the problem: layout
removal "hasnt worked that well". Most mail a person receives is designed
HTML. Flattening it loses meaning, not just decoration: which button is
primary, which figure is the total, which block is the footer.

**Independent Test**: Render the rendering corpus and compare each message
against its reviewed reference image. Delivers value without dark mode
(light theme only).

**Acceptance Scenarios**:

1. **Given** a message built from nested layout tables with widths,
   alignment, cell padding and background colours, **When** it renders,
   **Then** it matches its reference within the stated tolerance (SC-002).
2. **Given** a message whose `<style>` block targets its own classes and ids,
   **When** it renders, **Then** those rules apply to the elements they name
   (#1545).
3. **Given** legacy markup (`<font color face size>`, `<center>`, body
   `bgcolor`, `text` and `link` attributes, `background` on tables), **When**
   it renders, **Then** it is presented as those attributes intend.
4. **Given** a message whose images are attached to the message itself (inline
   parts referenced by `cid:`, or embedded `data:` images), **When** it
   renders, **Then** those images appear in place at their intended size, with
   no consent prompt, because nothing left the machine.
5. **Given** a bulk newsletter, **When** it opens, **Then** it opens in its
   sender's layout, and the simplified reading presentation is one action
   away. [NEEDS CLARIFICATION: Should bulk mail open in its original layout by
   default, with the simplified Reader view opt-in, or keep opening in Reader
   view as it does today?]
6. **Given** a message whose layout is wider than the pane, **When** it
   renders, **Then** it is readable without the pane scrolling sideways (001
   FR-026). Designs meant to shrink do shrink, and fixed-width designs scroll
   inside their own box.

---

### User Story 3 - Rendering cannot betray the reader (Priority: P1)

A user opens hostile or careless mail: a tracking pixel in CSS, a script, a
form, a 40,000-pixel-tall message, a malformed image, an attempt to overlay the
app's own buttons. Nothing leaves the machine, nothing runs, and the app stays
responsive and recognisably itself.

**Why this priority**: Restoring fidelity is only acceptable because the
renderer structurally cannot fetch or execute. That property has to be proven
from the first commit that renders a sender's markup in the new renderer, not
bolted on later.

**Independent Test**: Render the hostile fixtures against a listening local
socket and a resource-use budget. Assert zero connection attempts, zero
executed script, bounded time and memory, and an intact app.

**Acceptance Scenarios**:

1. **Given** a message that names remote resources every way HTML and CSS
   allow (image, background, font, stylesheet import, list marker, cursor,
   conditional style), **When** it renders for a sender who is not allowed,
   **Then** no connection is attempted to any of them.
2. **Given** a message carrying script in any form (element, event attribute,
   `javascript:` link), **When** it renders or is clicked, **Then** nothing
   executes and no such link is launched.
3. **Given** a malformed, truncated or enormous body or image, **When** it
   renders, **Then** the app does not crash or freeze. The message renders
   what it can, or falls back to its plain-text form with a notice.
4. **Given** a message that tries to position content outside its own box or
   to imitate the app's chrome, **When** it renders, **Then** it stays
   within its bounds (001 FR-020, FR-021, FR-025).

---

### User Story 4 - Read the way you read everywhere else (Priority: P2)

A user selects a sentence and copies it, searches the open message for a word,
clicks a link and sees where it goes before it opens, zooms in on small print,
scrolls a very long message with keyboard or trackpad, and uses a screen reader
to hear the body.

**Why this priority**: These are behaviours today's web-engine reader provides
for free and a disconnected renderer does not. The spike built none of them.
They are P2 only because Stories 1–3 can be demonstrated without them.
**The new renderer MUST NOT replace today's reader for users until this story
is complete** (FR-030).

**Independent Test**: With a long, styled fixture open, select across
paragraphs and table cells, copy, find a word, activate a link by mouse and
keyboard, zoom, and read the body with the platform screen reader.

**Acceptance Scenarios**:

1. **Given** an open message, **When** the user drags across text spanning
   several blocks or table cells, **Then** that text is selected visibly and
   copies as text in reading order.
2. **Given** an open message, **When** the user searches for a word, **Then**
   every occurrence is highlighted and the view moves between them.
3. **Given** a link, **When** the user hovers or focuses it, **Then** its real
   destination is shown. **When** they activate it, **Then** only web and mail
   links open, in the system handler (spike behaviour, kept).
4. **Given** a message many screens long, **When** the user scrolls to the
   end, **Then** all of it is present and scrolling stays smooth. There is no
   height at which content is cut off.
5. **Given** the platform screen reader, **When** the body has focus, **Then**
   its text, headings, links, lists and images' alternative text are exposed
   in reading order.
6. **Given** the user zooms the body, **When** it re-renders, **Then** text
   and layout scale together and the chrome is unaffected.

---

### User Story 5 - Show a trusted sender's remote images (Priority: P3)

A user who has allowed a sender's remote images, or chose "show once", sees
those images. The renderer still has no network of its own.

**Why this priority**: Per-sender allowing exists today (001 FR-005, FR-022)
and must not silently stop working. It is P3 because most of the value in
this spec comes from images attached to the message, which need no network.
[NEEDS CLARIFICATION: Must allowed remote images keep working in the first
landing of this branch, or may that landing show only in-message images, with
remote images following on the same branch before it merges?]

**Independent Test**: Allow a sender, open their message against a local
listener serving an image, and see the image. Revoke, reopen, and see a
placeholder with zero connections.

**Acceptance Scenarios**:

1. **Given** an allowed sender, **When** their message renders, **Then** its
   remote images appear. They are fetched by the application on the user's
   behalf and handed to the renderer, not fetched by the renderer.
2. **Given** a sender not allowed, **When** their message renders, **Then**
   remote images take their intended space as placeholders, so the layout does
   not collapse, and the blocked-images banner is shown (001 FR-005).
3. **Given** an allowed sender and no network, **When** their message renders,
   **Then** it renders at once with placeholders and does not wait (001
   FR-032).

### Edge Cases

- **A sender's canvas colour sits on an element the sanitizer used to drop**
  (`<body>`, `<html>`, a wrapping `<center>`): it becomes the message box's
  canvas.
- **Sender dark-mode styling that is itself illegible**: the legibility floor
  (FR-012) still holds. A sender's dark CSS is honoured, not trusted.
- **A message that is a single image, such as a scanned flyer** (no text to
  adapt): shown as is in both themes, on the sender's canvas if one is
  declared.
- **Mixed messages** (a personal reply quoting a designed newsletter): each
  part is legible, and the quoted part keeps its own canvas inside the fold.
- **Transparent images** (logos drawn for a white page) in dark mode: they
  stay visible, because the canvas they were designed for is kept behind
  them, or they are given one.
- **A `cid:` reference to a part that is missing or not yet downloaded**: a
  sized placeholder, never a broken-image glyph that shifts the layout later.
- **Images in formats the renderer cannot decode, or of absurd dimensions**: a
  placeholder, bounded decode cost, no crash (#1501).
- **Right-to-left and mixed-direction text, CJK, emoji, and fonts the system
  lacks**: shaped and laid out correctly, with a fallback font rather than
  missing glyphs.
- **Messages whose only HTML is a wrapper around plain text** (`<pre>`, or
  `<div>` with line breaks): treated as correspondence and fully legible.
- **High-contrast mode, and the reduced-motion preference**: high contrast
  raises the legibility floor. Reduced motion needs no animation, and the
  renderer adds none.
- **Printing a message**: out of scope here (see Assumptions). Nothing in this
  spec may make it harder later.

## Requirements *(mandatory)*

Inherited unchanged from `specs/001-conversation-reading-pane`: FR-019 to
FR-027 (fidelity, containment, no remote fetch without consent, no script,
chrome distinguishable, no sideways pane scroll, source reachable), FR-029 (no
blank frame between messages), FR-030 (bounded resources) and FR-032 (never
wait on the network). The requirements below add to those. Where they tighten
one, they say so.

### Functional Requirements

**The renderer is disconnected**

- **FR-001**: The component that turns a message into pixels MUST be incapable
  of opening a network connection. This MUST be a property of what the
  component is built from, not a setting. A test MUST prove that no code path
  in the renderer can reach a socket (for example, by the dependency graph)
  as well as observe none being opened.
- **FR-002**: The renderer MUST NOT execute script in any form: elements,
  event attributes, `javascript:` URLs, or CSS expressions. Unlike today's
  reader, this includes Postio's own script. Behaviour that used script (the
  conversation rail's position, scroll-to-message) MUST be provided without
  it.
- **FR-003**: The renderer's only sources of resources MUST be a closed set
  supplied by the application: the message's own parts (by Content-ID and by
  Content-Location), embedded `data:` images, the application's bundled fonts
  and system fonts, and remote images the application fetched under FR-025.
  A resource outside that set MUST resolve to nothing, and the outcome is
  counted, not silently lost.
- **FR-004**: A message part MUST resolve only within the message that
  referenced it. One message in a conversation MUST NOT be able to name
  another's parts (tightens 001 FR-020).

**Fidelity**

- **FR-005**: Markup MUST be kept unless removing it serves containment,
  privacy or no-script, and every removal MUST be listed with its reason (001
  FR-019b, now applied to elements and attributes as well as CSS properties).
  In particular, `class` and `id` MUST survive, so that a sender's stylesheet
  matches its own elements (#1545).
- **FR-006**: The attributes and rules a sender puts on the page itself
  (`<body>` and `<html>`: `bgcolor`, `background` colour, `text`, `link`,
  `style`, and `body`/`html` rules in a stylesheet) MUST be applied to that
  message's own box. That box is its canvas.
- **FR-007**: Legacy presentational markup still common in mail MUST be
  honoured: `<font>` (`color`, `face`, `size`), `<center>`, table and cell
  `background` colour and `bgcolor`, `border`, `cellpadding`, `cellspacing`,
  `align`, `valign`, and `width` and `height` on tables, cells and images.
- **FR-008**: A sender's responsive rules (`@media` by width) MUST be
  evaluated against the width the message actually has in the pane, not
  against the window or a fixed size.
- **FR-009**: A sender's fonts MUST resolve to a bundled or installed font by
  family name. Where none matches, the sender's generic family (serif,
  sans-serif, monospace) is honoured. No font is ever downloaded (tightens 001
  FR-022).
- **FR-010**: The renderer MUST NOT rely on browser defaults it lacks. Elements
  that are not displayed in a browser (document head, title, metadata) MUST
  NOT appear, and a sender's `<title>` text MUST NOT leak into the body.

**Dark mode and legibility**

- **FR-011**: The body MUST take its theme from the application's own theme
  state, the same source the chrome uses, and follow every change to it while
  open, including high contrast.
- **FR-012**: In every theme, every run of text the renderer paints MUST meet
  a minimum contrast of **4.5:1** (WCAG AA, body text) against the colour
  actually painted behind it. Where a sender's colours would fall below that,
  the renderer MUST adjust the text colour and nothing else about the
  sender's design. In high-contrast mode the floor is **7:1**.
- **FR-013**: The presentation of a sender-styled message in dark mode MUST
  follow one rule, stated and testable, chosen from these in priority order:
  (a) if the sender supplied dark-mode styling, use it, subject to FR-012;
  (b) otherwise [NEEDS CLARIFICATION: For a designed message with no dark
  styling of its own, should dark mode keep the sender's light canvas as a
  "sheet of paper" inside the dark app, or recolour the message into dark
  tones (as some webmail clients do), or keep the paper and offer recolouring
  per message?];
  (c) messages that set only text colours and no backgrounds adapt to the
  theme under FR-012.
- **FR-014**: Whatever rule FR-013 applies MUST apply to text and background
  together. A sender's text colour MUST NEVER be painted against a background
  the sender did not intend without FR-012's adjustment.
- **FR-015**: Images MUST NOT be inverted or recoloured. Where an image's
  intended canvas has been replaced by a dark one, the image MUST keep a
  backing of its intended canvas colour, so that transparent artwork stays
  visible.
- **FR-016**: Plain-text mail and the app's own words inside the body
  (placeholders, notices, quote folds) MUST use the reading palette. That
  palette's dark values MUST be designed values from the design system
  (#1588), not values derived by formula.

**Reading affordances** (the renderer must supply these itself)

- **FR-017**: Users MUST be able to select text with pointer and keyboard,
  across blocks and table cells, with visible selection, and copy it as text
  in reading order.
- **FR-018**: Users MUST be able to find text within the open message or
  conversation, with every match highlighted and next/previous navigation,
  through a registry command (Constitution II).
- **FR-019**: Links MUST show their real destination on hover and on focus, be
  reachable by keyboard, and open only `http`, `https` and `mailto` targets,
  in the system handler, on deliberate activation.
- **FR-020**: The body MUST expose an accessibility tree to the platform
  screen reader, covering text, headings, links, lists, tables and image
  alternative text in reading order.
- **FR-021**: Users MUST be able to zoom the body independently of the app
  chrome, through registry commands, and the zoom persists as a preference.
- **FR-022**: A message of any length MUST be fully present and scrollable. No
  content may be cut off at any height. Memory held MUST NOT grow with the
  message's pixel height: only what is near the viewport is kept painted.

**Robustness and cost**

- **FR-023**: Rendering a message MUST NOT be able to crash, hang or block the
  interface. Work is bounded in time and memory. If rendering fails or
  exceeds its bound, the message falls back to its plain-text form with a
  notice stating that the original could not be shown and offering the source
  (001 FR-027).
- **FR-024**: Decoding an image MUST be bounded by its declared and actual
  dimensions and byte size. An over-limit image becomes a placeholder (#1501).

**Remote images**

- **FR-025**: When the user has allowed a sender, or chosen to show once,
  remote images MUST be fetched by the application outside the renderer,
  only for that message, and supplied through FR-003's closed set. The
  renderer itself stays disconnected (FR-001). Allowing a sender MUST NOT load
  remote fonts, stylesheets or other non-image resources.
- **FR-026**: A blocked or not-yet-arrived remote image MUST occupy its
  declared size, so that the layout does not jump when it arrives.

**One engine**

- **FR-027**: The reading pane MUST render bodies through a single renderer.
  When this branch lands, there MUST be no second reading engine kept behind a
  flag or as a fallback. FR-023's fallback is plain text, not another engine.
- **FR-028**: The renderer MUST paint the conversation as one surface (ADR
  0032's intent), with each message's box, canvas and styling confined to
  itself (001 FR-020).
- **FR-029**: Moving between messages, and switching theme, MUST NOT show a
  blank, black or unpainted frame (001 FR-029, now also on theme change).
- **FR-030**: User Stories 1–4 MUST be complete before the new renderer
  replaces today's reader for users. Parity in reading affordances is a
  condition of the switch, not a follow-up.

### Key Entities

- **Rendering corpus**: the set of fixture messages that defines "renders
  well". It covers designed newsletters, transactional mail, legacy-client
  mail, plain-text-in-HTML, dark-mode-aware mail, hostile mail, and
  international text. Each entry has a reviewed reference image per theme.
  It extends the `.eml` corpus. Every address in it uses a reserved domain,
  and no real person's mail may be used (Constitution VI).
- **Message canvas**: the box one message renders into. It carries that
  message's resolved background, text colour and styling scope, and is the
  unit of containment, theme adaptation and part resolution.
- **Resource table**: the closed set of resources a message may use (its own
  parts, embedded images, fonts, and consented remote images the app
  fetched). It is built before rendering, and every lookup outside it is
  counted.
- **Refusal list**: the enumerable list of elements, attributes, properties
  and at-rules removed or neutralised, each with its reason (containment,
  privacy, no-script). Tests walk it.
- **Theme adaptation rule**: the single stated rule by which a message's
  colours are presented in the current theme (FR-013), together with the
  contrast floor (FR-012).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In dark, light and high-contrast themes, **100%** of text runs
  across the rendering corpus meet the contrast floor (4.5:1, or 7:1 in high
  contrast) against their painted background. Zero illegible messages.
- **SC-002**: At least **95%** of designed messages in the rendering corpus
  match their reviewed light-theme reference: columns in the same places,
  images in place at intended size, colours as specified. Every mismatch is
  listed with its cause. The remainder are cosmetic (for example, a missing
  border), and none of them loses content.
- **SC-003**: **Zero** network connection attempts by the renderer across the
  whole corpus, including the hostile fixtures. With remote images allowed,
  the only connections are the application's own fetches of the image URLs
  of the allowed message.
- **SC-004**: **Zero** crashes or hangs across the corpus plus a malformed and
  oversized set. Every message either renders or falls back to plain text,
  within a bounded time.
- **SC-005**: A typical message is on screen within the interaction budget
  after it is chosen (< 16 ms warm, and never a blank frame), and a theme
  switch repaints an open message within one frame.
- **SC-006**: Opening the reader no longer starts a separate rendering
  process. The memory the reading pane holds for a conversation is bounded,
  and it does not grow with message height or with the number of messages
  viewed.
- **SC-007**: Select-and-copy, find, link activation, zoom and screen-reader
  reading each succeed on every text-bearing message in the corpus.
- **SC-008**: The maintainer, using the app daily in dark mode for one week
  after the switch, reports no message they could not read. The first
  dark-on-dark report is treated as a defect against SC-001's corpus: it is
  added as a fixture, not answered with a workaround.

## Assumptions

- **The reading pane spec (001) stands.** This spec replaces nothing in it
  except where a requirement above says it tightens one.
- **The engine is the plan's call.** The Blitz spike is the evidence for a
  disconnected in-process renderer, and the plan's first job is to confirm it
  against FR-001 to FR-030, including the affordances the spike did not
  build. The plan also has to answer the spike's note that a shipped client
  "vendors its own" renderer crates instead of taking published ones. If the
  plan finds a renderer that meets the requirements better, the requirements
  hold and the engine changes.
- **ADRs.** This spec inherits ADR 0020 (bodies and parts are local), ADR
  0023 (bundled fonts) and ADR 0039 (the composer is native, so the reader is
  the only web-content consumer left). It amends ADR 0032 if the rendering
  mechanism changes. The branch also absorbs #1547's list, and #1545 is
  settled by FR-005. Neither needs a separate ADR.
- **Scope is Linux (v1).** The renderer MUST NOT preclude the macOS frontend,
  which today shares the reader document (ADR 0019), but no macOS work is in
  this spec.
- **Printing and "save as PDF" are out of scope.** So are editing, and
  rendering attachments other than inline images (PDF preview and so on).
- **Replies keep quoting what ADR 0033 says they quote.** Changing the
  renderer does not change what a reply contains.
- **No backwards compatibility.** Stored bodies are re-rendered from source.
  Nothing about the old rendering needs to be preserved.
- **Fixtures are synthetic.** Designed-mail fixtures are rebuilt by hand in
  the shape of real campaigns, with reserved domains and invented brands. The
  spike could not copy a live store, and this spec does not ask anyone to.
