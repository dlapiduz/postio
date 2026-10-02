import PostioFFI

/// What *Open part* — `Return` in the parts panel — should do with a part.
///
/// Three answers, and the boundary has already decided which: `PartFfi`
/// carries `previewable`, whose whole doc comment is this rule —
/// *"Images and PDFs. Everything else is bytes the application has no
/// business interpreting, and 'open' on one means handing it to the desktop
/// rather than guessing."* Reading it here rather than re-deciding is what
/// keeps `Return` meaning the same thing on both frontends.
///
/// Handing bytes to the desktop is a consent boundary, which is why it is
/// not the default: a part Postio can draw itself is drawn inside Postio,
/// where a document cannot fetch, launch, or phone home.
public enum PartOpening: Equatable, Sendable {
    /// Draw it in a sheet, from bytes already on this machine.
    case preview
    /// Hand it to whichever application claims the type.
    case desktop
    /// Nothing to open: a container, or bytes that have not arrived.
    case nothing

    /// What `Return` means for `part`.
    public static func of(_ part: PartFfi?) -> PartOpening {
        guard let part, !part.isContainer else { return .nothing }
        // Not a fault and not a thing to fix by fetching: an attachment on a
        // message that has only been described has no bytes here, and going
        // to get them on `Return` would be the reader reaching the network.
        guard part.downloaded else { return .nothing }
        return part.previewable ? .preview : .desktop
    }
}
