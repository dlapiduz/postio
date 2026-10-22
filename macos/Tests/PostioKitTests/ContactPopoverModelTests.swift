import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The known-sender popover's model (specs/011-focus-contacts S6, handoff
/// screen 04). The controller says what it draws; the Mac sends back a
/// click on a segment and a click on an action, and keeps no opinion of its
/// own about either.
@MainActor
struct ContactPopoverModelTests {
    final class Engine: ContactPopoverEngine {
        var arrivals: [ContactArrivalFfi] = []
        var invoked: [String] = []
        func focusContactArrival(_ arrival: ContactArrivalFfi) { arrivals.append(arrival) }
        func invoke(_ id: String) { invoked.append(id) }
    }

    nonisolated static func view(arrival: ContactArrivalFfi = .inbox) -> ContactPopoverViewFfi {
        ContactPopoverViewFfi(
            address: "ada@example.org",
            avatar: AvatarFfi(text: "AM", rgb: 0xb0624a, round: true),
            name: "Ada Moreno", subline: "ada@example.org",
            role: "Finance lead, Northwind Partners",
            tiles: [
                PopoverTileFfi(value: "431", caption: "messages"),
                PopoverTileFfi(value: "88", caption: "from you"),
                PopoverTileFfi(value: "today", caption: "last"),
            ],
            open: [
                OpenLineFfi(
                    label: "Waiting on you", text: "Approve the Q3 numbers by Friday", yours: true,
                    message: 7)
            ],
            arrivalTitle: "Mail from Ada arrives in",
            arrival: [
                ArrivalOptionFfi(label: "Inbox", arrival: .inbox, on: arrival == .inbox),
                ArrivalOptionFfi(label: "Digest", arrival: .digest, on: arrival == .digest),
                ArrivalOptionFfi(label: "Filtered out", arrival: .filtered, on: arrival == .filtered),
            ],
            actions: [
                PopoverActionFfi(
                    label: "All mail", command: "contact_show_mail", key: "Return", primary: true,
                    enabled: true),
                PopoverActionFfi(
                    label: "Open card", command: "contact_open_card", key: "cmd+i", primary: false,
                    enabled: false),
            ],
            intro: nil,
            hints: [
                FocusHintFfi(key: "Left/Right", label: "choose"),
                FocusHintFfi(key: "0", label: "default rules"),
            ])
    }

    @Test func itOpensRedrawsAndClosesOnTheControllersWord() {
        let model = ContactPopoverModel(engine: Engine())
        #expect(!model.isOpen)
        #expect(model.apply(.focusContactPopover(view: Self.view())) == .open)
        #expect(model.view?.name == "Ada Moreno")
        #expect(
            model.apply(.focusContactPopover(view: Self.view(arrival: .digest))) == .redraw,
            "a change of arrival is drawn in place")
        #expect(model.view?.arrival.first(where: { $0.on })?.label == "Digest")
        #expect(model.apply(.focusCloseSurface(kind: .contactPopover)) == .close)
        #expect(!model.isOpen)
        #expect(model.apply(.focusCloseSurface(kind: .contactPopover)) == nil, "closed once")
    }

    @Test func aSegmentAndAnActionAreSentAndADisabledOneIsNot() {
        let engine = Engine()
        let model = ContactPopoverModel(engine: engine)
        model.choose(.digest)
        model.apply(.focusContactPopover(view: Self.view()))
        model.choose(.digest)
        #expect(engine.arrivals == [.digest], "nothing is sent while it is closed")
        let actions = model.view?.actions ?? []
        model.run(actions[0])
        model.run(actions[1])
        #expect(engine.invoked == ["contact_show_mail"], "Open card waits for its mode")
    }

    @Test func aClickAwayLeavesItClosed() {
        let model = ContactPopoverModel(engine: Engine())
        model.apply(.focusContactPopover(view: Self.view()))
        model.closedByToolkit()
        #expect(!model.isOpen)
    }

    @Test func theAvatarColourIsTheEnginesOwnRGB() {
        // Not compared as a Color: the channels are what the engine said.
        let model = ContactPopoverModel(engine: Engine())
        model.apply(.focusContactPopover(view: Self.view()))
        #expect(model.view?.avatar.rgb == 0xb0624a)
    }
}
