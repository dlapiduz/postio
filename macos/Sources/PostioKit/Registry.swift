import PostioFFI

/// Postio's command vocabulary, as the macOS application reads it.
///
/// The palette, the cheat sheet and the menu bar are all built from this
/// rather than from a list kept in Swift, so a command added on the Rust side
/// reaches this application without anybody editing it — and
/// `docs/PRODUCT.md`'s rule that *a command that is not in the registry does
/// not exist* stays true on both platforms.
///
/// Needs no session, and that is deliberate rather than incidental: opening a
/// session reads the store's key from the login Keychain, and an unsigned
/// build has a new code identity on every rebuild, so a menu that needed a
/// session would raise a Keychain prompt to draw itself.
public enum PostioRegistry {
    /// Every command, in cheat-sheet order.
    public static var commands: [CommandSpecFfi] { PostioFFI.commands() }
}
