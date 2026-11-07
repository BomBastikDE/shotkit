//! Menus return ordinary editor command IDs; they never retain a WindowState
//! borrow or a global mutex while Win32 runs a nested popup message loop.
use super::*;
use shotkit::shapes::{
    ArrowEnds, FillMode, LineStyle, RectangleCorners, ShapeKind, StampKind, Style,
};

static OVERFLOW: OnceLock<Mutex<HashMap<usize, Vec<usize>>>> = OnceLock::new();
#[link(name = "user32")]
extern "system" {
    fn CreatePopupMenu() -> Hwnd;
    fn AppendMenuW(menu: Hwnd, flags: u32, id: usize, text: *const u16) -> i32;
    fn DestroyMenu(menu: Hwnd) -> i32;
    fn GetWindowRect(hwnd: Hwnd, rect: *mut NativeRect) -> i32;
    fn TrackPopupMenu(
        menu: Hwnd,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        owner: Hwnd,
        rect: *const NativeRect,
    ) -> u32;
}

pub(super) fn set_overflow(hwnd: Hwnd, ids: Vec<usize>) {
    if let Ok(mut state) = OVERFLOW.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        state.insert(hwnd as usize, ids);
    }
}
pub(super) fn remove(hwnd: Hwnd) {
    if let Ok(mut state) = OVERFLOW.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        state.remove(&(hwnd as usize));
    }
}
const FILE: &[(usize, &str)] = &[
    (ID_SAVE, "Speichern\tStrg+S"),
    (ID_SAVE_AS, "Speichern unter …\tStrg+Umschalt+S"),
    (ID_OPEN, "Öffnen …\tStrg+O"),
    (ID_PASTE, "Bild aus Zwischenablage einfügen\tStrg+V"),
    (ID_FOLDER, "Speicherordner öffnen"),
];
const VIEW: &[(usize, &str)] = &[
    (ID_ZOOM_IN, "Vergrößern\tStrg++"),
    (ID_ZOOM_OUT, "Verkleinern\tStrg+-"),
    (ID_ZOOM_ORIGINAL, "Originalgröße\tStrg+0"),
    (ID_ZOOM_FIT, "Einpassen\tStrg+9"),
];

const STAMPS: &[(usize, &str, StampKind)] = &[
    (ID_STAMP_CHECK, "Haken", StampKind::Check),
    (ID_STAMP_CROSS, "Kreuz", StampKind::Cross),
    (ID_STAMP_WARNING, "Warnung", StampKind::Warning),
    (ID_STAMP_INFO, "Information", StampKind::Info),
    (ID_STAMP_CURSOR, "Mauszeiger", StampKind::Cursor),
];

const FORM_STYLES: &[(usize, &str)] = &[
    (ID_FILL_NONE, "Füllung: Keine"),
    (ID_FILL_SOLID, "Füllung: Deckend"),
    (ID_FILL_TRANSLUCENT, "Füllung: Halbtransparent"),
    (ID_LINE_SOLID, "Linie: Durchgezogen"),
    (ID_LINE_DASHED, "Linie: Gestrichelt"),
    (ID_LINE_DOTTED, "Linie: Gepunktet"),
    (ID_CORNERS_SQUARE, "Rechteck-Ecken: Eckig"),
    (ID_CORNERS_ROUNDED, "Rechteck-Ecken: Abgerundet"),
    (ID_ARROW_SINGLE, "Pfeilenden: Einfach"),
    (ID_ARROW_DOUBLE, "Pfeilenden: Doppelt"),
    (ID_ARROW_DOT, "Pfeilenden: Punkt"),
    (ID_CALLOUT_FILL, "Sprechblase: Füllfarbe …"),
    (ID_CALLOUT_TEXT_COLOR, "Sprechblase: Textfarbe …"),
    (ID_CALLOUT_POINTER, "Sprechblase: Spitze ein / aus"),
    (ID_SPOTLIGHT_25, "Spotlight: Abdunklung 25 %"),
    (ID_SPOTLIGHT_50, "Spotlight: Abdunklung 50 %"),
    (ID_SPOTLIGHT_75, "Spotlight: Abdunklung 75 %"),
    (ID_SPOTLIGHT_90, "Spotlight: Abdunklung 90 %"),
    (ID_MAGNIFY_150, "Bildlupe: 1,5×"),
    (ID_MAGNIFY_200, "Bildlupe: 2×"),
    (ID_MAGNIFY_300, "Bildlupe: 3×"),
    (ID_MAGNIFY_400, "Bildlupe: 4×"),
    (ID_MAGNIFIER_CIRCLE, "Bildlupe: Rund / oval"),
    (ID_MAGNIFIER_RECT, "Bildlupe: Rechteck"),
];

#[derive(Clone, Copy)]
pub enum StyleTarget {
    Any,
    Rectangle,
    Ellipse,
    Arrow,
    Line,
    Freehand,
    Callout,
    Spotlight {
        opacity: f32,
    },
    Magnifier {
        zoom: f32,
        frame: shotkit::shapes::MagnifierFrame,
    },
    Other,
}

pub fn style_target(kind: Option<&ShapeKind>) -> StyleTarget {
    match kind {
        None => StyleTarget::Any,
        Some(ShapeKind::Rectangle { .. }) => StyleTarget::Rectangle,
        Some(ShapeKind::Ellipse { .. }) => StyleTarget::Ellipse,
        Some(ShapeKind::Arrow { .. }) => StyleTarget::Arrow,
        Some(ShapeKind::Line { .. }) => StyleTarget::Line,
        Some(ShapeKind::Freehand { .. }) => StyleTarget::Freehand,
        Some(ShapeKind::Callout { .. }) => StyleTarget::Callout,
        Some(ShapeKind::Spotlight { data }) => StyleTarget::Spotlight {
            opacity: data.dim_opacity,
        },
        Some(ShapeKind::Magnifier { data }) => StyleTarget::Magnifier {
            zoom: data.zoom,
            frame: data.frame,
        },
        Some(_) => StyleTarget::Other,
    }
}

pub fn form_style_applies(id: usize, target: StyleTarget) -> bool {
    if matches!(target, StyleTarget::Any) {
        return FORM_STYLES.iter().any(|entry| entry.0 == id);
    }
    match id {
        ID_FILL_NONE..=ID_FILL_TRANSLUCENT => {
            matches!(target, StyleTarget::Rectangle | StyleTarget::Ellipse)
        }
        ID_LINE_SOLID..=ID_LINE_DOTTED => matches!(
            target,
            StyleTarget::Rectangle
                | StyleTarget::Ellipse
                | StyleTarget::Arrow
                | StyleTarget::Line
                | StyleTarget::Freehand
        ),
        ID_CORNERS_SQUARE..=ID_CORNERS_ROUNDED => matches!(target, StyleTarget::Rectangle),
        ID_ARROW_SINGLE..=ID_ARROW_DOT => matches!(target, StyleTarget::Arrow),
        ID_CALLOUT_FILL..=ID_CALLOUT_POINTER => matches!(target, StyleTarget::Callout),
        ID_SPOTLIGHT_25..=ID_SPOTLIGHT_90 => matches!(target, StyleTarget::Spotlight { .. }),
        ID_MAGNIFY_150..=ID_MAGNIFIER_RECT => matches!(target, StyleTarget::Magnifier { .. }),
        _ => false,
    }
}

fn form_style_checked(id: usize, style: &Style, target: StyleTarget) -> bool {
    if let StyleTarget::Spotlight { opacity } = target {
        if let Some(expected) = match id {
            ID_SPOTLIGHT_25 => Some(0.25),
            ID_SPOTLIGHT_50 => Some(0.5),
            ID_SPOTLIGHT_75 => Some(0.75),
            ID_SPOTLIGHT_90 => Some(0.9),
            _ => None,
        } {
            return opacity == expected;
        }
    }
    if let StyleTarget::Magnifier { zoom, frame } = target {
        if let Some(expected) = match id {
            ID_MAGNIFY_150 => Some(1.5),
            ID_MAGNIFY_200 => Some(2.0),
            ID_MAGNIFY_300 => Some(3.0),
            ID_MAGNIFY_400 => Some(4.0),
            _ => None,
        } {
            return zoom == expected;
        }
        if id == ID_MAGNIFIER_CIRCLE {
            return frame == shotkit::shapes::MagnifierFrame::Circle;
        }
        if id == ID_MAGNIFIER_RECT {
            return frame == shotkit::shapes::MagnifierFrame::Rectangle;
        }
    }
    match id {
        ID_FILL_NONE => style.fill == FillMode::None,
        ID_FILL_SOLID => style.fill == FillMode::Solid,
        ID_FILL_TRANSLUCENT => style.fill == FillMode::Translucent,
        ID_LINE_SOLID => style.line_style == LineStyle::Solid,
        ID_LINE_DASHED => style.line_style == LineStyle::Dashed,
        ID_LINE_DOTTED => style.line_style == LineStyle::Dotted,
        ID_CORNERS_SQUARE => style.rectangle_corners == RectangleCorners::Square,
        ID_CORNERS_ROUNDED => style.rectangle_corners == RectangleCorners::Rounded,
        ID_ARROW_SINGLE => style.arrow_ends == ArrowEnds::Single,
        ID_ARROW_DOUBLE => style.arrow_ends == ArrowEnds::Double,
        ID_ARROW_DOT => style.arrow_ends == ArrowEnds::Dot,
        _ => false,
    }
}

pub fn stamp_kind_for_command(id: usize) -> Option<StampKind> {
    STAMPS
        .iter()
        .find(|entry| entry.0 == id)
        .map(|entry| entry.2)
}

/// Called inside the editor's ModalGuard: parent and its child toolbar remain
/// alive while TrackPopupMenu reenters. All lookup results are copied first.
// SAFETY CONTRACT: parent belongs to the UI thread and is kept alive by caller;
// HWNDs are checked before use; no global lock spans a native call or menu loop.
pub unsafe fn popup(
    parent: Hwnd,
    id: usize,
    stamp_kind: StampKind,
    style: &Style,
    target: StyleTarget,
) -> Option<usize> {
    let toolbar = ACTIVE_TOOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|state| state.get(&(parent as usize)).map(|x| x.0))? as Hwnd;
    if IsWindow(toolbar) == 0 {
        return None;
    }
    let mut anchor: NativeRect = std::mem::zeroed();
    if GetWindowRect(GetDlgItem(toolbar, id as i32), &mut anchor) == 0 {
        return None;
    }
    let entries: Vec<(usize, &str)> = match id {
        ID_FILE => FILE.to_vec(),
        ID_VIEW => VIEW.to_vec(),
        ID_STAMP => STAMPS.iter().map(|entry| (entry.0, entry.1)).collect(),
        ID_FORM_STYLE => FORM_STYLES.to_vec(),
        ID_MORE => {
            let ids = OVERFLOW
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .ok()
                .and_then(|state| state.get(&(toolbar as usize)).cloned())
                .unwrap_or_default();
            ids.into_iter()
                .filter_map(|id| ACTIONS.iter().find(|a| a.id == id).map(|a| (a.id, a.label)))
                .collect()
        }
        _ => return None,
    };
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return None;
    }
    let mut allowed = Vec::new();
    for (entry_id, label) in entries {
        // Both grouped menus keep the same contents in every responsive stage.
        let appended = if matches!(entry_id, ID_VIEW | ID_STAMP | ID_FORM_STYLE) {
            let submenu = CreatePopupMenu();
            if submenu.is_null() {
                false
            } else {
                let children: Vec<_> = if entry_id == ID_STAMP {
                    STAMPS.iter().map(|entry| (entry.0, entry.1)).collect()
                } else if entry_id == ID_FORM_STYLE {
                    FORM_STYLES.to_vec()
                } else {
                    VIEW.to_vec()
                };
                let built = children.iter().all(|&(id, label)| {
                    let mut flags = if stamp_kind_for_command(id) == Some(stamp_kind)
                        || form_style_checked(id, style, target)
                    {
                        0x0008 // MF_CHECKED
                    } else {
                        0
                    };
                    if entry_id == ID_FORM_STYLE && !form_style_applies(id, target) {
                        flags |= 0x0001;
                    } // MF_GRAYED
                    AppendMenuW(submenu, flags, id, wide(label).as_ptr()) != 0
                });
                if built && AppendMenuW(menu, 0x0010, submenu as usize, wide(label).as_ptr()) != 0 {
                    allowed.extend(children.iter().map(|x| x.0));
                    true
                } else {
                    DestroyMenu(submenu);
                    false
                }
            }
        } else {
            allowed.push(entry_id);
            let mut flags = if stamp_kind_for_command(entry_id) == Some(stamp_kind)
                || form_style_checked(entry_id, style, target)
            {
                0x0008 // MF_CHECKED
            } else {
                0
            };
            if id == ID_FORM_STYLE && !form_style_applies(entry_id, target) {
                flags |= 0x0001;
            } // MF_GRAYED
            AppendMenuW(menu, flags, entry_id, wide(label).as_ptr()) != 0
        };
        if !appended {
            DestroyMenu(menu);
            return None;
        }
    }
    let selected = TrackPopupMenu(
        menu,
        0x0100 | 0x0002,
        anchor.left,
        anchor.bottom,
        0,
        parent,
        null(),
    ) as usize; // TPM_RETURNCMD | TPM_RIGHTBUTTON
    DestroyMenu(menu); // also destroys attached submenus
    allowed.contains(&selected).then_some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_style_menu_has_unique_commands_and_one_checked_value_per_group() {
        let style = Style::default();
        let mut ids: Vec<_> = FORM_STYLES.iter().map(|entry| entry.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), FORM_STYLES.len());
        assert_eq!(
            FORM_STYLES
                .iter()
                .filter(|entry| form_style_checked(entry.0, &style, StyleTarget::Any))
                .count(),
            4
        );
        assert!(FORM_STYLES.iter().all(|entry| !entry.1.is_empty()));
        assert!(form_style_applies(ID_FILL_SOLID, StyleTarget::Rectangle));
        assert!(!form_style_applies(ID_ARROW_DOT, StyleTarget::Rectangle));
        assert!(form_style_applies(ID_ARROW_DOT, StyleTarget::Arrow));
        assert!(!form_style_applies(ID_FILL_SOLID, StyleTarget::Arrow));
        assert!(FORM_STYLES
            .iter()
            .all(|entry| form_style_applies(entry.0, StyleTarget::Any)));
    }

    #[test]
    fn stamp_menu_covers_each_symbol_once_and_rejects_other_commands() {
        let kinds: Vec<_> = STAMPS.iter().map(|entry| entry.2).collect();
        assert_eq!(kinds, StampKind::ALL);
        for &(id, label, kind) in STAMPS {
            assert!(!label.is_empty());
            assert_eq!(stamp_kind_for_command(id), Some(kind));
            assert_eq!(STAMPS.iter().filter(|entry| entry.0 == id).count(), 1);
        }
        for id in [0, ID_STAMP, ID_STEP, ID_PEN, usize::MAX] {
            assert_eq!(stamp_kind_for_command(id), None);
        }
    }
}
