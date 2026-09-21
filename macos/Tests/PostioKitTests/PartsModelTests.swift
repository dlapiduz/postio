import PostioFFI
import Testing

@testable import PostioKit

/// The parts panel's own state: which part the keyboard is on.
///
/// **An attachment that arrived on a Mac could not be saved.** There was no
/// parts surface at all and no other route to a message's MIME tree, so a
/// message with a PDF on it showed its body and nothing else. Eight registry
/// commands drove GTK's panel and every one of them reached nothing here.
///
/// What is local is the cursor. Everything else — the tree, the prefixes,
/// what a row is called, what is safe to write it under — is the boundary's,
/// because two frontends disagreeing about which bytes are "the attachment"
/// is not a cosmetic difference.
@MainActor
@Suite struct PartsModelTests {
    private func part(_ id: String, _ label: String, container: Bool = false) -> PartFfi {
        PartFfi(
            partId: id, depth: 1, prefix: "  ", mimeType: "application/pdf",
            filename: label, label: label, saveName: label, size: 1024,
            detail: "application/pdf · 1.0 kB", spoken: "\(label), a PDF",
            downloaded: true, isContainer: container, inline: false,
            contentId: nil, previewable: false
        )
    }

    private func parts(_ count: Int, cursor: UInt32 = 0) -> MessagePartsFfi {
        MessagePartsFfi(
            root: "multipart/mixed",
            summary: "\(count) parts",
            parts: (0..<count).map { part("\($0)", "file\($0).pdf") },
            cursor: cursor
        )
    }

    @Test func thePanelOpensWhereTheBoundarySaysItShould() {
        // Not at the root: the message itself is row zero and is never the
        // interesting one, which is why the boundary answers with a cursor
        // rather than leaving the frontend to guess.
        let model = PartsModel()
        model.show(parts(4, cursor: 1))
        #expect(model.cursor == 1)
        #expect(model.focused?.label == "file1.pdf")
    }

    @Test func theCursorWalksAndStopsAtBothEnds() {
        let model = PartsModel()
        model.show(parts(3, cursor: 0))
        model.step(forward: true)
        #expect(model.cursor == 1)
        model.step(forward: true)
        model.step(forward: true)
        #expect(model.cursor == 2, "it wrapped past the end")
        model.step(forward: false)
        model.step(forward: false)
        model.step(forward: false)
        #expect(model.cursor == 0, "it wrapped past the start")
    }

    @Test func anEmptyPanelHasNothingToFocus() {
        let model = PartsModel()
        #expect(model.focused == nil)
        model.step(forward: true)
        #expect(model.cursor == 0, "stepping an empty panel is not a crash")
    }

    @Test func aContainerIsNotSomethingToSave() {
        // A `multipart/mixed` holds other parts and has no bytes of its own.
        // Offering Save on it is offering a verb that cannot run.
        let model = PartsModel()
        model.show(
            MessagePartsFfi(
                root: "multipart/mixed", summary: "2 parts",
                parts: [part("1", "multipart/alternative", container: true), part("2", "a.pdf")],
                cursor: 0
            )
        )
        #expect(!model.canSaveFocused)
        model.step(forward: true)
        #expect(model.canSaveFocused)
    }

    @Test func showingAnotherMessageForgetsWhereTheCursorWas() {
        // The cursor is a position in *this* message's tree. Carrying it
        // across would land on whatever part of the next message happened to
        // share the index.
        let model = PartsModel()
        model.show(parts(5, cursor: 3))
        model.show(parts(2, cursor: 1))
        #expect(model.cursor == 1)
    }

    @Test func aPartThatHasNotArrivedIsStillARowAndCannotBeSavedYet() {
        // The ordinary state of an attachment on a message that has only been
        // described, not a fault — so the row is drawn and the verb waits.
        var notHere = part("2", "big.zip")
        notHere = PartFfi(
            partId: notHere.partId, depth: notHere.depth, prefix: notHere.prefix,
            mimeType: notHere.mimeType, filename: notHere.filename, label: notHere.label,
            saveName: notHere.saveName, size: notHere.size, detail: notHere.detail,
            spoken: notHere.spoken, downloaded: false, isContainer: false,
            inline: false, contentId: nil, previewable: false
        )
        let model = PartsModel()
        model.show(
            MessagePartsFfi(root: "multipart/mixed", summary: "1 part", parts: [notHere], cursor: 0)
        )
        #expect(model.focused?.label == "big.zip", "it is still a row")
        #expect(model.canSaveFocused, "asking for it is what fetches it")
    }
}
