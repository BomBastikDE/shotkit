//! Menus return ordinary editor command IDs; they never retain a WindowState
//! borrow or a global mutex while Win32 runs a nested popup message loop.
use super::*;

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

// SAFETY CONTRACT: menu is an owned popup handle, labels are copied by User32.
// AppendMenuW does not transfer a submenu unless successful; failures clean up.
unsafe fn append(menu: Hwnd, id: usize, label: &str) -> bool {
    AppendMenuW(menu, 0, id, wide(label).as_ptr()) != 0
}

/// Called inside the editor's ModalGuard: parent and its child toolbar remain
/// alive while TrackPopupMenu reenters. All lookup results are copied first.
// SAFETY CONTRACT: parent belongs to the UI thread and is kept alive by caller;
// HWNDs are checked before use; no global lock spans a native call or menu loop.
pub unsafe fn popup(parent: Hwnd, id: usize) -> Option<usize> {
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
        // Ansicht remains the same submenu in every responsive stage.
        let appended = if entry_id == ID_VIEW {
            let submenu = CreatePopupMenu();
            if submenu.is_null() {
                false
            } else {
                let built = VIEW.iter().all(|&(id, label)| append(submenu, id, label));
                if built && AppendMenuW(menu, 0x0010, submenu as usize, wide(label).as_ptr()) != 0 {
                    allowed.extend(VIEW.iter().map(|x| x.0));
                    true
                } else {
                    DestroyMenu(submenu);
                    false
                }
            }
        } else {
            allowed.push(entry_id);
            append(menu, entry_id, label)
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
