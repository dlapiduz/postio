import Observation
import PostioFFI

/// What the Contacts view tells the engine (specs/011-focus-contacts S7).
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `ContactsModel` without a store.
public protocol ContactsEngine: AnyObject {
    /// The row at `index`, or `nil` while its page is on its way.
    func focusContactsRow(_ index: UInt32) -> ContactRowViewFfi?
    /// How many rows the list has.
    func focusContactsTotal() -> UInt32
    /// Where the cursor is.
    func focusContactsCursor() -> UInt32?
    /// The card, once read.
    func focusContactsCard() -> ContactCardViewFfi?
    /// A click on a kind tab.
    func focusContactsTab(_ tab: ContactTabFfi)
    /// The filter holds `text` now.
    func focusContactsFilter(_ text: String)
    /// A click on a row.
    func focusContactsPoint(_ index: UInt32)
    /// The Edit contact form, while one is open.
    func focusContactEdit() -> ContactEditViewFfi?
    /// A change a person made to the open form.
    func focusContactEditChange(_ change: EditChangeFfi)
    /// A click on the card's switch at `index`.
    func focusContactCardSwitch(_ index: UInt32)
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
}

extension PostioSession: ContactsEngine {}

/// The Contacts view, as the controller's intents leave it (handoff screens
/// 01 to 03): the frame (strip, footer, length), the rows it is asked for one
/// at a time, the cursor and the card.
///
/// The controller (`crates/postio-focus/src/contacts.rs`) holds the tab, the
/// filter, the pages and the card, and says every word; this holds the last
/// frame whole and hands back a click on a tab, a row, a button or the
/// filter. No AppKit (#1264): `ContactsScreen` draws it.
@MainActor
@Observable
public final class ContactsModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// It opened.
        case open
        /// It was drawn again, in place.
        case redraw
        /// The controller put it away.
        case close
    }

    /// The frame; `nil` while the view is closed.
    public private(set) var view: ContactsViewFfi?
    /// The card beside the list; `nil` while it is read.
    public private(set) var card: ContactCardViewFfi?
    /// The Edit contact form over the card; `nil` while there is none.
    public private(set) var edit: ContactEditViewFfi?
    /// The row the keyboard is on.
    public private(set) var cursor: Int?
    /// Bumped when rows landed or the list started over, so the rows drawn
    /// read again.
    public private(set) var revision = 0
    /// What the filter field holds.
    public var filter = ""
    /// Bumped when the controller puts the keyboard in the filter.
    public private(set) var filterRequest = 0

    @ObservationIgnored private let engine: ContactsEngine

    public init(engine: ContactsEngine) {
        self.engine = engine
    }

    /// Whether it is up.
    public var isOpen: Bool { view != nil }

    /// How many rows the list has.
    public var total: Int { Int(view?.total ?? 0) }

    /// Apply `event` if it is the Contacts view's, and say what it changed;
    /// `nil` for every other event.
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
            cursor = nil
            filter = ""
            return .close
        default:
            return nil
        }
    }

    /// The row at `index`: asks for its page when it is not here.
    public func row(at index: Int) -> ContactRowViewFfi? {
        _ = revision
        return engine.focusContactsRow(UInt32(index))
    }

    /// A click on a tab.
    public func choose(_ tab: ContactTabFfi) {
        guard view != nil else { return }
        engine.focusContactsTab(tab)
    }

    /// A click on a row.
    public func point(_ index: Int) {
        guard view != nil, index >= 0, index < total else { return }
        engine.focusContactsPoint(UInt32(index))
    }

    /// What the filter field was typed into.
    public func typed(_ text: String) {
        guard view != nil, text != view?.filter else { return }
        engine.focusContactsFilter(text)
    }

    /// A click on a button on the card: the command a key would send.
    public func run(_ button: CardButtonFfi) {
        guard view != nil, button.enabled else { return }
        engine.invoke(button.command)
    }
}

// MARK: the Edit contact form (specs/011-focus-contacts S8)

extension ContactsModel {
    /// A field of the form changed. The controller owns the form, so what is
    /// drawn is read back from it at once rather than waiting for the event:
    /// a typed letter must not reach the field a beat after its cursor.
    public func change(_ change: EditChangeFfi) {
        guard view != nil, edit != nil else { return }
        engine.focusContactEditChange(change)
        edit = engine.focusContactEdit()
    }

    /// A click on Save: the command Cmd-S sends.
    public func save() {
        guard edit != nil else { return }
        engine.invoke("contact_save")
    }

    /// A click on Cancel: the key Escape sends, so a form with changes arms
    /// before it discards, here as with the keyboard.
    public func cancel() {
        guard edit != nil else { return }
        engine.invoke("back")
        edit = engine.focusContactEdit()
    }

    /// A click on "New contact": the command Cmd-Shift-N sends.
    public func startNew() {
        guard view != nil, edit == nil else { return }
        engine.invoke("contact_new")
    }

    /// A click on the card's switch at `index`, when it is one a click turns.
    public func toggle(switchAt index: Int) {
        guard view != nil, edit == nil, let card, index >= 0, index < card.switches.count,
            card.switches[index].canToggle
        else { return }
        engine.focusContactCardSwitch(UInt32(index))
    }
}
