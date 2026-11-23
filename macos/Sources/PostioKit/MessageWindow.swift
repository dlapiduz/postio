import Foundation
import Observation
import PostioFFI

/// Every call blocks on the store, and `rawSource` may reach the server, so none runs on the main
/// actor.
public protocol MessageWindowSource: AnyObject, Sendable {
    func focusMessageView(message: Int64, index: UInt32, total: UInt32) -> FocusMessageViewFfi
    func focusReaderDocument(
        message: Int64, remote: RemoteImagesFfi, chosen: TreatmentFfi?, mainWidth: Int32
    ) -> FocusReaderDocumentFfi
    func rawSource(_ message: Int64) throws -> Data
    func openChoices(_ message: Int64, _ document: String) -> OpenChoicesFfi
    func openAttachment(_ message: Int64, _ attachment: Int64) throws -> String
    func saveAttachment(_ message: Int64, _ attachment: Int64, to: String) throws -> String
    func saveAttachments(_ message: Int64, folder: String) throws -> String
    /// `nil` when recorded, else why not. Writes, so never on the main actor.
    func activateUnsubscribe(_ message: Int64) -> String?
    func listMailOffer(_ message: Int64) -> ListMailOfferFfi?
    func clearListMail(_ message: Int64, _ delete: Bool) -> String?
}

extension PostioSession: MessageWindowSource {}

/// Holds what the controller left on screen and fetches what to draw off the main actor, dropping
/// answers a later step overtook. No AppKit, so iOS stays reachable (#1264).
@MainActor
@Observable
public final class MessageWindowModel {
    public struct Place: Equatable, Sendable {
        public let message: Int64
        public let index: UInt32
        public let total: UInt32
    }

    /// Chrome and body are read together so neither is drawn over the other message's.
    public struct Screen: Equatable, Sendable {
        public let message: Int64
        public let view: FocusMessageViewFfi
        public let document: FocusReaderDocumentFfi
    }

    /// Waits this long for a body's first paint; a body that never reports still shows.
    public static let drawPatience: Duration = .milliseconds(200)

    public private(set) var place: Place?
    public private(set) var shown: Int64?
    public private(set) var displayed: Int64?
    public private(set) var view: FocusMessageViewFfi?
    public private(set) var document: FocusReaderDocumentFfi?
    /// Drawn out of sight until its body is drawn (`bodyDrawn`), so `j` never shows a header over
    /// the last body.
    public private(set) var incoming: Screen?
    public private(set) var chosen: TreatmentFfi?
    /// The body keeps the place being read when the last document only switched treatment.
    public private(set) var keepsScroll = false
    public private(set) var source: String?
    public private(set) var showingSource = false
    public private(set) var moreOpen = false
    public private(set) var find = FindInMessage()
    public private(set) var unsubscribing = Unsubscribing()

    public private(set) var bar: ThreadBarFfi?
    public private(set) var kicker = ""
    public var walkPlace: String? {
        guard !kicker.isEmpty, let place = bar?.place, !place.isEmpty else { return nil }
        return place
    }
    public private(set) var olderBanner: ThreadBannerFfi?
    /// Overlays the foot; the window never moves by itself.
    public private(set) var newReply: ThreadBannerFfi?
    public private(set) var replyTo: String?
    public private(set) var matchBanner: MatchBannerFfi?
    public var searchWords = ""

    /// Kept while open so `j` and `k` never resize the window.
    public private(set) var mainWidth: Int32 = 0

    public var isOpen: Bool { place != nil }

    @ObservationIgnored private let engine: any MessageWindowSource
    @ObservationIgnored private let report: (Bool, Bool) -> Void
    @ObservationIgnored private let gate = RenderGate()
    @ObservationIgnored private let documentGate = RenderGate()
    @ObservationIgnored private var pending: [Task<Void, Never>] = []
    /// Set once, after `self` is.
    @ObservationIgnored private var dwell: DwellClock?
    @ObservationIgnored private let markRead: (Int64) -> Void
    @ObservationIgnored private let matchEdge: (Bool) -> Void
    /// The next message shows its first match going forward, its last going back.
    @ObservationIgnored private var arriving: Bool?
    /// The one place a chosen attachment leaves Postio.
    @ObservationIgnored private let handOff: (URL) -> Void
    public private(set) var notice: String?
    @ObservationIgnored private var noticeTask: Task<Void, Never>?
    @ObservationIgnored private var away = false
    /// Not started again for this message until another is shown.
    @ObservationIgnored private var settledRead: Int64?

    public init(
        source: any MessageWindowSource,
        report: @escaping (Bool, Bool) -> Void,
        markRead: @escaping (Int64) -> Void = { _ in },
        matchEdge: @escaping (Bool) -> Void = { _ in },
        handOff: @escaping (URL) -> Void = { _ in },
        schedule: @escaping @MainActor (TimeInterval, @escaping () -> Void) -> DwellClock.Cancellable =
            DwellClock.afterOnMain
    ) {
        engine = source
        self.report = report
        self.markRead = markRead
        self.matchEdge = matchEdge
        self.handOff = handOff
        dwell = DwellClock(schedule: schedule) { [weak self] message in
            self?.dwellRanOut(message)
        }
    }

    /// A message open behind another app is not being read: the clock stops and restarts on return.
    public func setAppActive(_ active: Bool) {
        away = !active
        if away {
            dwell?.stop()
        } else if let shown, settledRead != shown {
            armDwell()
        }
    }

    public func readStateSet() {
        settledRead = shown
        dwell?.stop()
    }

    private func armDwell() {
        guard place != nil, !away, let shown else {
            dwell?.stop()
            return
        }
        dwell?.cursorMoved(to: shown)
    }

    private func dwellRanOut(_ message: Int64) {
        settledRead = message
        guard place != nil, shown == message else { return }
        markRead(message)
    }

    /// `true` when the window was closed and opens now, which sets its frame; a step answers
    /// `false`.
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

    public func said(_ thread: FocusIntents.ThreadSaid) {
        switch thread {
        case let .bar(view): bar = view.chips.isEmpty ? nil : view
        case let .kicker(text): kicker = text
        case let .olderBanner(banner): olderBanner = banner
        case let .newReply(banner): newReply = banner
        case let .replyTo(text): replyTo = text
        case let .matchBanner(banner): matchBanner = banner
        }
    }

    public func closed() {
        bar = nil
        kicker = ""
        olderBanner = nil
        newReply = nil
        replyTo = nil
        matchBanner = nil
        arriving = nil
        dwell?.stop()
        settledRead = nil
        _ = gate.begin()
        _ = documentGate.begin()
        let wasOpen = (moreOpen, find.isOpen)
        place = nil
        shown = nil
        displayed = nil
        incoming = nil
        view = nil
        document = nil
        chosen = nil
        source = nil
        showingSource = false
        keepsScroll = false
        moreOpen = false
        find.close()
        unsubscribing.clear()
        notice = nil
        noticeTask?.cancel()
        if wasOpen != (false, false) { tell() }
    }

    public func unsubscribeState(of message: Int64) -> Unsubscribing.State {
        unsubscribing.state(of: message)
    }

    /// Only on deliberate activation: the boundary re-derives the list from the message id and
    /// refuses one that offers nothing. A repeat press in flight does nothing.
    public func unsubscribe() {
        guard let message = displayed, unsubscribing.begin(message) else { return }
        let source = engine
        pending.append(
            Task { [weak self] in
                let complaint = await Task.detached { source.activateUnsubscribe(message) }.value
                guard let self, self.place != nil else { return }
                self.unsubscribing.finish(message, complaint: complaint)
                guard complaint == nil else { return }
                let offer = await Task.detached { source.listMailOffer(message) }.value
                guard let offer, self.place != nil else { return }
                self.unsubscribing.offerSweep(
                    message,
                    .init(summary: offer.summary, button: offer.button, delete: offer.delete))
            })
    }

    /// `true` deletes, `false` archives, `nil` dismisses. One command on those messages, so one
    /// undo.
    public func clearListMail(delete: Bool?) {
        guard let message = displayed else { return }
        unsubscribing.answerSweep(message)
        guard let delete else { return }
        let source = engine
        pending.append(
            Task.detached { _ = source.clearListMail(message, delete) })
    }

    public func takeUpOffer() {
        guard let message = displayed,
            case .clearing(let sweep) = unsubscribing.state(of: message)
        else { return }
        clearListMail(delete: sweep.delete)
    }

    /// A failure is a sentence, never an empty file. Off the main actor: it may reach the server.
    public func openAttachment(_ attachment: Int64) {
        guard let message = displayed else { return }
        let source = engine
        pending.append(
            Task { [weak self] in
                let opened = await Task.detached { () -> Result<String, Error> in
                    Result { try source.openAttachment(message, attachment) }
                }.value
                guard let self, self.place != nil else { return }
                switch opened {
                case let .success(path):
                    self.handOff(URL(fileURLWithPath: path))
                case let .failure(error):
                    if case let PartsError.Refused(message) = error {
                        self.say(message)
                    } else {
                        self.say(String(describing: error))
                    }
                }
            })
    }

    public func offerAttachments(present: @escaping (OpenChoicesFfi) -> Void) {
        guard let message = displayed else { return }
        let source = engine
        let shown = document?.html ?? ""
        pending.append(
            Task { [weak self] in
                let choices = await Task.detached { source.openChoices(message, shown) }.value
                guard let self, self.displayed == message else { return }
                if let nothing = choices.nothing {
                    self.say(nothing)
                } else {
                    present(choices)
                }
            })
    }

    public func openLink(_ target: String) {
        guard place != nil, let url = URL(string: target) else { return }
        handOff(url)
    }

    public func saveAttachment(_ attachment: Int64, to file: URL) {
        guard let message = displayed else { return }
        let source = engine
        let path = file.path
        saving { try source.saveAttachment(message, attachment, to: path) }
    }

    public func saveAttachments(to folder: URL) {
        guard let message = displayed else { return }
        let source = engine
        let path = folder.path
        saving { try source.saveAttachments(message, folder: path) }
    }

    private func saving(_ save: @escaping @Sendable () throws -> String) {
        pending.append(
            Task { [weak self] in
                let saved = await Task.detached { () -> Result<String, Error> in
                    Result { try save() }
                }.value
                guard let self, self.place != nil else { return }
                switch saved {
                case let .success(words):
                    self.say(words)
                case let .failure(error):
                    if case let PartsError.Refused(message) = error {
                        self.say(message)
                    } else {
                        self.say(String(describing: error))
                    }
                }
            })
    }

    private func say(_ words: String) {
        notice = words
        noticeTask?.cancel()
        noticeTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(5))
            guard !Task.isCancelled else { return }
            self?.notice = nil
        }
    }

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
        case .nextMatch:
            arriving = nil
            find.nextWithin(searchWords)
        case .prevMatch:
            arriving = nil
            find.previousWithin(searchWords)
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
        }
    }

    public func keepForSender(_ keep: (String, TreatmentFfi) -> Void) {
        guard let document, let sender = document.sender else { return }
        keep(sender, document.treatmentShown)
        chosen = nil
        loadDocument(keepingScroll: true)
    }

    public func closeSource() {
        showingSource = false
        source = nil
    }

    public func setFindQuery(_ text: String) {
        find.setQuery(text)
    }

    public func found(_ any: Bool) {
        let ranOut = !any && find.request?.bounded == true
        let backwards = find.request?.backwards ?? false
        find.found(any)
        if ranOut {
            arriving = !backwards
            matchEdge(!backwards)
        }
    }

    public func settled() async {
        while let next = pending.first {
            await next.value
            pending.removeFirst()
        }
    }

    private func tell() {
        report(moreOpen, find.isOpen)
    }

    /// If `message` is on screen, reread in place keeping the treatment and the place being read;
    /// not under the source `v` put there (#1788).
    public func bodyLanded(_ message: Int64) {
        guard place != nil, shown == message else { return }
        if showingSource {
            loadView(message)
        } else {
            read(message, keepingScroll: displayed == message)
        }
    }

    public func networkReturned() {
        if let shown { bodyLanded(shown) }
    }

    public func bodyDrawn(_ message: Int64) {
        guard let incoming, incoming.message == message else { return }
        put(incoming)
    }

    private func show(_ message: Int64) {
        shown = message
        chosen = nil
        source = nil
        showingSource = false
        settledRead = nil
        armDwell()
        read(message, keepingScroll: false)
    }

    private func read(_ message: Int64, keepingScroll: Bool) {
        guard let place else { return }
        let token = gate.begin()
        let documentToken = documentGate.begin()
        let source = engine
        let chosen = chosen
        let width = mainWidth
        pending.append(
            Task { [weak self] in
                let (view, document) = await Task.detached {
                    async let view = source.focusMessageView(
                        message: message, index: place.index, total: place.total)
                    async let document = source.focusReaderDocument(
                        message: message, remote: .blocked, chosen: chosen, mainWidth: width)
                    return await (view, document)
                }.value
                guard let self, self.gate.isCurrent(token), self.documentGate.isCurrent(documentToken)
                else { return }
                let next = Screen(message: message, view: view, document: document)
                if self.displayed == nil || self.displayed == message {
                    self.put(next, keepingScroll: keepingScroll)
                } else {
                    self.incoming = next
                    self.pending.append(
                        Task { [weak self] in
                            try? await Task.sleep(for: Self.drawPatience)
                            self?.bodyDrawn(message)
                        })
                }
            })
    }

    private func put(_ screen: Screen, keepingScroll: Bool = false) {
        incoming = nil
        keepsScroll = keepingScroll
        displayed = screen.message
        view = screen.view
        document = screen.document
        arrive(at: screen.message)
    }

    private func arrive(at message: Int64) {
        let backwards = arriving.map { !$0 } ?? false
        arriving = nil
        guard !searchWords.isEmpty else { return }
        pending.append(
            Task { [weak self] in
                try? await Task.sleep(for: Self.drawPatience)
                guard let self, self.displayed == message, !self.showingSource, self.matchBanner != nil
                else { return }
                self.find.arrive(self.searchWords, backwards: backwards)
            })
    }

    private func loadView(_ message: Int64) {
        guard let place else { return }
        let token = gate.begin()
        let source = engine
        pending.append(
            Task { [weak self] in
                let view = await Task.detached {
                    source.focusMessageView(message: message, index: place.index, total: place.total)
                }.value
                guard let self, self.gate.isCurrent(token), self.displayed == message else { return }
                self.view = view
            })
    }

    private func loadDocument(keepingScroll: Bool) {
        guard let message = shown else { return }
        guard displayed == message else {
            read(message, keepingScroll: false)
            return
        }
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
                guard let self, self.documentGate.isCurrent(token), self.displayed == message else { return }
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
                        // Server bytes are not promised UTF-8; a lossy view is still the source.
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

    nonisolated static func sentence(_ error: SessionError) -> String {
        switch error {
        case let .StoreUnavailable(message), let .KeyringLocked(message),
             let .StoreFromAnotherBuild(message), let .StoreUnderAnotherKey(message),
             let .RuntimeUnavailable(message):
            return message
        }
    }
}
