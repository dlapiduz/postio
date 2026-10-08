import Foundation
import Observation
import PostioFFI

/// What the folders popover asks the engine (specs/009-focus-macos T086).
///
/// `PostioSession` is the one real answer; the protocol lets a test drive
/// `PlacesModel` without a store. `focusPlaces` answers from the last read
/// held in memory, so it is cheap enough to ask on every keystroke.
public protocol PlacesEngine: AnyObject {
    /// The places whose names hold `filter`, in the popover's order.
    func focusPlaces(_ filter: String) -> [PlaceEntryFfi]
    /// Go to the place `token`.
    func focusOpenPlace(_ token: UInt64)
    /// What the filter says before anything is typed.
    func focusPlacesPlaceholder() -> String
    /// The binding in force for `command`, for a row's keycap.
    func binding(for command: String) -> String?
}

extension PostioSession: PlacesEngine {}

/// The folders and labels popover's state (specs/009-focus-macos T086,
/// screen 10), and the place the list shows (`FocusPlace`).
///
/// Which places there are, what each is called and counts, its mark and
/// its footer are the engine's (`postio_ui::places`, read once per opening
/// by the controller). This holds the filter typed, the highlight the
/// arrows move, and whether the popover is up; Return goes where the
/// highlighted row says, through the controller, which moves the list and
/// names the place back.
///
/// No AppKit: the popover (`PlacesPopover`) and its SwiftUI content
/// (`PlacesView`) both read it, and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class PlacesModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// Show the popover.
        case open
        /// The rows were read again.
        case entries
        /// The list shows another place: the strip's Inbox ▾ names it.
        case place
    }

    /// What sits before a place's name.
    public enum Mark: Equatable {
        /// A mailbox's SF Symbol, by its role.
        case symbol(String)
        /// A label's dot, in its own colour; `nil` for the secondary colour.
        case dot(LabelColour?)
    }

    /// One row of the popover.
    public struct Entry: Equatable, Identifiable {
        /// The place's token: what opening it hands back.
        public let id: UInt64
        /// The section heading drawn above it, where the section changes.
        public let heading: String?
        public let name: String
        public let count: String?
        public let mark: Mark
        /// The keycap of its direct key (`g i`), from the binding in force.
        public let cap: String?
        /// The footer's words while it is highlighted.
        public let footer: String
    }

    /// Whether the popover is up.
    public private(set) var isOpen = false
    /// What is typed in its filter.
    public private(set) var filter = ""
    public private(set) var entries: [Entry] = []
    /// The highlighted row's index in `entries`.
    public private(set) var highlighted: Int?
    /// The filter's placeholder.
    public private(set) var placeholder = ""
    /// The place the list shows: what Inbox ▾ says.
    public private(set) var placeName = "Inbox"

    /// The footer: what Return does on the highlighted row.
    public var footer: String {
        highlighted.map { entries[$0].footer } ?? "Esc close"
    }

    @ObservationIgnored private let engine: PlacesEngine

    public init(engine: PlacesEngine) {
        self.engine = engine
    }

    /// Apply `event` if it is the popover's or the strip's place; `nil`
    /// for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case .focusOpenPlaces:
            isOpen = true
            filter = ""
            placeholder = engine.focusPlacesPlaceholder()
            read(keeping: nil)
            return .open
        case .focusPlacesChanged:
            guard isOpen else { return nil }
            read(keeping: highlighted.map { entries[$0].name })
            return .entries
        case let .focusPlace(name):
            placeName = name
            return .place
        default:
            return nil
        }
    }

    /// The filter holds `text` now.
    public func filterChanged(_ text: String) {
        guard isOpen, text != filter else { return }
        filter = text
        read(keeping: nil)
    }

    /// ↑ or ↓, stopping at either end.
    public func move(by delta: Int) {
        guard !entries.isEmpty else { return }
        highlighted = min(max((highlighted ?? 0) + delta, 0), entries.count - 1)
    }

    /// Return: go to the highlighted place, and close. `false` when there
    /// is none.
    @discardableResult
    public func openHighlighted() -> Bool {
        guard isOpen, let highlighted else { return false }
        open(entries[highlighted].id)
        return true
    }

    /// A click on the row `token`: go there, and close.
    public func open(_ token: UInt64) {
        guard isOpen else { return }
        close()
        engine.focusOpenPlace(token)
    }

    /// The popover went away. `true` the first time, when the keyboard has
    /// to go back to the list.
    @discardableResult
    public func close() -> Bool {
        guard isOpen else { return false }
        isOpen = false
        filter = ""
        entries = []
        highlighted = nil
        return true
    }

    // MARK: -

    /// Ask for the places again under the filter, the highlight on the row
    /// called `name` if it is still listed, or on the first.
    private func read(keeping name: String?) {
        var section: String?
        entries = engine.focusPlaces(filter).map { place in
            let heading = place.section == section ? nil : place.section
            section = place.section
            return Entry(
                id: place.token, heading: heading, name: place.name, count: place.count,
                mark: Self.mark(place.mark),
                cap: place.command.flatMap { KeyCapSpelling.cap(engine.binding(for: $0)) },
                footer: place.footer)
        }
        if entries.isEmpty {
            highlighted = nil
        } else {
            highlighted = name.flatMap { name in entries.firstIndex { $0.name == name } } ?? 0
        }
    }

    static func mark(_ mark: PlaceMarkFfi) -> Mark {
        switch mark {
        case let .role(role): return .symbol(symbol(for: role))
        case let .dot(color): return .dot(LabelColour(hex: color))
        }
    }

    /// The SF Symbol a mailbox's role is drawn with (screen 10).
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
