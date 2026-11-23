import Foundation
import Observation
import PostioFFI

/// `focusPlaces` answers from the last read held in memory, so it is cheap on every keystroke.
public protocol PlacesEngine: AnyObject {
    func focusPlaces(_ filter: String) -> [PlaceEntryFfi]
    func focusOpenPlace(_ token: UInt64)
    func focusPlacesPlaceholder() -> String
    func binding(for command: String) -> String?
}

extension PostioSession: PlacesEngine {}

/// Places, names, counts, marks and footers are the engine's, read once per opening. This holds the
/// filter, the highlight and whether the popover is up. No AppKit (#1264).
@MainActor
@Observable
public final class PlacesModel {
    public enum Change: Equatable {
        case open
        case entries
        case place
    }

    public enum Mark: Equatable {
        case symbol(String)
        case dot(LabelColour?)
    }

    public struct Entry: Equatable, Identifiable {
        public let id: UInt64
        public let heading: String?
        public let name: String
        public let count: String?
        public let mark: Mark
        public let cap: String?
        public let footer: String
    }

    public private(set) var isOpen = false
    public private(set) var filter = ""
    public private(set) var entries: [Entry] = []
    public private(set) var highlighted: Int?
    public private(set) var placeholder = ""
    public private(set) var placeName = "Inbox"

    public var footer: String {
        highlighted.map { entries[$0].footer } ?? "Esc close"
    }

    @ObservationIgnored private let engine: PlacesEngine
    /// Only a highlight the arrows moved is kept when the places are read again.
    @ObservationIgnored private var moved = false

    public init(engine: PlacesEngine) {
        self.engine = engine
    }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case .focusOpenPlaces:
            isOpen = true
            filter = ""
            moved = false
            placeholder = engine.focusPlacesPlaceholder()
            read(keeping: nil)
            return .open
        case .focusPlacesChanged:
            guard isOpen else { return nil }
            read(keeping: moved ? highlighted.map { entries[$0].name } : nil)
            return .entries
        case let .focusPlace(name):
            placeName = name
            return .place
        default:
            return nil
        }
    }

    public func filterChanged(_ text: String) {
        guard isOpen, text != filter else { return }
        filter = text
        moved = false
        read(keeping: nil)
    }

    public func move(by delta: Int) {
        guard !entries.isEmpty else { return }
        moved = true
        highlighted = min(max((highlighted ?? 0) + delta, 0), entries.count - 1)
    }

    @discardableResult
    public func openHighlighted() -> Bool {
        guard isOpen, let highlighted else { return false }
        open(entries[highlighted].id)
        return true
    }

    public func open(_ token: UInt64) {
        guard isOpen else { return }
        close()
        engine.focusOpenPlace(token)
    }

    /// `true` the first time, when the keyboard must go back to the list.
    @discardableResult
    public func close() -> Bool {
        guard isOpen else { return false }
        isOpen = false
        filter = ""
        entries = []
        highlighted = nil
        return true
    }

    private func read(keeping name: String?) {
        var section: String?
        entries = engine.focusPlaces(filter).map { place in
            let heading = place.section == section ? nil : place.section
            section = place.section
            return Entry(
                id: place.token, heading: heading, name: place.name, count: place.count,
                mark: Self.mark(place.mark, name: place.name),
                cap: place.command.flatMap { KeyCapSpelling.cap(engine.binding(for: $0)) },
                footer: place.footer)
        }
        if entries.isEmpty {
            highlighted = nil
        } else {
            highlighted = name.flatMap { name in entries.firstIndex { $0.name == name } } ?? 0
        }
    }

    static func mark(_ mark: PlaceMarkFfi, name: String = "") -> Mark {
        switch mark {
        case let .role(role): return .symbol(symbol(for: role))
        case let .dot(color): return .dot(LabelColour.of(name: name, stored: color))
        // The system's filter glyph, so it is not a second Archive one row under the first.
        case .filtered: return .symbol(filteredSymbol)
        }
    }

    static let filteredSymbol = "line.3.horizontal.decrease.circle"

    static func symbol(for role: MailboxRoleFfi) -> String {
        switch role {
        case .inbox: return "tray"
        case .drafts: return "doc"
        case .sent: return "paperplane"
        case .archive: return "archivebox"
        case .trash: return "trash"
        case .junk: return "xmark.bin"
        case .flagged: return "flag"
        case .snoozed: return "clock"
        case .outbox: return "tray.and.arrow.up"
        case .regular: return "folder"
        }
    }
}
