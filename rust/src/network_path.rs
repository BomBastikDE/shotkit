//! Windows network path classification shared with the platform-specific UI.

/// Returns whether a UTF-16 path uses a UNC network namespace.
///
/// Both slash characters are treated as separators because Win32 file APIs
/// accept forward slashes in ordinary paths. Extended local device paths such
/// as `\\?\C:\...` are not classified as network paths.
pub fn is_unc_path_utf16(path: &[u16]) -> bool {
    fn is_sep(unit: u16) -> bool {
        unit == b'\\' as u16 || unit == b'/' as u16
    }
    fn ascii_eq(unit: u16, byte: u8) -> bool {
        unit <= 0x7f && (unit as u8).eq_ignore_ascii_case(&byte)
    }
    fn starts(units: &[u16], prefix: &[u8]) -> bool {
        units.len() >= prefix.len()
            && units
                .iter()
                .zip(prefix)
                .all(|(&unit, &byte)| ascii_eq(unit, byte))
    }

    if path.len() >= 4 && is_sep(path[0]) && is_sep(path[1]) {
        // Device prefixes are not network paths unless their namespace is UNC.
        if (ascii_eq(path[2], b'?') || ascii_eq(path[2], b'.')) && is_sep(path[3]) {
            let rest = &path[4..];
            return starts(rest, b"UNC") && rest.get(3).copied().map(is_sep).unwrap_or(false);
        }
        return true;
    }

    // Native DOS-device spelling accepted by some Windows path consumers.
    path.len() >= 7
        && is_sep(path[0])
        && ascii_eq(path[1], b'?')
        && ascii_eq(path[2], b'?')
        && is_sep(path[3])
        && starts(&path[4..], b"UNC")
        && path.get(7).copied().map(is_sep).unwrap_or(false)
}

/// Classification based on an absolute Windows path and a drive-type query.
/// A known drive type does not prove the target is local: reparse points and
/// changing mappings can redirect it. Unknown paths also may access a network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveLocation {
    Network,
    KnownDrive,
    Unknown,
}

/// Classify UNC lexically, or query only an absolute drive root such as `Z:\\`.
/// The callback receives a NUL-terminated UTF-16 drive root.
pub fn classify_drive_utf16(path: &[u16], query: impl FnOnce(&[u16; 4]) -> u32) -> DriveLocation {
    if is_unc_path_utf16(path) {
        return DriveLocation::Network;
    }
    let sep = |u| u == u16::from(b'\\') || u == u16::from(b'/');
    let start = if path.len() >= 4
        && sep(path[0])
        && sep(path[1])
        && path[2] == u16::from(b'?')
        && sep(path[3])
    {
        4
    } else {
        0
    };
    let rest = &path[start..];
    if rest.len() < 3
        || !(rest[0] <= 0x7f && (rest[0] as u8).is_ascii_alphabetic())
        || rest[1] != u16::from(b':')
        || !sep(rest[2])
    {
        return DriveLocation::Unknown;
    }
    let root = [
        u16::from((rest[0] as u8).to_ascii_uppercase()),
        u16::from(b':'),
        u16::from(b'\\'),
        0,
    ];
    match query(&root) {
        4 => DriveLocation::Network,
        2 | 3 | 5 | 6 => DriveLocation::KnownDrive,
        _ => DriveLocation::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::is_unc_path_utf16;

    fn classify(path: &str) -> bool {
        is_unc_path_utf16(&path.encode_utf16().collect::<Vec<_>>())
    }

    #[test]
    fn mapped_drives_and_unknown_paths_are_distinguished_without_opening_files() {
        use super::{classify_drive_utf16, DriveLocation};
        let utf16 = |s: &str| s.encode_utf16().collect::<Vec<_>>();
        for path in [r"z:\image.png", "z:/image.png", r"\\?\z:\image.png"] {
            assert_eq!(
                classify_drive_utf16(&utf16(path), |root| {
                    assert_eq!(*root, [b'Z' as u16, b':' as u16, b'\\' as u16, 0]);
                    4
                }),
                DriveLocation::Network
            );
        }
        assert_eq!(
            classify_drive_utf16(&utf16(r"C:\local.png"), |_| 3),
            DriveLocation::KnownDrive
        );
        for result in [0, 1, 7, u32::MAX] {
            assert_eq!(
                classify_drive_utf16(&utf16(r"X:\missing.png"), |_| result),
                DriveLocation::Unknown
            );
        }
        for path in [
            r"C:relative.png",
            r"\\?\GLOBALROOT\Device\Mup\server\share\a.png",
            "relative.png",
        ] {
            assert_eq!(
                classify_drive_utf16(&utf16(path), |_| panic!(
                    "unknown paths must not be queried"
                )),
                DriveLocation::Unknown
            );
        }
        assert_eq!(
            classify_drive_utf16(&utf16(r"\\server\share\a.png"), |_| panic!(
                "UNC needs no drive query"
            )),
            DriveLocation::Network
        );
    }

    #[test]
    fn recognizes_unc_forms_and_separator_variants() {
        for path in [
            r"\\server\share\image.png",
            r"//server/share/image.png",
            r"\\?\UNC\server\share\image.png",
            r"//?/unc/server/share/image.png",
            r"\\.\unc\server/share/image.png",
            r"\??\UNC\server\share\image.png",
        ] {
            assert!(classify(path), "expected UNC path: {path}");
        }
    }

    #[test]
    fn excludes_local_and_non_unc_device_paths() {
        for path in [
            r"C:\images\image.png",
            "C:/images/image.png",
            r"images\image.png",
            r"\\?\C:\images\image.png",
            r"\\?\Volume{123}\image.png",
        ] {
            assert!(!classify(path), "unexpected UNC path: {path}");
        }
    }
}
