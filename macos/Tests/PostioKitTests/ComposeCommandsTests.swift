import PostioFFI
import Testing

@testable import PostioKit

/// The route from a command id to the composer that has the keyboard.
///
/// Fourteen registry commands — Send, Save draft, Discard draft, Attach file,
/// Detach composer, Copy fields, Insert image, Insert link, Bold, Italic,
/// Bulleted list, Numbered list, Quote block, Schedule send — were drawn in
/// the Message and Format menus, carried a `⌘` chord from the second keyboard
/// layer, were offered in the palette, and reached **nothing**. The composer
/// itself was real the whole time: `ComposeModel` could already apply a mark,
/// attach a file and send. What was missing was anything that turned an id
/// into a call. `postio-gtk` does it with `connect_command`; this is that.
///
/// The decision is here rather than in `Engine` for the usual reason — the
/// executable target has no tests — and because *which* id does *what* is the
/// part worth asserting. Whether a window is in front is AppKit's business.
@MainActor
@Suite struct ComposeCommandsTests {
    private func model(rich: Bool = true) -> ComposeModel {
        ComposeModel(
            id: 1,
            draft: DraftFfi(
                id: 0, account: 1, kind: .new,
                from: "Mara Ostwald <mara@example.com>",
                to: "", cc: "", bcc: "", subject: "", body: "",
                bodyHtml: nil, rich: rich, inReplyTo: nil,
                path: "/tmp/mail", attachments: []
            )
        )
    }

    @Test func aMarkReachesTheDocument() {
        // `⌘B` over a rich draft asks the surface to bold the selection.
        let composer = model()
        for mark in ["bold", "italic", "bullet_list", "numbered_list", "quote_block"] {
            let before = composer.markRequest?.serial ?? 0
            #expect(ComposeCommands.run(mark, on: composer, through: nil))
            #expect(composer.markRequest?.command == mark)
            #expect(
                (composer.markRequest?.serial ?? 0) > before,
                "\(mark) did not ask again"
            )
        }
    }

    @Test func aMarkOnAPlainDraftIsStillClaimedAndDoesNothing() {
        // The bar is disabled on a plain draft, and the keyboard reaches this
        // anyway. It is handled — the composer is what `⌘B` is *for* — and
        // the document has nothing to apply it to.
        let composer = model(rich: false)
        #expect(ComposeCommands.run("bold", on: composer, through: nil))
        #expect(composer.markRequest == nil)
    }

    // -- a picture in the body (#1571) ----------------------------------------

    @Test func insertImageAsksForAPicture() {
        // A command cannot put up an open panel, so it says one is wanted --
        // the same shape as Attach file and Insert link.
        let composer = model()
        #expect(ComposeCommands.run("insert_image", on: composer, through: nil))
        #expect(composer.wantsImage)
    }

    @Test func insertImageOnAPlainDraftSaysWhyNot() {
        // A plain draft has no document to put a picture in. Claimed, because
        // the composer is what the key is for, and said rather than
        // swallowed: a key that silently does nothing cannot be told from one
        // that is broken.
        let composer = model(rich: false)
        #expect(ComposeCommands.run("insert_image", on: composer, through: nil))
        #expect(!composer.wantsImage)
        #expect(composer.status?.contains("Rich") == true, "\(composer.status ?? "nothing said")")
    }

    @Test func aPictureTheBoundaryStoredReachesTheDocumentOnce() {
        // The part is in the store and on the draft by the time the model
        // hears about it; what is left is the surface running the script,
        // once -- a script re-run on every SwiftUI update would insert the
        // picture again on every keystroke.
        let composer = model()
        var withPart = composer.draft
        withPart.id = 9
        withPart.attachments = [
            AttachmentFfi(id: 3, filename: "inline-image.png", mimeType: "image/png", size: "12 B")
        ]
        let before = composer.imageRequest?.serial ?? 0

        composer.took(InlineImageFfi(draft: withPart, script: "insert();"))

        #expect(composer.draft.id == 9, "the id the store assigned is kept")
        #expect(composer.attachments.count == 1)
        #expect(composer.imageRequest?.script == "insert();")
        #expect((composer.imageRequest?.serial ?? 0) == before + 1)
    }

    @Test func theCopyFieldsCommandOpensThem() {
        let composer = model()
        #expect(!composer.showsCopyFields)
        #expect(ComposeCommands.run("copy_fields", on: composer, through: nil))
        #expect(composer.showsCopyFields)
    }

    @Test func aCommandTheComposerDoesNotOwnIsLeftAlone() {
        // `archive` in a compose window is not the composer's to answer, and
        // claiming it would be the bug this whole layer is about in reverse.
        let composer = model()
        #expect(!ComposeCommands.run("archive", on: composer, through: nil))
        #expect(!ComposeCommands.run("next_message", on: composer, through: nil))
    }

    /// Every id this layer claims must be a real command, and must be one the
    /// registry says belongs in the composer.
    @Test func everyClaimedIdIsAComposerCommand() {
        let specs = Dictionary(
            uniqueKeysWithValues: PostioRegistry.commands.map { ($0.id, $0) }
        )
        for id in ComposeCommands.handled {
            let spec = specs[id]
            #expect(spec != nil, "`\(id)` is claimed and is not a command")
            #expect(
                spec?.contexts.contains(.composer) == true,
                "`\(id)` is claimed by the composer and the registry does not put it there"
            )
        }
    }
}
