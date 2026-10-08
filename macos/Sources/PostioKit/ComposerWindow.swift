import Foundation
import Observation
import PostioFFI

/// What the composer tells the engine (specs/009-focus-macos T079).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `ComposerWindow` without a store.
public protocol ComposerEngine: AnyObject {
    /// Something was written: said on every edit.
    func focusComposerEdited()
    /// How the save `FocusSaveDraft { composition }` asked for went.
    func focusDraftSaved(_ composition: UInt64, kept: Bool, error: String?)
    /// A surface the Mac opened itself: a `mailto:` link's composer.
    func focusSurfaceOpened(_ kind: SurfaceKindFfi)
    /// A surface closed some way the controller did not close it.
    func focusSurfaceClosed(_ kind: SurfaceKindFfi)
}

extension PostioSession: ComposerEngine {}

/// The composer, as the controller's intents leave it (specs/009-focus-macos
/// T079; the controller's compose slice, T073).
///
/// The controller decides which composer opens and what it answers
/// (`FocusComposer`), when what is written is saved (`FocusSaveDraft`:
/// after its 1500 ms quiet period, or at once when the composition ends),
/// and closes it (`FocusCloseSurface(.composer)`). The draft's words are
/// this side's: the fields and the editing surface hold them, in
/// `ComposeModel`, and this saves them when told and says how it went.
///
/// One composer at a time (M4), in the secondary window. The model is kept
/// after the window closes, because the save that closing asks for comes
/// after the close: it is replaced only by the next composition.
///
/// No AppKit: the window (`PostioAppKit`'s `SecondaryWindowController`) and
/// its SwiftUI content read it, and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class ComposerWindow {
    /// What one applied event asks for.
    public enum Change: Equatable {
        /// `FocusComposer`: make a draft of this kind, answering this
        /// message, and show it -- opening the window, or refilling it.
        case open(ComposerKindFfi, Int64?)
        /// `FocusSaveDraft`: save what the composer holds now, and say how
        /// it went for this composition.
        case save(UInt64)
        /// The controller closed it.
        case close
    }

    /// Whether the composer is up, as far as the controller knows.
    public private(set) var isOpen = false
    /// The draft being written, or the one just closed until its save.
    public private(set) var model: ComposeModel?

    @ObservationIgnored private let engine: ComposerEngine
    /// Each model's own id, so a window's tag can name it.
    @ObservationIgnored private var made: Int64 = 0

    public init(engine: ComposerEngine) {
        self.engine = engine
    }

    /// Apply `event` if it is the composer's, and say what it asks for;
    /// `nil` for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusComposer(kind, message):
            isOpen = true
            return .open(kind, message)
        case let .focusSaveDraft(composition):
            return .save(composition)
        case .focusCloseSurface(.composer):
            guard isOpen else { return nil }
            isOpen = false
            return .close
        default:
            return nil
        }
    }

    /// Put `draft` in the composer: what the controller asked for, made.
    public func show(_ draft: DraftFfi) {
        made += 1
        model = ComposeModel(id: made, draft: draft)
    }

    /// A draft the Mac made itself -- a `mailto:` link -- in the composer,
    /// and the controller told it is up (it saves the one it replaces).
    public func open(own draft: DraftFfi) {
        show(draft)
        isOpen = true
        engine.focusSurfaceOpened(.composer)
    }

    /// The draft the controller asked for could not be made (no account to
    /// write from, a message gone): nothing opens, and the controller's
    /// stack is put right.
    public func couldNotOpen() {
        guard isOpen else { return }
        isOpen = false
        engine.focusSurfaceClosed(.composer)
    }

    /// Something was written. The controller waits out the quiet period.
    public func edited() {
        guard isOpen, model != nil else { return }
        engine.focusComposerEdited()
    }

    /// Save what the composer holds, through `write` (`save_draft`), and say
    /// how it went. Kept when the store has the draft now; not when it was
    /// sent or discarded, or never written down; the sentence when the
    /// write failed.
    public func save(_ composition: UInt64, through write: (DraftFfi) -> DraftFfi?) {
        guard let model, !model.sent else {
            engine.focusDraftSaved(composition, kept: false, error: nil)
            return
        }
        if model.isDirty, !model.save(with: write) {
            engine.focusDraftSaved(composition, kept: false, error: ComposeModel.notSaved)
            return
        }
        let kept = model.draft.id != 0
        if kept { model.savedAt = Date() }
        engine.focusDraftSaved(composition, kept: kept, error: nil)
    }

    /// The window was closed by the toolkit -- its close button, ⌘W, Send
    /// or Discard closing it -- not by the controller: `true` once, when it
    /// was up, so the engine is told.
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        isOpen = false
        return true
    }
}
