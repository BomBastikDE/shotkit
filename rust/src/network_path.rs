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

#[cfg(test)]
mod tests {
    use super::is_unc_path_utf16;

    fn classify(path: &str) -> bool {
        is_unc_path_utf16(&path.encode_utf16().collect::<Vec<_>>())
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
