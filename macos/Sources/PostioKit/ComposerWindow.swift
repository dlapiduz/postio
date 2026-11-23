import Foundation
import Observation
import PostioFFI

public protocol ComposerEngine: AnyObject {
    func focusComposerEdited()
    func focusDraftSaved(_ composition: UInt64, kept: Bool, error: String?)
    func focusComposerDraft(edit: DraftEditFfi, draft: DraftFfi)
    func focusNewerMessages(_ ids: [Int64]) -> [NewerMessageFfi]
    func focusSurfaceOpened(_ kind: SurfaceKindFfi)
    func focusSurfaceClosed(_ kind: SurfaceKindFfi)
}

extension PostioSession: ComposerEngine {}

/// The controller decides when what is written is saved (`FocusSaveDraft`, after a 1500 ms quiet
/// period or at once when the composition ends); this side saves it and says how it went. The model
/// is kept after the window closes because the save that closing asks for comes after the close. No
/// AppKit, so iOS stays reachable (#1264).
@MainActor
@Observable
public final class ComposerWindow {
    public enum Change: Equatable {
        case open(ComposerKindFfi, Int64?)
        case save(UInt64)
        case close
        case threadChanged
    }

    public private(set) var isOpen = false
    public private(set) var model: ComposeModel?

    @ObservationIgnored private let engine: ComposerEngine
    @ObservationIgnored private var made: Int64 = 0

    public init(engine: ComposerEngine) {
        self.engine = engine
    }

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
        case let .focusForwardChooser(view):
            guard let model else { return nil }
            model.chooser = view
            return .threadChanged
        case let .focusOlderReply(view):
            guard let model else { return nil }
            model.olderReply = view
            return .threadChanged
        case let .focusNewerMessages(ids):
            guard let model else { return nil }
            model.newer = ids.map { engine.focusNewerMessages($0) }
            return .threadChanged
        case let .focusQuotedExpanded(expanded):
            guard let model else { return nil }
            model.setQuotedExpanded(expanded)
            return .threadChanged
        case let .focusComposerDraftWanted(edit):
            guard let model else { return nil }
            engine.focusComposerDraft(edit: edit, draft: model.edited)
            return .threadChanged
        case let .focusComposerDraft(draft):
            guard let model else { return nil }
            model.adopt(draft)
            return .threadChanged
        default:
            return nil
        }
    }

    public func show(_ draft: DraftFfi) {
        made += 1
        model = ComposeModel(id: made, draft: draft)
    }

    public func open(own draft: DraftFfi) {
        show(draft)
        isOpen = true
        engine.focusSurfaceOpened(.composer)
    }

    public func couldNotOpen() {
        guard isOpen else { return }
        isOpen = false
        engine.focusSurfaceClosed(.composer)
    }

    public func edited() {
        guard isOpen, model != nil else { return }
        engine.focusComposerEdited()
    }

    /// Kept when the store has the draft now; not when it was sent or discarded, or never written
    /// down; the sentence when the write failed.
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

    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        isOpen = false
        return true
    }
}
