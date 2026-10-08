import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The folders and labels popover's state (specs/009-focus-macos T086,
/// screen 10).
///
/// The controller reads the places in one request and says when it has
/// (`FocusOpenPlaces`, then `FocusPlacesChanged`); `focusPlaces(filter)`
/// answers from that read, in the popover's order, with the words each row
/// says. This holds the filter and the highlight, and goes where Return
/// says.
@MainActor
struct PlacesModelTests {
    final class Engine: PlacesEngine {
        var places: [PlaceEntryFfi] = PlacesModelTests.screen10
        var asked: [String] = []
        var opened: [UInt64] = []
        var bindings: [String: String] = ["go_to_inbox": "g i", "go_to_drafts": "g t"]

        func focusPlaces(_ filter: String) -> [PlaceEntryFfi] {
            asked.append(filter)
            guard !filter.isEmpty else { return places }
            return places.filter { $0.name.localizedCaseInsensitiveContains(filter) }
        }
        func focusOpenPlace(_ token: UInt64) { opened.append(token) }
        func focusPlacesPlaceholder() -> String { "Go to folder or label" }
        func binding(for command: String) -> String? { bindings[command] }
    }

    nonisolated static func entry(
        _ token: UInt64, _ section: String, _ name: String, count: String? = nil,
        mark: PlaceMarkFfi, command: String? = nil
    ) -> PlaceEntryFfi {
        PlaceEntryFfi(
            token: token, section: section, name: name, count: count, mark: mark, command: command,
            footer: "↵ open \(name) · Esc close")
    }

    nonisolated static let screen10: [PlaceEntryFfi] = [
        entry(1, "Mailboxes", "Inbox", count: "312", mark: .role(role: .inbox), command: "go_to_inbox"),
        entry(2, "Mailboxes", "Drafts", count: "2", mark: .role(role: .drafts), command: "go_to_drafts"),
        entry(3, "Folders", "Receipts", count: "214", mark: .role(role: .regular)),
        entry(4, "Folders", "Reading list", count: "37", mark: .role(role: .regular)),
        entry(5, "Labels", "Harbor", count: "71", mark: .dot(color: "#2a9d8f")),
        entry(6, "Labels", "Kids", mark: .dot(color: nil)),
    ]

    static func model() -> (PlacesModel, Engine) {
        let engine = Engine()
        return (PlacesModel(engine: engine), engine)
    }

    @Test func openingListsThePlacesBySectionWithTheirCountsAndKeys() {
        let (model, _) = Self.model()
        #expect(model.apply(.focusOpenPlaces) == .open)
        #expect(model.isOpen)
        #expect(model.entries.map(\.name) == ["Inbox", "Drafts", "Receipts", "Reading list", "Harbor", "Kids"])
        #expect(model.entries.map(\.count) == ["312", "2", "214", "37", "71", nil])
        #expect(model.entries.map(\.cap) == ["g i", "g t", nil, nil, nil, nil])
        // A heading goes wherever the section changes.
        #expect(model.entries.map(\.heading) == ["Mailboxes", nil, "Folders", nil, "Labels", nil])
        #expect(model.placeholder == "Go to folder or label")
    }

    @Test func aMailboxShowsItsRolesSymbolAndALabelItsDot() {
        let (model, _) = Self.model()
        model.apply(.focusOpenPlaces)
        #expect(model.entries[0].mark == .symbol("tray"))
        #expect(model.entries[1].mark == .symbol("doc"))
        #expect(model.entries[2].mark == .symbol("folder"))
        #expect(model.entries[4].mark == .dot(LabelColour(hex: "#2a9d8f")))
        #expect(model.entries[5].mark == .dot(nil), "no colour of its own: the frontend's")
    }

    @Test func typingFiltersAndPutsTheHighlightOnTheFirst() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenPlaces)
        model.move(by: 2)
        model.filterChanged("re")
        #expect(engine.asked.last == "re")
        #expect(model.entries.map(\.name) == ["Receipts", "Reading list"])
        #expect(model.highlighted == 0)
        #expect(model.entries.first?.heading == "Folders")
    }

    @Test func theArrowsMoveTheHighlightAndTheFooterFollowsIt() {
        let (model, _) = Self.model()
        model.apply(.focusOpenPlaces)
        #expect(model.footer == "↵ open Inbox · Esc close")
        model.move(by: 2)
        #expect(model.highlighted == 2)
        #expect(model.footer == "↵ open Receipts · Esc close")
        model.move(by: 10)
        #expect(model.highlighted == 5)
        model.move(by: -10)
        #expect(model.highlighted == 0)
    }

    @Test func nothingMatchingSaysOnlyEscape() {
        let (model, _) = Self.model()
        model.apply(.focusOpenPlaces)
        model.filterChanged("zzz")
        #expect(model.entries.isEmpty)
        #expect(model.highlighted == nil)
        #expect(model.footer == "Esc close")
    }

    @Test func returnOpensTheHighlightedPlaceAndClosesThePopover() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenPlaces)
        model.move(by: 2)
        #expect(model.openHighlighted())
        #expect(engine.opened == [3])
        #expect(!model.isOpen)
        #expect(!model.openHighlighted(), "closed, nothing to open")
        #expect(engine.opened == [3])
    }

    @Test func aClickOpensItsOwnRow() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenPlaces)
        model.open(5)
        #expect(engine.opened == [5])
        #expect(!model.isOpen)
    }

    @Test func aNewReadIsAskedForAgainKeepingTheFilterAndTheRow() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenPlaces)
        model.filterChanged("r")
        model.move(by: 1)
        let highlighted = model.entries[model.highlighted ?? 0].name
        // The read lands: new tokens, the same places.
        engine.places = engine.places.map { place in
            var place = place
            place.token += 100
            return place
        }
        #expect(model.apply(.focusPlacesChanged) == .entries)
        #expect(engine.asked.last == "r")
        #expect(model.entries[model.highlighted ?? 0].name == highlighted)
        #expect(model.entries.allSatisfy { $0.id > 100 })
    }

    @Test func aReadWhileClosedIsNotAsked() {
        let (model, engine) = Self.model()
        #expect(model.apply(.focusPlacesChanged) == nil)
        #expect(engine.asked.isEmpty)
    }

    @Test func thePlaceNameFollowsTheList() {
        let (model, _) = Self.model()
        #expect(model.placeName == "Inbox")
        #expect(model.apply(.focusPlace(name: "Receipts")) == .place)
        #expect(model.placeName == "Receipts")
    }

    @Test func closingForgetsTheFilter() {
        let (model, _) = Self.model()
        model.apply(.focusOpenPlaces)
        model.filterChanged("rec")
        #expect(model.close())
        #expect(!model.close(), "once")
        model.apply(.focusOpenPlaces)
        #expect(model.filter.isEmpty)
        #expect(model.entries.count == Self.screen10.count)
    }

    @Test func theFirstReadLandingPutsTheHighlightOnTheFirstPlace() {
        // Before the read lands only the uncounted views are listed; the
        // highlight on the first of them is not a choice to keep (screen 10
        // opens on Inbox).
        let (model, engine) = Self.model()
        let snoozed = Self.entry(9, "Mailboxes", "Snoozed", mark: .role(role: .snoozed))
        var full = engine.places
        full.insert(snoozed, at: 2)
        engine.places = [snoozed]
        model.apply(.focusOpenPlaces)
        #expect(model.entries.first?.name == "Snoozed")
        engine.places = full
        model.apply(.focusPlacesChanged)
        #expect(model.highlighted == 0)
        #expect(model.entries.first?.name == "Inbox")
    }
}
