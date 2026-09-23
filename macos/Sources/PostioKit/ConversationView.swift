import PostioFFI
import SwiftUI

/// The conversation in the reading pane: its header, and the whole thread as
/// one document (ADR 0032, #1595).
///
/// Selecting a thread does not navigate anywhere: the list stays a list and
/// every message of the conversation appears here, oldest first. It used to be
/// a stack of views with a web view per open message; it is one page now,
/// composed by the boundary with the same function GTK's pane uses, in one
/// web view whatever the thread's length (`ThreadDocumentView`).
///
/// What stays native is what is about the *conversation* or about the message
/// the list is on: the subject and its line, the verb bar that says what it
/// acts on (FR-008, FR-008a), and the notices for that message -- the same
/// split GTK's pane makes. Each message's own chrome is in the page.
///
/// Nothing here decides anything. The page, its anchors, the verbs' scope and
/// their words are the boundary's, and every verb is a registry command, so
/// `Reply` from the mouse and `e` from the keyboard are one code path, undo
/// included.
public struct ConversationView: View {
    private let session: PostioSession
    private let model: ConversationModel
    /// Run a command, and say which message it is for.
    ///
    /// `nil` means the conversation as a whole -- *Archive conversation* is
    /// about the thread. Everything else names a message: a verb that fell
    /// back to the *list* cursor replied to the thread's representative
    /// message rather than the one it was drawn under, the wrong recipient
    /// with nothing on screen to say so.
    private let run: (String, Int64?) -> Void
    /// The messages the reader asked to see as sent (`⌘O`).
    private let originals: [Int64]
    /// Bumped when what the page is made of may have changed -- see
    /// `ThreadDocumentView`.
    private let revision: Int
    private let page: UInt32
    private let pageToken: Int
    /// The message the list is on, which the notices above the page are
    /// about.
    private let showing: Int64?
    /// A verb a message offered inside the page.
    private let onVerb: (ThreadVerbFfi, ThreadAnchorFfi?) -> Void
    /// How wide the window is -- the rail's ladder is window widths.
    private let windowWidth: CGFloat
    /// The reader's own `⇧I`: no rail in this window (FR-047).
    private let railHidden: Bool

    /// Where each message is in the page on screen, and what it says about
    /// itself -- the decode caveat rides here.
    @State private var anchors: [ThreadAnchorFfi] = []
    /// The unsubscribe offer for the message the list is on, read once when
    /// it changes rather than per redraw.
    @State private var offer: UnsubscribeOfferFfi?
    /// The rail's rows, from the page that was drawn.
    @State private var railRows: [RailRowFfi] = []
    /// Whether the narrow window's index is open.
    @State private var showingIndex = false

    public init(
        session: PostioSession,
        model: ConversationModel,
        run: @escaping (String, Int64?) -> Void,
        originals: [Int64] = [],
        revision: Int = 0,
        page: UInt32 = 0,
        pageToken: Int = 0,
        showing: Int64? = nil,
        windowWidth: CGFloat = 0,
        railHidden: Bool = false,
        onVerb: @escaping (ThreadVerbFfi, ThreadAnchorFfi?) -> Void
    ) {
        self.session = session
        self.model = model
        self.run = run
        self.originals = originals
        self.revision = revision
        self.page = page
        self.pageToken = pageToken
        self.showing = showing
        self.windowWidth = windowWidth
        self.railHidden = railHidden
        self.onVerb = onVerb
    }

    /// Which presentation the rail gets in this window, or none.
    private var rail: RailPresentationFfi? {
        railPresentation(
            width: Int32(windowWidth), messages: UInt32(model.rows.count), hidden: railHidden
        )
    }

    /// The message the pane lands on when the conversation opens (FR-015).
    private var focusMessage: Int64? {
        model.focus.flatMap { focus in
            model.rows.indices.contains(Int(focus)) ? model.rows[Int(focus)].id : nil
        }
    }

    /// The newest message -- what the header's reply verbs answer (FR-008).
    private var latestMessage: Int64? { model.rows.last?.id }

    public var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            notices
            HStack(spacing: 0) {
                ThreadDocumentView(
                    source: session,
                    thread: model.conversation?.thread ?? 0,
                    originals: originals,
                    revision: revision,
                    focus: focusMessage,
                    request: model.documentRequest,
                    page: page,
                    pageToken: pageToken,
                    onVerb: onVerb,
                    onAnchors: { anchors = $0 },
                    onRail: { railRows = $0 },
                    onObserved: { model.observed(message: $0) },
                    onSettled: { model.settled($0) }
                )
                // After the body, as GTK's is. The reading measure never
                // gives up width to fund it (FR-043): below the ladder's
                // bottom step the column goes and the header's counter opens
                // the same index instead.
                if let rail, rail != .popover {
                    Divider()
                    ConversationRail(
                        rows: railRows, marked: model.marked, narrow: rail == .narrow,
                        choose: { model.choose($0) }
                    )
                    .frame(width: ConversationRail.width(narrow: rail == .narrow))
                }
            }
        }
        .accessibilityLabel(Pane.reader.label)
        .task(id: showing) {
            // Off the main actor: a row read, but not the drawing actor's.
            guard let showing else {
                offer = nil
                return
            }
            let session = session
            let facts = await Task.detached { session.messageFacts(showing) }.value
            guard !Task.isCancelled else { return }
            offer = facts.offer
        }
    }

    /// What is true of the message the list is on, above the page: the list
    /// it came from, and whether any of it could not be decoded. GTK's pane
    /// keeps the same two above its document, for the same message.
    @ViewBuilder
    private var notices: some View {
        if let showing, let offer {
            UnsubscribeBanner(offer: offer, message: showing, session: session)
                .padding(.horizontal, PostioTokens.space6)
                .padding(.top, PostioTokens.space3)
        }
        // A body that silently lost a part is exactly what ADR 0005 Q10's
        // omission rule is about; the wording is the boundary's.
        if let caveat = anchors.first(where: { $0.message == showing })?.caveat {
            Label(caveat, systemImage: "exclamationmark.triangle")
                .font(.callout)
                .foregroundStyle(.secondary)
                .padding(.horizontal, PostioTokens.space3)
                .padding(.vertical, PostioTokens.space2)
                .background(.quaternary.opacity(0.4), in: .rect(cornerRadius: 6))
                .padding(.horizontal, PostioTokens.space6)
                .padding(.top, PostioTokens.space3)
        }
    }

    // -- the header ---------------------------------------------------------

    private var header: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space2) {
            Text(model.subject)
                .font(.system(size: 22, weight: .semibold))
                .lineLimit(2)
                .accessibilityAddTraits(.isHeader)
            HStack(alignment: .firstTextBaseline) {
                Text(model.meta)
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: PostioTokens.space4)
                if rail == .popover {
                    counter
                }
                Button {
                    model.expandAll()
                } label: {
                    HStack(spacing: PostioTokens.space2) {
                        Text("Expand all")
                        if let chord = session.accelerator(for: "expand_all") {
                            Text(chord)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .help("Open every message in this conversation")
                Menu {
                    // The thread, not a message in it -- which is what
                    // `archive_thread` means and what the other two act on
                    // through the list's own cursor.
                    Button("Archive conversation") { run("archive_thread", nil) }
                    Button("Mark unread") { run("mark_unread", nil) }
                    Button("Flag") { run("flag", nil) }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .accessibilityLabel("More actions for this conversation")
            }
            verbs
        }
        .padding(.horizontal, PostioTokens.space6)
        .padding(.vertical, PostioTokens.space4)
    }

    /// Where the reader is, as `3/6` -- and the way to the index in a window
    /// too narrow for the column.
    private var counter: some View {
        let total = model.rows.count
        let at = (model.marked ?? model.focused) + 1
        return Button("\(at)/\(total)") { showingIndex.toggle() }
            .font(.system(.callout, design: .monospaced))
            .help("Message \(at) of \(total) — open the index")
            .accessibilityLabel("Message \(at) of \(total), open the index")
            .popover(isPresented: $showingIndex, arrowEdge: .bottom) {
                ConversationRail(
                    rows: railRows, marked: model.marked, narrow: false,
                    choose: { index in
                        model.choose(index)
                        showingIndex = false
                    }
                )
                .frame(width: ConversationRail.width(narrow: false), height: 320)
            }
    }

    /// Reply, Reply all, Forward and Archive, each saying what it will act on
    /// (FR-008, FR-008a): the first three the latest message, Archive the
    /// whole thread. The split and its words are the boundary's.
    private var verbs: some View {
        HStack(spacing: PostioTokens.space3) {
            ForEach(session.conversationActions(messages: UInt32(model.rows.count)), id: \.command) {
                action in
                verb(action)
            }
            Spacer()
        }
    }

    @ViewBuilder
    private func verb(_ action: ConversationActionFfi) -> some View {
        let target = action.wholeConversation ? nil : latestMessage
        let label = HStack(spacing: PostioTokens.space2) {
            Text(action.title)
            if let chord = session.accelerator(for: action.command) {
                Text(chord).opacity(0.75)
            }
        }
        let enabled = session.isAvailable(action.command, in: .reader)
        // Two branches rather than a style-erasing wrapper: `.buttonStyle`
        // takes a concrete type.
        if action.primary {
            Button(action: { run(action.command, target) }, label: { label })
                .buttonStyle(.borderedProminent)
                .disabled(!enabled)
                .help(action.description)
                .accessibilityLabel(action.description)
        } else {
            Button(action: { run(action.command, target) }, label: { label })
                .buttonStyle(.bordered)
                .disabled(!enabled)
                .help(action.description)
                .accessibilityLabel(action.description)
        }
    }
}

/// An open message: who it is from, when it arrived, its body, and the three
/// things you can do about it.
public struct ExpandedMessage: View {
    public let session: PostioSession
    public let row: RowFfi
    public let isLatest: Bool
    /// Whether this message's `Cc` list is open.
    ///
    /// On the model rather than in `@State` here — as everything a person
    /// turned on in this pane now is: a disclosure is a thing somebody opened
    /// and can be asserted, and #1259 is what happens when a piece of the
    /// header exists only inside a view nobody can look at from a test.
    public let showingCc: Bool
    public let collapse: () -> Void
    public let toggleCc: () -> Void
    /// Run a command against **this** message. See `ConversationView.run`.
    public let run: (String, Int64?) -> Void
    public let openSettings: () -> Void

    @State private var height: CGFloat = BodyHeight.minimum
    /// The three things this message says about itself, read once when it
    /// opens rather than on every redraw.
    ///
    /// Each is a point read through the boundary, and SwiftUI re-evaluates a
    /// `body` whenever anything it observes changes — so reading them inline
    /// meant three store round trips per open message per redraw, on the
    /// actor that draws. A conversation of eight is twenty-four. The 16 ms
    /// interaction budget is not a thing to spend on answers that cannot
    /// have changed since the message opened.
    @State private var notice: ReaderNoticeFfi?
    @State private var offer: UnsubscribeOfferFfi?
    @State private var caveat: String?
    /// Who the message was addressed to, read once with the rest.
    ///
    /// This was `session.recipients(row.id)` inside the view body — a
    /// blocking SQL round trip on the main actor per expanded message per
    /// redraw, and a conversation open redraws each message several times
    /// (height, notices). Twelve-plus round trips to draw three headers.
    @State private var addressed: RecipientsFfi?
    /// Whether this message is drawn as its sender wrote it.
    ///
    /// Per message and per view: reader view is on by default for bulk mail
    /// and leaving it is one gesture about one message, not a mode.
    /// Whether this message is drawn as its sender wrote it.
    ///
    /// Passed in rather than `@State`, because `⌘O` is a command and a
    /// command cannot reach view state — which is why the key did nothing
    /// while the menu item below it worked. `OriginalView` holds it, per
    /// message and per view.
    public let showingOriginal: Bool
    /// Whether this *message's* images are showing.
    ///
    /// Per message and reset with the pane, which is what "show once" means:
    /// the standing grant is the popover's, and it is the only thing that
    /// survives closing the conversation.
    ///
    /// Held outside this view, like `showingOriginal` beside it and for the
    /// same reason: `H` (*Render part once*) is a command, and a command
    /// cannot reach an `@State`. While this was one, the notice's button
    /// worked and the key did nothing.
    public let showingImages: Bool
    /// Whether the `⋯` menu offers to fold this message away.
    ///
    /// `false` in the single-message pane, where there is no conversation to
    /// fold it into and the message would simply vanish. The rest of the
    /// header is the same on purpose — one implementation of "who is this
    /// from", not two that drift.
    public var collapsible = true
    /// Where measured heights are remembered across visits.
    let heights: BodyHeights?
    public let toggleOriginal: () -> Void

    public init(
        session: PostioSession,
        row: RowFfi,
        isLatest: Bool,
        showingCc: Bool,
        showingOriginal: Bool,
        showingImages: Bool,
        collapsible: Bool = true,
        heights: BodyHeights? = nil,
        collapse: @escaping () -> Void,
        toggleCc: @escaping () -> Void,
        toggleOriginal: @escaping () -> Void,
        run: @escaping (String, Int64?) -> Void,
        openSettings: @escaping () -> Void
    ) {
        self.session = session
        self.row = row
        self.isLatest = isLatest
        self.showingCc = showingCc
        self.showingOriginal = showingOriginal
        self.showingImages = showingImages
        self.collapsible = collapsible
        self.heights = heights
        // Seeded from the cache, so a revisited message draws at the size
        // it measured last time instead of popping from the minimum. The
        // measurement still runs and corrects a stale number — a width
        // change makes one — so this is a starting point, never a claim.
        _height = State(initialValue: heights?.height(for: row.id) ?? BodyHeight.minimum)
        self.collapse = collapse
        self.toggleCc = toggleCc
        self.toggleOriginal = toggleOriginal
        self.run = run
        self.openSettings = openSettings
    }

    public var body: some View {
        HStack(alignment: .top, spacing: 0) {
            Rectangle()
                .fill(Color(nsColor: PostioTokens.colorAccent))
                .frame(width: 3)
            VStack(alignment: .leading, spacing: PostioTokens.space3) {
                header
                // Per message, never per pane: a conversation can hold back
                // pictures from three senders and one notice above them all
                // could not say whose.
                if let notice, !notice.allowed, !showingImages {
                    BlockedImagesNotice(
                        notice: notice,
                        session: session,
                        show: { run(Intercepted.renderPartOnce, row.id) },
                        openSettings: openSettings
                    )
                }
                // A body that silently lost a part is exactly what ADR
                // 0005 Q10's omission rule is about: a pane drawing an
                // incomplete message as though it were whole is making a
                // claim about somebody's mail. The wording is the
                // boundary's.
                if let caveat {
                    Label(caveat, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, PostioTokens.space3)
                        .padding(.vertical, PostioTokens.space2)
                        .background(.quaternary.opacity(0.4), in: .rect(cornerRadius: 6))
                        .fixedSize(horizontal: false, vertical: true)
                }
                // Also per message: a conversation can hold eight messages
                // from four lists. `PRODUCT.md` lists one-click unsubscribe
                // among the privacy features, and until this existed the
                // sentence was not true on a Mac.
                if let offer {
                    UnsubscribeBanner(offer: offer, message: row.id, session: session)
                }
                ReaderView(
                    source: session,
                    message: row.id,
                    remoteImages: remoteImages,
                    original: showingOriginal,
                    onHeight: { measured in
                        height = measured
                        heights?.remember(measured, for: row.id)
                    },
                    // The render's two by-products (#1589): what used to be
                    // two more boundary calls, each re-loading the body this
                    // render loads anyway.
                    onAnswers: { rendered, flagged in
                        notice = rendered
                        caveat = flagged
                    }
                )
                .frame(height: height)
                actions
            }
            .task(id: row.id) {
                // Keyed on the row: a pane reused for another message must
                // not keep the last one's banner.
                //
                // Off the main actor, and not as an optimisation flourish:
                // `readerNotice` loads the body and *renders* it to count
                // what was held back, and the actor that draws must not pay
                // for a render whose only output is a number. Reading these
                // inline is part of why moving between messages had a beat.
                let session = session
                let id = row.id
                // The row's own facts — no body load (#1589). The notice
                // and the caveat arrive with the document render below,
                // which is the one body load a message open pays for.
                let facts = await Task.detached { session.messageFacts(id) }.value
                guard !Task.isCancelled else { return }
                (offer, addressed) = (facts.offer, facts.recipients)
            }
            .padding(.horizontal, PostioTokens.space4)
            .padding(.vertical, PostioTokens.space4)
        }
        .background(Color.primary.opacity(0.03))
        Divider()
    }

    /// Whether this message's remote images may load.
    ///
    /// `Blocked` unless somebody said otherwise about *this* message or
    /// about its sender. The default is the product's, not a convenience:
    /// fetching one picture tells the sender the message was opened, when,
    /// and roughly where from.
    private var remoteImages: RemoteImagesFfi {
        if showingImages { return .allowed }
        // The cached notice, not a fresh read: this is evaluated on every
        // redraw, and the standing grant cannot change while the pane is
        // drawing.
        return notice?.allowed == true ? .allowed : .blocked
    }

    private var header: some View {
        HStack(alignment: .top, spacing: PostioTokens.space3) {
            Avatar(initials: row.initials)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: PostioTokens.space2) {
                    Text(row.from ?? "Unknown sender")
                        .fontWeight(.semibold)
                        .lineLimit(1)
                    if let address = row.fromAddress {
                        Text(address)
                            .font(.system(.callout, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Text(messageWhen(receivedAt: row.receivedAt))
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                recipients
            }
            Spacer(minLength: PostioTokens.space2)
            if isLatest {
                Text("latest")
                    .font(.system(.caption, design: .monospaced))
                    .padding(.horizontal, PostioTokens.space2)
                    .padding(.vertical, 2)
                    .overlay(
                        RoundedRectangle(cornerRadius: PostioTokens.radiusSm)
                            .stroke(Color(nsColor: PostioTokens.colorAccent), lineWidth: 1)
                    )
                    .foregroundStyle(Color(nsColor: PostioTokens.colorAccent))
            }
            Menu {
                if collapsible {
                    Button("Collapse", action: collapse)
                }
                Button("Reply") { run("reply", row.id) }
                Button("Forward") { run("forward", row.id) }
                Toggle(
                    "View original",
                    isOn: Binding(get: { showingOriginal }, set: { _ in toggleOriginal() })
                )
            } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .accessibilityLabel("More actions for this message")
        }
    }

    /// Who this message was addressed to (#1259).
    ///
    /// Every line already rendered by `postio_ui::reader::header`, which is
    /// what GTK's own reader draws from — so one recipient list reads the
    /// same on both. Read when the message is open rather than carried on
    /// the row: the list draws no recipients, and paying for them per row
    /// would load a mailbox's addresses to show one message's.
    ///
    /// Nothing at all when the message names nobody. A `To:` with no
    /// recipients after it is a line about the header rather than about the
    /// message.
    @ViewBuilder
    private var recipients: some View {
        if let lines = addressed {
            if let to = lines.to {
                Text(to)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            if let label = lines.ccLabel {
                // Folded until asked: the common message has one recipient,
                // and `Cc` costs nothing at all when there is none.
                Button(action: toggleCc) {
                    HStack(spacing: 2) {
                        Image(systemName: showingCc ? "chevron.down" : "chevron.right")
                        Text(label)
                    }
                }
                .buttonStyle(.borderless)
                .font(.callout)
                .foregroundStyle(.secondary)
                .accessibilityLabel(showingCc ? "Hide Cc recipients" : "Show Cc recipients")

                if showingCc, let cc = lines.cc {
                    Text(cc)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    /// Reply, Reply All, Forward and Archive — the engine's list, in the
    /// engine's order.
    ///
    /// Built from `readerActions()` rather than written out here, so this bar
    /// and GTK's cannot end up offering different verbs. `archive` was the
    /// one missing: `a` archived and nothing on screen said so or offered
    /// another way, which is the keyboard-only gap #1221 closed for the list.
    private var actions: some View {
        HStack(spacing: PostioTokens.space3) {
            ForEach(
                ReaderActionPlan.items(
                    from: session.readerActions(),
                    available: { session.isAvailable($0, in: .reader) },
                    bindings: { session.bindings(for: $0) }
                ),
                id: \.command
            ) { item in
                action(item)
            }
            Spacer()
        }
    }

    @ViewBuilder
    private func action(_ item: ReaderActionPlan.Item) -> some View {
        let label = HStack(spacing: PostioTokens.space2) {
            Text(item.title)
            if let chord = item.chord {
                Text(chord).opacity(0.75)
            }
        }
        // Two branches rather than a style-erasing wrapper: `.buttonStyle`
        // takes a concrete type, and the ceremony of hiding that behind one
        // is longer than saying it twice.
        if item.prominent {
            Button(action: { run(item.command, row.id) }, label: { label })
                .buttonStyle(.borderedProminent)
                .disabled(!item.enabled)
        } else {
            Button(action: { run(item.command, row.id) }, label: { label })
                .buttonStyle(.bordered)
                .disabled(!item.enabled)
        }
    }

}

/// The square of initials beside a sender.
struct Avatar: View {
    let initials: String

    var body: some View {
        Text(initials)
            .font(.system(.caption, design: .monospaced))
            .frame(width: 28, height: 28)
            .background(
                RoundedRectangle(cornerRadius: PostioTokens.radiusSm)
                    .fill(Color(nsColor: PostioTokens.colorAccent).opacity(0.18))
            )
            .accessibilityHidden(true)
    }
}

/// Read, unread, or sent by me — the canvas's three dots.
struct StateDot: View {
    let seen: Bool
    let mine: Bool

    var body: some View {
        Group {
            if mine {
                RoundedRectangle(cornerRadius: 1)
                    .stroke(Color.secondary, lineWidth: 1)
            } else {
                RoundedRectangle(cornerRadius: 1)
                    .fill(seen ? Color.secondary.opacity(0.5) : Color(nsColor: PostioTokens.colorAccent))
            }
        }
        .frame(width: 8, height: 8)
        .accessibilityHidden(true)
    }
}
