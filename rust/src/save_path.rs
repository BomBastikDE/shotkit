//! Resolve the final image filename selected in the save dialog.
use std::path::{Path, PathBuf};

/// Preserve supported extensions and append the selected format otherwise.
pub fn with_extension(path: &Path, filter_index: u32) -> PathBuf {
    let supported = path.extension().and_then(|v| v.to_str()).is_some_and(|v| {
        ["png", "jpg", "jpeg", "bmp", "gif"]
            .iter()
            .any(|ext| v.eq_ignore_ascii_case(ext))
    });
    if supported {
        return path.to_owned();
    }
    let ext = match filter_index {
        2 => "jpg",
        3 => "bmp",
        4 => "gif",
        _ => "png",
    };
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(ext);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::with_extension;
    use std::path::Path;

    #[test]
    fn dotted_names_keep_their_stem_and_selected_format() {
        for (filter, expected) in [
            (1, "Bericht v1.2.png"),
            (2, "Bericht v1.2.jpg"),
            (3, "Bericht v1.2.bmp"),
            (4, "Bericht v1.2.gif"),
        ] {
            assert_eq!(
                with_extension(Path::new("Bericht v1.2"), filter),
                Path::new(expected)
            );
        }
    }

    #[test]
    fn supported_extensions_are_preserved() {
        for name in [
            "report.PNG",
            "report.jpg",
            "report.jpeg",
            "report.bmp",
            "report.gif",
        ] {
            assert_eq!(with_extension(Path::new(name), 2), Path::new(name));
        }
    }
}
