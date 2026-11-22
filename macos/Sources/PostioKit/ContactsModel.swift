import Observation
import PostioFFI

public protocol ContactsEngine: AnyObject {
    func focusContactsRow(_ index: UInt32) -> ContactRowViewFfi?
    func focusContactsTotal() -> UInt32
    func focusContactsCursor() -> UInt32?
    func focusContactsCard() -> ContactCardViewFfi?
    func focusContactsTab(_ tab: ContactTabFfi)
    func focusContactsFilter(_ text: String)
    func focusContactsPoint(_ index: UInt32)
    func focusContactMergeKeep(_ index: UInt32)
    func focusContactSplit(_ address: String)
    func focusContactEdit() -> ContactEditViewFfi?
    func focusContactEditChange(_ change: EditChangeFfi)
    func focusContactCardSwitch(_ index: UInt32)
    func invoke(_ id: String)
}

extension PostioSession: ContactsEngine {}

/// The controller holds the tab, filter, pages and card, and says every word; this holds the last
/// frame whole and hands back clicks. No AppKit (#1264).
@MainActor
@Observable
public final class ContactsModel {
    public enum Change: Equatable {
        case open
        case redraw
        case close
    }

    public private(set) var view: ContactsViewFfi?
    public private(set) var card: ContactCardViewFfi?
    public private(set) var merge: MergeSheetViewFfi?
    public private(set) var edit: ContactEditViewFfi?
    public private(set) var cursor: Int?
    public private(set) var revision = 0
    public var filter = ""
    public private(set) var filterRequest = 0

    @ObservationIgnored private let engine: ContactsEngine

    public init(engine: ContactsEngine) {
        self.engine = engine
    }

    public var isOpen: Bool { view != nil }

    public var total: Int { Int(view?.total ?? 0) }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusContacts(next):
            let change: Change = view == nil ? .open : .redraw
            if view == nil {
                cursor = engine.focusContactsCursor().map(Int.init)
                card = engine.focusContactsCard()
            }
            view = next
            if filter != next.filter { filter = next.filter }
            return change
        case .focusContactsRows:
            revision += 1
            return view == nil ? nil : .redraw
        case let .focusContactsCursor(index):
            cursor = index.map(Int.init)
            return view == nil ? nil : .redraw
        case let .focusContactsCard(next):
            card = next
            return view == nil ? nil : .redraw
        case let .focusContactMerge(next):
            merge = next
            return view == nil ? nil : .redraw
        case let .focusContactEdit(next):
            edit = next
            return view == nil ? nil : .redraw
        case .focusContactsFocusFilter:
            filterRequest += 1
            return view == nil ? nil : .redraw
        case .focusCloseSurface(.contacts):
            guard view != nil else { return nil }
            view = nil
            card = nil
            edit = nil
            merge = nil
            cursor = nil
            filter = ""
            return .close
        default:
            return nil
        }
    }

    public func row(at index: Int) -> ContactRowViewFfi? {
        _ = revision
        return engine.focusContactsRow(UInt32(index))
    }

    public func choose(_ tab: ContactTabFfi) {
        guard view != nil else { return }
        engine.focusContactsTab(tab)
    }

    public func point(_ index: Int) {
        guard view != nil, index >= 0, index < total else { return }
        engine.focusContactsPoint(UInt32(index))
    }

    public func typed(_ text: String) {
        guard view != nil, text != view?.filter else { return }
        engine.focusContactsFilter(text)
    }

    public func run(_ button: CardButtonFfi) {
        guard view != nil, button.enabled else { return }
        engine.invoke(button.command)
    }
}

extension ContactsModel {
    /// The controller owns the form, so what is drawn is read back at once: a typed letter must not
    /// reach the field a beat after its cursor.
    public func change(_ change: EditChangeFfi) {
        guard view != nil, edit != nil else { return }
        engine.focusContactEditChange(change)
        edit = engine.focusContactEdit()
    }

    public func save() {
        guard edit != nil else { return }
        engine.invoke("contact_save")
    }

    /// The key Escape sends, so a form with changes arms before it discards, as with the keyboard.
    public func cancel() {
        guard edit != nil else { return }
        engine.invoke("back")
        edit = engine.focusContactEdit()
    }

    public func startNew() {
        guard view != nil, edit == nil else { return }
        engine.invoke("contact_new")
    }

    public func toggle(switchAt index: Int) {
        guard view != nil, edit == nil, let card, index >= 0, index < card.switches.count,
            card.switches[index].canToggle
        else { return }
        engine.focusContactCardSwitch(UInt32(index))
    }
}

extension ContactsModel {
    public func keep(cardAt index: Int) {
        guard merge != nil, index >= 0 else { return }
        engine.focusContactMergeKeep(UInt32(index))
    }

    public func choose(_ button: EditButtonFfi, command: String) {
        guard merge != nil else { return }
        engine.invoke(command)
    }

    public func split(_ address: String) {
        guard view != nil, edit == nil, merge == nil else { return }
        engine.focusContactSplit(address)
    }
}
