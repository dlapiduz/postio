import PostioFFI

/// Built from the registry, so a command added in Rust reaches this app untouched. Needs no
/// session: opening one reads the Keychain, and an unsigned build has a new identity every rebuild,
/// so a menu needing one would raise a prompt.
public enum PostioRegistry {
    public static var commands: [CommandSpecFfi] { PostioFFI.commands() }
}
