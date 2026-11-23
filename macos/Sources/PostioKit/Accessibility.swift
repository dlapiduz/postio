import PostioFFI

/// Commands this frontend presents a surface for rather than sending to `invoke`. Named, not
/// literals at the `switch`: a literal that no longer matches the registry is a key that silently
/// does nothing (`everyInterceptedCommandIsInTheRegistry` catches it). Keep the list short (#657).
public enum Intercepted {
    public static let back = "back"
    public static let settings = "settings"
    /// The conversation pane's own commands act on a fold this frontend holds. Paging is here
    /// because a hardened web view has no scroll call (`ReaderPaging`).
    public static let scrollReaderDown = "scroll_reader_down"
    public static let scrollReaderUp = "scroll_reader_up"
    /// The draft is in a window this frontend owns and the store has not seen most of it
    /// (`ComposeCommands`).
    public static let composeVerbs = ComposeCommands.handled
    /// This frontend's state: nothing is remembered about an original past the view it was asked
    /// for in.
    public static let prevView = "prev_view"
    public static let viewOriginal = "view_original"
    public static let toggleReaderView = "toggle_reader_view"
    public static let zoomIn = "zoom_in"
    public static let zoomOut = "zoom_out"
    public static let zoomReset = "zoom_reset"
    public static let findInMessage = "find_in_message"
    public static let findNext = "find_next"
    public static let findPrevious = "find_previous"
    public static let showImages = "show_images"
    public static let quit = "quit"
    public static let unsubscribe = "unsubscribe"
    public static let clearListMail = "clear_list_mail"
    public static let alwaysShowImages = "always_show_images"
    /// A file is read or written only through a panel the user used.
    public static let contactsImport = "contacts_import"
    public static let contactsExport = "contacts_export"
    public static let contactsSync = "contacts_sync"
    public static let openAttachmentOrLink = "open_attachment_or_link"
    /// Every one acts on the row the keyboard is on, `SettingsAccounts`'s cursor.
    public static let addAccount = "add_account"
    public static let editConfig = "edit_config"
    public static let toggleAccountEnabled = "toggle_account_enabled"
    public static let removeAccount = "remove_account"
    public static let updateCredential = "update_credential"
    public static let rebuildAccountIndex = "rebuild_account_index"
    public static let setDefaultAccount = "set_default_account"
    public static let expandAll = "expand_all"
    public static let toggleFold = "toggle_fold"
    public static let nextInConversation = "next_in_thread"
    public static let prevInConversation = "prev_in_thread"

    public static let all = [
        back, settings,
        expandAll, toggleFold, nextInConversation, prevInConversation,
        scrollReaderDown, scrollReaderUp,
        prevView, viewOriginal, toggleReaderView, zoomIn, zoomOut, zoomReset,
        findInMessage, findNext, findPrevious,
        addAccount, editConfig, toggleAccountEnabled, removeAccount,
        updateCredential, rebuildAccountIndex, setDefaultAccount,
        showImages,
        quit, unsubscribe, clearListMail, alwaysShowImages, openAttachmentOrLink,
        contactsImport, contactsExport, contactsSync,
    ] + composeVerbs
}
