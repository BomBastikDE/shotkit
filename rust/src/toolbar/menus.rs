//! Menus return ordinary editor command IDs; they never retain a WindowState
//! borrow or a global mutex while Win32 runs a nested popup message loop.
use super::*;
use shotkit::shapes::StampKind;

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
pub unsafe fn popup(parent: Hwnd, id: usize, stamp_kind: StampKind) -> Option<usize> {
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
        let appended = if matches!(entry_id, ID_VIEW | ID_STAMP) {
            let submenu = CreatePopupMenu();
            if submenu.is_null() {
                false
            } else {
                let children: Vec<_> = if entry_id == ID_STAMP {
                    STAMPS.iter().map(|entry| (entry.0, entry.1)).collect()
                } else {
                    VIEW.to_vec()
                };
                let built = children.iter().all(|&(id, label)| {
                    let flags = if stamp_kind_for_command(id) == Some(stamp_kind) {
                        0x0008 // MF_CHECKED
                    } else {
                        0
                    };
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
            let flags = if stamp_kind_for_command(entry_id) == Some(stamp_kind) {
                0x0008 // MF_CHECKED
            } else {
                0
            };
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
