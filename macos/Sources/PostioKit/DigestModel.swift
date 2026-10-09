import Foundation
import Observation
import PostioFFI

/// What the digest's window tells the engine (specs/009-focus-macos T114).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `DigestModel` without a store.
public protocol DigestEngine: AnyObject {
    /// The list page's row `index` was clicked.
    func focusDigestPoint(_ index: UInt32)
    /// The summary's reference `index` (in reading order) was clicked.
    func focusDigestReference(_ index: UInt32)
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
    /// The binding in force for `command`, for its keycap.
    func binding(for command: String) -> String?
}

extension PostioSession: DigestEngine {}

/// The registry commands the digest window's buttons are.
public enum DigestCommand {
    /// "Archive all 14" (`⇧A`).
    public static let archiveAll = "archive_thread"
    /// "Edit rule and cadence" (`d`).
    public static let rule = "digest_rule"
    /// The Summary and list tabs (Tab).
    public static let toggle = "toggle_digest_summary"
    /// "‹ Summary" (Esc), or the window's close by key.
    public static let back = "back"
    /// Open the focused reference's or row's email in place (Return).
    public static let open = "open_message"
    /// The email page's stepper: the digest's previous and next message.
    public static let previous = "prev_message"
    public static let next = "next_message"
}

/// What answering a `FocusConfirm` tells the engine.
public protocol ConfirmEngine: AnyObject {
    /// Yes to the question named `token`.
    func focusConfirmed(_ token: UInt64)
}

extension PostioSession: ConfirmEngine {}

/// A question the controller asks before doing what no single undo takes
/// back whole -- `D`, stop digesting a sender: its words, and the one
/// thing an answer does. Yes is `focus_confirmed(token)`; no is nothing,
/// and the controller forgets the question when the next one comes.
public struct ConfirmQuestion: Equatable {
    public let token: UInt64
    public let heading: String
    public let body: String
    /// The button that says yes; Cancel is the other.
    public let confirm: String
    /// Draw yes as destructive rather than as the default.
    public let destructive: Bool

    public init(_ confirm: ConfirmFfi) {
        token = confirm.token
        heading = confirm.heading
        body = confirm.body
        self.confirm = confirm.confirm
        destructive = confirm.destructive
    }

    /// Say the answer: only a yes reaches the controller.
    public func answer(_ yes: Bool, to engine: ConfirmEngine) {
        guard yes else { return }
        engine.focusConfirmed(token)
    }
}

/// The digest's window, as the controller's intents leave it
/// (specs/009-focus-macos T114, screens 22 and 23).
///
/// The controller (`crates/postio-focus/src/digest.rs`) keeps the page --
/// the summary by topic, the plain list, or an email in place -- the
/// focused reference and row, and what each key does; the words are
/// `postio_ui::digest`'s. This holds the last `FocusDigest` whole and hands
/// back what the pointer did. A click moves nothing here: the focus moves
/// when the next view says so.
///
/// What it does itself is read the email `view.email` names, when it names
/// a new one: its header (`focus_message_view`, placed among the digest's
/// rows) and its treated body (`focus_reader_document`), off the main actor,
/// dropping an answer for an email since left. The email page is never a
/// second window (M4): it is drawn in this one, at the same size.
///
/// No AppKit: the window (`PostioAppKit`) and the SwiftUI views read it,
/// and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class DigestModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// `FocusOpenDigest`: open the window.
        case open
        /// A new view: draw it whole.
        case redraw
        /// The controller closed it.
        case close
    }

    /// Whether the window is up.
    public private(set) var isOpen = false
    /// The delivery `FocusOpenDigest` named.
    public private(set) var delivery: Int64?
    /// The last view, whole; `nil` until the first lands.
    public private(set) var view: DigestViewFfi?
    /// The email page's header, once read.
    public private(set) var emailView: FocusMessageViewFfi?
    /// The email page's body, once read.
    public private(set) var emailDocument: FocusReaderDocumentFfi?
    /// The main window's width, which the body's column is sized from (M1).
    public var mainWidth: Int32 = 0

    @ObservationIgnored private let engine: DigestEngine
    @ObservationIgnored private let source: any MessageWindowSource
    @ObservationIgnored private let gate = RenderGate()
    @ObservationIgnored private var pending: [Task<Void, Never>] = []
    /// The email whose reads were last asked for.
    @ObservationIgnored private var reading: Int64?

    public init(engine: DigestEngine, source: any MessageWindowSource) {
        self.engine = engine
        self.source = source
    }

    // MARK: what is drawn

    /// The topic the focused reference is in: the card is drawn under it.
    public var focusedTopic: Int? {
        guard let view, let focused = view.focusedReference else { return nil }
        return view.topics.firstIndex { $0.statements.contains { $0.index == focused } }
    }

    public var archiveCap: String? { KeyCapSpelling.cap(view?.archiveKey) }
    public var ruleCap: String? { KeyCapSpelling.cap(view?.ruleKey) }
    public var tabCap: String? { KeyCapSpelling.cap(view?.tabKey) }
    public var backCap: String? { KeyCapSpelling.cap(view?.backKey) }
    public var cardCap: String? { KeyCapSpelling.cap(view?.card?.key) }

    /// The email page's stepper cap, previous then next ("k j").
    public var stepCap: String? {
        let caps = [DigestCommand.previous, DigestCommand.next].compactMap {
            KeyCapSpelling.cap(engine.binding(for: $0))
        }
        return caps.isEmpty ? nil : caps.joined(separator: " ")
    }

    /// The cited passage the email page highlights.
    public var highlight: String? { view?.email?.excerpt }

    // MARK: events

    /// Apply `event` if it is the digest's, and say what it changed; `nil`
    /// for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusOpenDigest(delivery):
            let fresh = !isOpen
            isOpen = true
            self.delivery = delivery
            if self.view?.delivery != delivery { forget() }
            return fresh ? .open : .redraw
        case let .focusDigest(view):
            let fresh = !isOpen
            isOpen = true
            delivery = view.delivery
            self.view = view
            follow(view.email, rows: view.rows)
            return fresh ? .open : .redraw
        case .focusCloseSurface(.digest):
            guard isOpen else { return nil }
            close()
            return .close
        default:
            return nil
        }
    }

    /// The window was closed by the toolkit -- its close button, ⌘W -- not
    /// by the controller: `true` once, when it was up, so the engine is told.
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        close()
        return true
    }

    /// Wait for the reads asked for so far: what a test, and the window's
    /// first showing, need.
    public func settled() async {
        while let next = pending.first {
            await next.value
            pending.removeFirst()
        }
    }

    // MARK: the pointer

    /// The list page's row `index` was clicked.
    public func point(_ index: Int) {
        engine.focusDigestPoint(UInt32(index))
    }

    /// The list page's row `index` was double-clicked: its email in place.
    public func open(_ index: Int) {
        engine.focusDigestPoint(UInt32(index))
        engine.invoke(DigestCommand.open)
    }

    /// The summary's reference `index` was clicked.
    public func reference(_ index: UInt32) {
        engine.focusDigestReference(index)
    }

    /// The card's "open the full email".
    public func openReference() {
        engine.invoke(DigestCommand.open)
    }

    public func archiveAll() { engine.invoke(DigestCommand.archiveAll) }
    public func editRule() { engine.invoke(DigestCommand.rule) }
    public func toggleTab() { engine.invoke(DigestCommand.toggle) }
    public func back() { engine.invoke(DigestCommand.back) }
    public func step(_ command: String) { engine.invoke(command) }

    // MARK: the email in place

    private func follow(_ email: DigestEmailFfi?, rows: [DigestLineFfi]) {
        guard let email else {
            if reading != nil { forgetEmail() }
            return
        }
        guard email.message != reading else { return }
        reading = email.message
        emailView = nil
        emailDocument = nil
        let message = email.message
        let index = UInt32(rows.firstIndex { $0.message == message } ?? 0)
        let total = UInt32(max(rows.count, 1))
        let width = mainWidth
        let source = source
        let token = gate.begin()
        pending.append(
            Task { [weak self] in
                let (header, document) = await Task.detached {
                    (
                        source.focusMessageView(message: message, index: index, total: total),
                        source.focusReaderDocument(
                            message: message, remote: .blocked, chosen: nil, mainWidth: width)
                    )
                }.value
                guard let self, self.gate.isCurrent(token) else { return }
                self.emailView = header
                self.emailDocument = document
            })
    }

    private func forgetEmail() {
        _ = gate.begin()
        reading = nil
        emailView = nil
        emailDocument = nil
    }

    private func forget() {
        view = nil
        forgetEmail()
    }

    private func close() {
        isOpen = false
        delivery = nil
        forget()
    }
}
