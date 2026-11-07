use super::Tool;

// Keep the short editor title and full accessible toolbar label in one source.
// concat! preserves static labels without runtime allocation or suffix parsing.
macro_rules! tool_labels {
    ($($tool:ident => ($name:literal, $suffix:literal)),+ $(,)?) => {
        impl Tool {
            /// User-facing German name, without keyboard shortcut or explanation.
            pub const fn display_name(self) -> &'static str {
                match self {
                    $(Self::$tool => $name),+
                }
            }

            /// Accessible toolbar label, including shortcut and safety guidance.
            pub const fn toolbar_label(self) -> &'static str {
                match self {
                    $(Self::$tool => concat!($name, $suffix)),+
                }
            }
        }
    };
}

tool_labels! {
    Select => ("Auswahl", " (V)"),
    Arrow => ("Pfeil", " (A)"),
    Line => ("Linie", " (L)"),
    Rectangle => ("Rechteck", " (R)"),
    Ellipse => ("Ellipse", " (E)"),
    Text => ("Text", " (T)"),
    Callout => ("Sprechblase", ""),
    Spotlight => ("Spotlight", " – Bereich hervorheben"),
    Magnifier => ("Bildlupe", " – Ausschnitt vergrößern"),
    Highlight => ("Marker", " (M)"),
    Blur => ("Verpixeln", " (B) – keine sichere Schwärzung"),
    GaussianBlur => ("Weichzeichnen", " (W) – keine sichere Schwärzung"),
    Redact => ("Schwärzen", " (S)"),
    Step => ("Nummer", " (N)"),
    Stamp => ("Stempel", " auswählen…"),
    Pen => ("Stift", " (P)"),
    Crop => ("Zuschneiden", " (C)"),
}

#[cfg(test)]
mod tests {
    use super::Tool;

    #[test]
    fn every_tool_has_the_expected_german_name_and_matching_toolbar_label() {
        let tools = [
            (Tool::Select, "Auswahl"),
            (Tool::Arrow, "Pfeil"),
            (Tool::Line, "Linie"),
            (Tool::Rectangle, "Rechteck"),
            (Tool::Ellipse, "Ellipse"),
            (Tool::Text, "Text"),
            (Tool::Callout, "Sprechblase"),
            (Tool::Spotlight, "Spotlight"),
            (Tool::Magnifier, "Bildlupe"),
            (Tool::Highlight, "Marker"),
            (Tool::Blur, "Verpixeln"),
            (Tool::GaussianBlur, "Weichzeichnen"),
            (Tool::Redact, "Schwärzen"),
            (Tool::Step, "Nummer"),
            (Tool::Stamp, "Stempel"),
            (Tool::Pen, "Stift"),
            (Tool::Crop, "Zuschneiden"),
        ];
        for (tool, expected) in tools {
            assert_eq!(tool.display_name(), expected, "{tool:?}");
            assert!(tool.toolbar_label().starts_with(expected), "{tool:?}");
        }
        assert_eq!(
            Tool::Blur.toolbar_label(),
            "Verpixeln (B) – keine sichere Schwärzung"
        );
        assert_eq!(
            Tool::GaussianBlur.toolbar_label(),
            "Weichzeichnen (W) – keine sichere Schwärzung"
        );
    }
}
