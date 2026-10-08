import Foundation
import Observation
import PostioFFI

/// What the message window asks the engine for (specs/009-focus-macos US3).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `MessageWindowModel` without a store. Every call blocks on the
/// store, and `rawSource` may reach the server for bytes the person named,
/// so none of them is ever made on the main actor.
public protocol MessageWindowSource: AnyObject, Sendable {
    /// What surrounds the body: the chrome and the header block, composed.
    func focusMessageView(message: Int64, index: UInt32, total: UInt32) -> FocusMessageViewFfi
    /// The treated body, and the window's geometry for `mainWidth` (M1).
    func focusReaderDocument(
        message: Int64, remote: RemoteImagesFfi, chosen: TreatmentFfi?, mainWidth: Int32
    ) -> FocusReaderDocumentFfi
    /// The message as it came off the wire, for `v`.
    func rawSource(_ message: Int64) throws -> Data
}

extension PostioSession: MessageWindowSource {}

/// The message window's state, as the controller's intents leave it
/// (specs/009-focus-macos T070; `FocusOpenMessage` and `FocusReader`).
///
/// The controller (`crates/postio-focus`) decides which message is open,
/// what `j` and `k` reach, and what Back closes first. This holds what that
/// left on screen and fetches what to draw, off the main actor, dropping an
/// answer a later step has overtaken. It decides nothing about the list.
///
/// No AppKit: the window (`SecondaryWindowController`) and the SwiftUI
/// views both read it, and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class MessageWindowModel {
    /// The list row the window was opened from, as the controller said it:
    /// the row's message, and its place in the list.
    public struct Place: Equatable, Sendable {
        public let message: Int64
        public let index: UInt32
        public let total: UInt32
    }

    /// The row the window shows, or `nil` while it is closed.
    public private(set) var place: Place?
    /// The message on screen: the row's, or one of its conversation's that
    /// `[` or `]` stepped to.
    public private(set) var shown: Int64?
    /// What surrounds the body, once read.
    public private(set) var view: FocusMessageViewFfi?
    /// The body, its treatment and the window's geometry, once read.
    public private(set) var document: FocusReaderDocumentFfi?
    /// The treatment `O` chose for this message, or `nil` for the rule's
    /// (or the sender's remembered) choice.
    public private(set) var chosen: TreatmentFfi?
    /// Whether the last document is a switch of the one before, so the body
    /// keeps the place being read rather than going back to the top.
    public private(set) var keepsScroll = false
    /// The raw source `v` put in place of the content, once read.
    public private(set) var source: String?
    /// Whether `v` has put the source in place (it may still be reading).
    public private(set) var showingSource = false
    /// Whether the More menu is open.
    public private(set) var moreOpen = false
    /// Find in this message.
    public private(set) var find = FindInMessage()

    /// The main window's width the window opened beside: kept while it is
    /// open, so `j` and `k` never resize it (M1).
    public private(set) var mainWidth: Int32 = 0

    /// Whether the window shows a message.
    public var isOpen: Bool { place != nil }

    @ObservationIgnored private let engine: any MessageWindowSource
    @ObservationIgnored private let report: (Bool, Bool) -> Void
    @ObservationIgnored private let gate = RenderGate()
    @ObservationIgnored private let documentGate = RenderGate()
    @ObservationIgnored private var pending: [Task<Void, Never>] = []

    /// `report` is told More's and find's state whenever either changes:
    /// what the controller's Back closes first (`focus_reader_state`).
    public init(source: any MessageWindowSource, report: @escaping (Bool, Bool) -> Void) {
        engine = source
        self.report = report
    }

    /// `FocusOpenMessage`: show `message`, row `index` of `total`, opening
    /// the window or in place of what it shows. `true` when the window was
    /// closed and opens now, which is when its frame is set and the
    /// surface is reported open; a step answers `false`.
    @discardableResult
    public func open(message: Int64, index: UInt32, total: UInt32, mainWidth: Int32) -> Bool {
        let fresh = place == nil
        let next = Place(message: message, index: index, total: total)
        guard next != place || shown != message else { return false }
        if fresh { self.mainWidth = mainWidth }
        place = next
        let wasOpen = (moreOpen, find.isOpen)
        moreOpen = false
        find.close()
        if wasOpen != (false, false) { tell() }
        show(message)
        return fresh
    }

    /// The window closed, however it did: forget the message.
    public func closed() {
        _ = gate.begin()
        _ = documentGate.begin()
        let wasOpen = (moreOpen, find.isOpen)
        place = nil
        shown = nil
        view = nil
        document = nil
        chosen = nil
        source = nil
        showingSource = false
        keepsScroll = false
        moreOpen = false
        find.close()
        if wasOpen != (false, false) { tell() }
    }

    /// `FocusReader`: what the open message does as its own.
    public func apply(_ verb: ReaderVerbFfi) {
        guard place != nil else { return }
        switch verb {
        case .closeMore:
            moreOpen = false
            tell()
        case .showMore:
            moreOpen = true
            tell()
        case .closeFind:
            find.close()
            tell()
        case .findInMessage:
            find.open()
            tell()
        case .findNext:
            _ = find.next()
        case .findPrevious:
            _ = find.previous()
        case .switchTreatment:
            guard let shownTreatment = document?.treatmentShown else { return }
            chosen = shownTreatment == .paper ? .appColours : .paper
            showingSource = false
            source = nil
            loadDocument(keepingScroll: true)
        case .viewSource:
            if showingSource {
                closeSource()
            } else {
                openSource()
            }
        case let .stepThread(by):
            let target = by < 0 ? view?.earlier : view?.later
            guard let target else { return }
            show(target)
        }
    }

    /// Back from the source to the message (`v` again, or Esc).
    public func closeSource() {
        showingSource = false
        source = nil
    }

    /// The person typed in the find field.
    public func setFindQuery(_ text: String) {
        find.setQuery(text)
    }

    /// The web view's answer to the last find.
    public func found(_ any: Bool) {
        find.found(any)
    }

    /// Wait for whatever is being read: a test's way to see the result.
    public func settled() async {
        while let next = pending.first {
            await next.value
            pending.removeFirst()
        }
    }

    // MARK: reading

    private func tell() {
        report(moreOpen, find.isOpen)
    }

    /// Show `message` in the window: its chrome and its body, read again.
    private func show(_ message: Int64) {
        shown = message
        chosen = nil
        source = nil
        showingSource = false
        guard let place else { return }
        let token = gate.begin()
        let source = engine
        pending.append(
            Task { [weak self] in
                let view = await Task.detached {
                    source.focusMessageView(message: message, index: place.index, total: place.total)
                }.value
                guard let self, self.gate.isCurrent(token) else { return }
                self.view = view
            })
        loadDocument(keepingScroll: false)
    }

    private func loadDocument(keepingScroll: Bool) {
        guard let message = shown else { return }
        let token = documentGate.begin()
        let source = engine
        let chosen = chosen
        let width = mainWidth
        pending.append(
            Task { [weak self] in
                let document = await Task.detached {
                    source.focusReaderDocument(
                        message: message, remote: .blocked, chosen: chosen, mainWidth: width)
                }.value
                guard let self, self.documentGate.isCurrent(token) else { return }
                self.keepsScroll = keepingScroll
                self.document = document
            })
    }

    private func openSource() {
        guard let message = shown else { return }
        showingSource = true
        // A document still on its way is not drawn under the source.
        _ = documentGate.begin()
        let source = engine
        pending.append(
            Task { [weak self] in
                let text = await Task.detached { () -> String in
                    do {
                        let bytes = try source.rawSource(message)
                        // As GTK's source view: bytes the server sent are not
                        // promised to be UTF-8, and a lossy view of them is
                        // still the source.
                        return String(decoding: bytes, as: UTF8.self)
                    } catch let error as SessionError {
                        return Self.sentence(error)
                    } catch {
                        return String(describing: error)
                    }
                }.value
                guard let self, self.showingSource, self.shown == message else { return }
                self.source = text
            })
    }

    /// The boundary's sentence for `error`.
    nonisolated static func sentence(_ error: SessionError) -> String {
        switch error {
        case let .StoreUnavailable(message), let .KeyringLocked(message),
             let .StoreFromAnotherBuild(message), let .RuntimeUnavailable(message):
            return message
        }
    }
}
