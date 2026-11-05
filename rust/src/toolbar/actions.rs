use super::*;

#[derive(Clone, Copy)]
pub(super) struct Action {
    pub(super) id: usize,
    pub(super) label: &'static str,
    pub(super) icon: Icon,
    pub(super) tool: bool,
}
#[derive(Clone, Copy)]
pub(super) enum Icon {
    Select,
    Arrow,
    Line,
    Rectangle,
    Ellipse,
    Text,
    Marker,
    Pixelate,
    Step,
    Pen,
    Crop,
    Color,
    SwatchRed,
    SwatchBlack,
    SwatchYellow,
    SwatchBlue,
    LineWidth,
    FontSize,
    Frame,
    Capture,
    Copy,
    Save,
    Undo,
    Redo,
    More,
    File,
    View,
}
pub(super) const ACTIONS: [Action; 27] = [
    Action {
        id: ID_SELECT,
        label: "Auswahl (V)",
        icon: Icon::Select,
        tool: true,
    },
    Action {
        id: ID_ARROW,
        label: "Pfeil (A)",
        icon: Icon::Arrow,
        tool: true,
    },
    Action {
        id: ID_LINE,
        label: "Linie (L)",
        icon: Icon::Line,
        tool: true,
    },
    Action {
        id: ID_RECTANGLE,
        label: "Rechteck (R)",
        icon: Icon::Rectangle,
        tool: true,
    },
    Action {
        id: ID_ELLIPSE,
        label: "Ellipse (E)",
        icon: Icon::Ellipse,
        tool: true,
    },
    Action {
        id: ID_TEXT,
        label: "Text (T)",
        icon: Icon::Text,
        tool: true,
    },
    Action {
        id: ID_MARKER,
        label: "Marker (M)",
        icon: Icon::Marker,
        tool: true,
    },
    Action {
        id: ID_BLUR,
        label: "Verpixeln (B) – keine sichere Schwärzung",
        icon: Icon::Pixelate,
        tool: true,
    },
    Action {
        id: ID_STEP,
        label: "Nummer (N)",
        icon: Icon::Step,
        tool: true,
    },
    Action {
        id: ID_PEN,
        label: "Stift (P)",
        icon: Icon::Pen,
        tool: true,
    },
    Action {
        id: ID_CROP,
        label: "Zuschneiden (C)",
        icon: Icon::Crop,
        tool: true,
    },
    Action {
        id: ID_COLOR,
        label: "Farbe…",
        icon: Icon::Color,
        tool: false,
    },
    Action {
        id: ID_SWATCH_RED,
        label: "Rot",
        icon: Icon::SwatchRed,
        tool: false,
    },
    Action {
        id: ID_SWATCH_BLACK,
        label: "Schwarz",
        icon: Icon::SwatchBlack,
        tool: false,
    },
    Action {
        id: ID_SWATCH_YELLOW,
        label: "Gelb",
        icon: Icon::SwatchYellow,
        tool: false,
    },
    Action {
        id: ID_SWATCH_BLUE,
        label: "Blau",
        icon: Icon::SwatchBlue,
        tool: false,
    },
    Action {
        id: ID_LINE_WIDTH,
        label: "Linienbreite…",
        icon: Icon::LineWidth,
        tool: false,
    },
    Action {
        id: ID_FONT_SIZE,
        label: "Schriftgröße…",
        icon: Icon::FontSize,
        tool: false,
    },
    Action {
        id: ID_FRAME,
        label: "Rahmen / Schatten…",
        icon: Icon::Frame,
        tool: false,
    },
    Action {
        id: ID_CAPTURE,
        label: "Neue Aufnahme (Strg+N)",
        icon: Icon::Capture,
        tool: false,
    },
    Action {
        id: ID_COPY,
        label: "Kopieren",
        icon: Icon::Copy,
        tool: false,
    },
    Action {
        id: ID_SAVE,
        label: "Speichern",
        icon: Icon::Save,
        tool: false,
    },
    Action {
        id: ID_UNDO,
        label: "Zurück",
        icon: Icon::Undo,
        tool: false,
    },
    Action {
        id: ID_REDO,
        label: "Wiederholen",
        icon: Icon::Redo,
        tool: false,
    },
    Action {
        id: ID_VIEW,
        label: "Ansicht",
        icon: Icon::View,
        tool: false,
    },
    Action {
        id: ID_FILE,
        label: "Datei",
        icon: Icon::File,
        tool: false,
    },
    Action {
        id: ID_MORE,
        label: "»",
        icon: Icon::More,
        tool: false,
    },
];

pub(super) fn layout_items() -> Vec<shotkit::toolbar_layout::Item> {
    use shotkit::toolbar_layout::{Group, Item};
    // Fixed items are explicitly ordered independently of the left toolbar.
    ACTIONS
        .iter()
        .filter(|a| a.id != ID_MORE && !matches!(a.id, ID_CAPTURE | ID_COPY | ID_SAVE | ID_FILE))
        .map(|a| Item {
            id: a.id,
            group: match a.id {
                ID_FRAME | ID_VIEW => Group::View,
                ID_LINE_WIDTH | ID_FONT_SIZE | ID_UNDO | ID_REDO => Group::SizeHistory,
                ID_SWATCH_RED | ID_SWATCH_BLACK | ID_SWATCH_YELLOW | ID_SWATCH_BLUE => {
                    Group::Palette
                }
                _ => Group::Tool,
            },
        })
        .chain([ID_CAPTURE, ID_COPY, ID_SAVE, ID_FILE].map(|id| Item {
            id,
            group: Group::Fixed,
        }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_actions_follow_reference_stages_without_losing_commands() {
        use shotkit::toolbar_layout::layout;
        let items = layout_items();
        for (width, stage) in [(1000, 0), (900, 1), (800, 2), (700, 3), (430, 3), (180, 3)] {
            let plan = layout(width, 96, &items);
            assert_eq!(plan.stage, stage);
            let mut reachable: Vec<_> = plan
                .buttons
                .iter()
                .map(|p| p.0)
                .chain(plan.overflow.iter().copied())
                .collect();
            let mut expected: Vec<_> = ACTIONS
                .iter()
                .filter(|a| a.id != ID_MORE)
                .map(|a| a.id)
                .collect();
            reachable.sort_unstable();
            expected.sort_unstable();
            assert_eq!(reachable, expected);
            let fixed: Vec<_> = plan.buttons.iter().rev().take(4).map(|p| p.0).collect();
            assert_eq!(fixed, vec![ID_FILE, ID_SAVE, ID_COPY, ID_CAPTURE]);
            for (id, hidden_stage) in [
                (ID_FRAME, 1),
                (ID_VIEW, 1),
                (ID_LINE_WIDTH, 2),
                (ID_FONT_SIZE, 2),
                (ID_UNDO, 2),
                (ID_REDO, 2),
                (ID_SWATCH_RED, 3),
                (ID_SWATCH_BLACK, 3),
                (ID_SWATCH_YELLOW, 3),
                (ID_SWATCH_BLUE, 3),
            ] {
                assert_eq!(plan.overflow.contains(&id), stage >= hidden_stage);
            }
        }
    }
}
