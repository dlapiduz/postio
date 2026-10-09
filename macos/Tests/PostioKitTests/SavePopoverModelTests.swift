import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The Save popover's model (specs/010-focus-search T100; design §3.9,
/// screen 12). The controller says what it offers; the Mac keeps what the
/// field and the switches hold while they change, and Save sends all four.
@MainActor
struct SavePopoverModelTests {
    final class Engine: SavePopoverEngine {
        var saved: [(String, Bool, Bool, Bool)] = []
        func focusSearchSave(_ name: String, pin: Bool, notify: Bool, rolling: Bool) {
            saved.append((name, pin, notify, rolling))
        }
    }

    nonisolated static let view = SaveViewFfi(
        title: "Save as a saved search", nameLabel: "Name", name: "Atlas budget from Ada",
        chips: ["from:Ada Moreno", "after:2026-07-01", "atlas budget"],
        pin: true, pinLabel: "Pin to saved searches", pinNote: "Appears at the top of search as",
        pinKey: "alt+3", notify: false, notifyLabel: "Notify when new mail matches",
        notifyNote: "A quiet badge, not a banner", rolling: false, rollingLabel: "Keep the date rolling",
        rollingNote: "Off: always since 1 July. On: always the last 90 days", cancel: "Cancel",
        save: "Save", saveKey: "Return")

    @Test func itOpensOnWhatTheControllerOffersAndSavesWhatWasChanged() {
        let engine = Engine()
        let model = SavePopoverModel(engine: engine)
        #expect(model.apply(.focusSavePopover(view: Self.view)) == .open)
        #expect(model.name == "Atlas budget from Ada")
        #expect(model.pin && !model.notify && !model.rolling)
        model.name = "Atlas from Ada"
        model.notify = true
        model.rolling = true
        model.save()
        #expect(engine.saved.count == 1)
        #expect(engine.saved.first?.0 == "Atlas from Ada")
        #expect(engine.saved.first?.1 == true)
        #expect(engine.saved.first?.2 == true)
        #expect(engine.saved.first?.3 == true)
        #expect(model.apply(.focusSavePopover(view: nil)) == .close)
        #expect(!model.isOpen)
        #expect(model.apply(.focusSavePopover(view: nil)) == nil, "closed once")
    }

    @Test func cancelWritesNothing() {
        let engine = Engine()
        let model = SavePopoverModel(engine: engine)
        var dismissed = false
        model.dismiss = { dismissed = true }
        model.apply(.focusSavePopover(view: Self.view))
        model.cancel()
        #expect(dismissed)
        #expect(!model.isOpen)
        model.save()
        #expect(engine.saved.isEmpty, "a closed popover saves nothing")
    }
}
