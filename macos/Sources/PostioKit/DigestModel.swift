import Foundation
import Observation
import PostioFFI

public protocol DigestEngine: AnyObject {
    func focusDigestPoint(_ index: UInt32)
    func focusDigestReference(_ index: UInt32)
    func invoke(_ id: String)
    func binding(for command: String) -> String?
}

extension PostioSession: DigestEngine {}

public enum DigestCommand {
    public static let archiveAll = "archive"
    public static let rule = "digest_rule"
    public static let toggle = "toggle_digest_summary"
    public static let back = "back"
    public static let open = "open_message"
    public static let previous = "prev_message"
    public static let next = "next_message"
}

public protocol ConfirmEngine: AnyObject {
    func focusConfirmed(_ token: UInt64)
}

extension PostioSession: ConfirmEngine {}

/// Yes is `focus_confirmed(token)`; no is nothing, and the controller forgets the question when the
/// next one comes.
public struct ConfirmQuestion: Equatable {
    public let token: UInt64
    public let heading: String
    public let body: String
    public let confirm: String
    public let destructive: Bool

    public init(_ confirm: ConfirmFfi) {
        token = confirm.token
        heading = confirm.heading
        body = confirm.body
        self.confirm = confirm.confirm
        destructive = confirm.destructive
    }

    public func answer(_ yes: Bool, to engine: ConfirmEngine) {
        guard yes else { return }
        engine.focusConfirmed(token)
    }
}

/// A click moves nothing here: the focus moves when the next view says so. It reads the email
/// `view.email` names when it names
/// a new one (header and treated body, off the main actor), dropping an answer for an email since
/// left. The email page is never
/// a second window (M4). No AppKit (#1264).
@MainActor
@Observable
public final class DigestModel {
    public enum Change: Equatable {
        case open
        case redraw
        case close
    }

    public private(set) var isOpen = false
    public private(set) var delivery: Int64?
    public private(set) var view: DigestViewFfi?
    public private(set) var emailView: FocusMessageViewFfi?
    public private(set) var emailDocument: FocusReaderDocumentFfi?
    public var mainWidth: Int32 = 0

    @ObservationIgnored private let engine: DigestEngine
    @ObservationIgnored private let source: any MessageWindowSource
    @ObservationIgnored private let gate = RenderGate()
    @ObservationIgnored private var pending: [Task<Void, Never>] = []
    @ObservationIgnored private var reading: Int64?

    public init(engine: DigestEngine, source: any MessageWindowSource) {
        self.engine = engine
        self.source = source
    }

    public var focusedTopic: Int? {
        guard let view, let focused = view.focusedReference else { return nil }
        return view.topics.firstIndex { $0.statements.contains { $0.index == focused } }
    }

    public var archiveCap: String? { KeyCapSpelling.cap(view?.archiveKey) }
    public var ruleCap: String? { KeyCapSpelling.cap(view?.ruleKey) }
    public var tabCap: String? { KeyCapSpelling.cap(view?.tabKey) }
    public var backCap: String? { KeyCapSpelling.cap(view?.backKey) }
    public var cardCap: String? { KeyCapSpelling.cap(view?.card?.key) }

    public var stepCap: String? {
        let caps = [DigestCommand.previous, DigestCommand.next].compactMap {
            KeyCapSpelling.cap(engine.binding(for: $0))
        }
        return caps.isEmpty ? nil : caps.joined(separator: " ")
    }

    public var highlight: String? { view?.email?.excerpt }

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

    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        close()
        return true
    }

    public func settled() async {
        while let next = pending.first {
            await next.value
            pending.removeFirst()
        }
    }

    public func point(_ index: Int) {
        engine.focusDigestPoint(UInt32(index))
    }

    public func open(_ index: Int) {
        engine.focusDigestPoint(UInt32(index))
        engine.invoke(DigestCommand.open)
    }

    public func reference(_ index: UInt32) {
        engine.focusDigestReference(index)
    }

    public func openReference() {
        engine.invoke(DigestCommand.open)
    }

    public func archiveAll() { engine.invoke(DigestCommand.archiveAll) }
    public func editRule() { engine.invoke(DigestCommand.rule) }
    public func toggleTab() { engine.invoke(DigestCommand.toggle) }
    public func back() { engine.invoke(DigestCommand.back) }
    public func step(_ command: String) { engine.invoke(command) }

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
