import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The composer as the controller's surface (specs/009-focus-macos T079;
/// the controller's compose slice, T073).
///
/// The controller opens the composer (`FocusComposer`), closes it
/// (`FocusCloseSurface(.composer)`) and says when to save
/// (`FocusSaveDraft`); the Mac holds the draft's words, saves them when
/// told, and reports every edit, every save, and only the closes the
/// toolkit made.
@MainActor
@Suite struct ComposerWindowTests {
    final class Engine: ComposerEngine {
        var edits = 0
        var saved: [(UInt64, Bool, String?)] = []
        var opened: [SurfaceKindFfi] = []
        var closed: [SurfaceKindFfi] = []

        func focusComposerEdited() { edits += 1 }
        func focusDraftSaved(_ composition: UInt64, kept: Bool, error: String?) {
            saved.append((composition, kept, error))
        }
        func focusSurfaceOpened(_ kind: SurfaceKindFfi) { opened.append(kind) }
        func focusSurfaceClosed(_ kind: SurfaceKindFfi) { closed.append(kind) }
    }

    static func draft(id: Int64 = 0, to: String = "", body: String = "") -> DraftFfi {
        DraftFfi(
            id: id, account: 1, kind: .reply, from: "Mara Ostwald <mara@example.com>",
            to: to, cc: "", bcc: "", subject: "Re: the gate", body: body, bodyHtml: nil,
            rich: false, inReplyTo: 7, path: "/Users/someone/mail", attachments: [])
    }

    @Test func theControllersComposerOpensAndCloses() {
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        #expect(window.apply(.focusComposer(kind: .reply, message: 7)) == .open(.reply, 7))
        #expect(window.isOpen)
        window.show(Self.draft())
        #expect(window.model?.draft.subject == "Re: the gate")

        #expect(window.apply(.focusCloseSurface(kind: .composer)) == .close)
        #expect(!window.isOpen)
        #expect(window.closedByToolkit() == false, "the controller closed it: nothing to report")
        #expect(engine.opened.isEmpty && engine.closed.isEmpty, "the controller keeps its own stack")
    }

    @Test func otherEventsAreNotTheComposers() {
        let window = ComposerWindow(engine: Engine())
        #expect(window.apply(.focusCloseSurface(kind: .message)) == nil)
        #expect(window.apply(.focusCloseSurface(kind: .composer)) == nil, "not open")
        #expect(window.apply(.focusKeyboardHome) == nil)
    }

    @Test func aCloseTheToolkitMadeIsReportedOnce() {
        let window = ComposerWindow(engine: Engine())
        _ = window.apply(.focusComposer(kind: .new, message: nil))
        #expect(window.closedByToolkit())
        #expect(!window.closedByToolkit(), "once")
    }

    @Test func everyEditIsSaidWhileItIsOpen() {
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        window.edited()
        #expect(engine.edits == 0, "no composer, no edit")
        _ = window.apply(.focusComposer(kind: .new, message: nil))
        window.show(Self.draft())
        window.edited()
        window.edited()
        #expect(engine.edits == 2, "the controller waits out the quiet period, not the Mac")
    }

    @Test func aSaveWritesWhatIsTypedAndSaysItWasKept() {
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        _ = window.apply(.focusComposer(kind: .reply, message: 7))
        window.show(Self.draft(to: "ada@example.com"))
        window.model?.body = "Six is fine."
        var written: [DraftFfi] = []
        #expect(window.apply(.focusSaveDraft(composition: 3)) == .save(3))
        window.save(3) { draft in
            written.append(draft)
            var saved = draft
            saved.id = 42
            return saved
        }
        #expect(written.map(\.body) == ["Six is fine."])
        #expect(window.model?.draft.id == 42, "the id the store gave is kept for the next save")
        #expect(window.model?.savedAt != nil, "the title says when")
        #expect(engine.saved.count == 1)
        #expect(engine.saved.first?.0 == 3 && engine.saved.first?.1 == true)
    }

    @Test func aClosedComposersDraftIsStillSaved() {
        // Esc closes the window first; the save the controller asks for
        // after is of the composition it closed.
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        _ = window.apply(.focusComposer(kind: .new, message: nil))
        window.show(Self.draft())
        window.model?.body = "Kept."
        _ = window.apply(.focusCloseSurface(kind: .composer))
        window.save(1) { draft in
            var saved = draft
            saved.id = 5
            return saved
        }
        #expect(engine.saved.first?.1 == true)
    }

    @Test func aSentDraftIsNotSavedAgain() {
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        _ = window.apply(.focusComposer(kind: .new, message: nil))
        window.show(Self.draft())
        window.model?.markSent()
        var writes = 0
        window.save(1) { _ in
            writes += 1
            return nil
        }
        #expect(writes == 0)
        #expect(engine.saved.first?.1 == false, "nothing kept: it is on its way")
    }

    @Test func aSaveThatFailsSaysWhy() {
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        _ = window.apply(.focusComposer(kind: .new, message: nil))
        window.show(Self.draft())
        window.model?.body = "Lost?"
        window.save(1) { _ in nil }
        #expect(engine.saved.first?.1 == false)
        #expect(engine.saved.first?.2 == "This draft could not be saved.")
    }

    @Test func aDraftOfTheMacsOwnIsAComposerTheControllerIsToldOf() {
        // A `mailto:` link: the Mac made the draft and opens the window.
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        window.open(own: Self.draft(to: "bo@example.com"))
        #expect(window.isOpen)
        #expect(engine.opened == [.composer])
        #expect(window.model?.to == "bo@example.com")
    }

    @Test func aComposerThatCouldNotBeMadePutsTheStackRight() {
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        _ = window.apply(.focusComposer(kind: .reply, message: 7))
        window.couldNotOpen()
        #expect(!window.isOpen)
        #expect(engine.closed == [.composer])
    }
}
