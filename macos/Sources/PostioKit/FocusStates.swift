import Foundation
import Observation
import PostioFFI

/// What Focus's controller says about the app's own state
/// (specs/009-focus-macos T099, screens 16 to 19): the banner under the
/// header strip, the toolbar's sync label, and the page an empty list
/// shows in its place.
///
/// The words are `postio_ui::focus_state`'s and the decisions --
/// which banner outranks which, when an inbox is empty rather than not yet
/// counted, which shortcuts lead anywhere -- are the controller's
/// (`crates/postio-focus/src/states.rs`). Each crosses only when it changes,
/// so this holds what was last said, and decides nothing. Beside
/// `FocusIntents` rather than in it: those are the list's cursor and
/// selection, and nothing here moves the list.
///
/// No AppKit: the toolbar, the strip and the list's replacement all read it.
@MainActor
@Observable
public final class FocusStates {
    /// Which of the three an event said.
    public enum Change: Equatable, Sendable {
        case banner
        case syncLabel
        case empty
    }

    /// The banner, until the controller takes it away.
    public private(set) var banner: BannerFfi?
    /// The toolbar's label, or `nil` before the controller has said one.
    public private(set) var syncLabel: SyncLabel?
    /// The page drawn instead of the list, while there is one. The list is
    /// what is drawn until the controller says otherwise.
    public private(set) var empty: EmptyPageFfi?

    public init() {}

    /// Hold what `event` says, if it is one of the three; `nil` for every
    /// other event, which changes nothing here.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusBanner(said):
            banner = said
            return .banner
        case let .focusSyncLabel(text, mark):
            syncLabel = SyncLabel(text: text, mark: mark)
            return .syncLabel
        case let .focusEmpty(page):
            empty = page
            return .empty
        default:
            return nil
        }
    }
}
