import Foundation
import Observation
import PostioFFI

/// Each crosses only when it changes, so this holds what was last said and decides nothing. Beside
/// `FocusIntents` because nothing here moves the list. No AppKit.
@MainActor
@Observable
public final class FocusStates {
    public enum Change: Equatable, Sendable {
        case banner
        case syncLabel
        case empty
        case footer
    }

    public private(set) var banner: BannerFfi?
    public private(set) var syncLabel: SyncLabel?
    public private(set) var empty: EmptyPageFfi?
    public private(set) var footer: FooterViewFfi?

    public init() {}

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
        case let .focusFooter(view):
            footer = view
            return .footer
        default:
            return nil
        }
    }
}
