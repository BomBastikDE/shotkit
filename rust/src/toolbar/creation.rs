use super::*;

/// Create an editor toolbar as a child of `parent`.
///
/// Commands sent to the parent:
///
/// | ID | Action |
/// | --- | --- |
/// | `ID_SELECT` / `ID_ARROW` / `ID_LINE` | Auswahl / Pfeil / Linie |
/// | `ID_RECTANGLE` / `ID_ELLIPSE` / `ID_TEXT` | Rechteck / Ellipse / Text |
/// | `ID_CALLOUT` | Sprechblase |
/// | `ID_MARKER` / `ID_BLUR` / `ID_GAUSSIAN_BLUR` | Marker / Verpixeln / Weichzeichnen |
/// | `ID_REDACT` / `ID_STEP` / `ID_PEN` | Schwärzen / Nummer / Stift |
/// | `ID_STAMP` / `ID_CROP` | Stempelauswahl / Zuschneiden |
/// | `ID_COLOR` / `ID_FORM_STYLE` / `ID_LINE_WIDTH` / `ID_FONT_SIZE` / `ID_FRAME` | Farbe / Formstil / Linienbreite / Schrift- und Stempelgröße / Rahmen und Schatten |
/// | `ID_SAVE` / `ID_CAPTURE` / `ID_COPY` / `ID_UNDO` / `ID_REDO` | Speichern / Aufnahme / Kopieren / Zurück / Wiederholen |
///
/// `WM_COMMAND.wParam` contains the ID in its low word and `BN_CLICKED` in its
/// high word; `lParam` is the clicked button HWND.
// SAFETY CONTRACT:
// Voraussetzung: parent ist ein lebendes eigenes Fenster auf dem UI-Thread und zustandsseitig callbackbereit.
// Warum erfüllt bzw. offen: Der Aufrufer erstellt das Fenster zuerst; Tooltiptexte bleiben in Toolbar gespeichert, Handlelebensdauer bei Reentranz bleibt offen.
// Nachrichten: CreateWindowExW, SendMessageW und DestroyWindow können synchron reentrieren.
pub unsafe fn create(parent: *mut c_void) -> Result<Toolbar, String> {
    if parent.is_null() || IsWindow(parent) == 0 {
        return Err("Ungültiges Editor-Fenster".into());
    }
    let instance = GetModuleHandleW(null());
    if instance.is_null() || !register_class(instance) {
        return Err("Toolbar-Fensterklasse konnte nicht registriert werden".into());
    }

    let mut client: NativeRect = std::mem::zeroed();
    let width = if GetClientRect(parent, &mut client) != 0 {
        (client.right - client.left).max(1)
    } else {
        640
    };
    let height = shotkit::toolbar_layout::scale(44, GetDpiForWindow(parent));
    let class_name = wide(CLASS_NAME);
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        null(),
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
        0,
        0,
        width,
        height,
        parent,
        null_mut(),
        instance,
        parent,
    );
    if hwnd.is_null() {
        return Err("Toolbar-Fenster konnte nicht erstellt werden".into());
    }

    let button_class = wide("BUTTON");
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let mut buttons = Vec::with_capacity(ACTIONS.len());
    let mut tip_text = Vec::with_capacity(ACTIONS.len());
    InitCommonControls();
    let tip_class = wide("tooltips_class32");
    let tooltip = CreateWindowExW(
        0x00000008,
        tip_class.as_ptr(),
        null(),
        0x80000000 | 0x00000001,
        0,
        0,
        0,
        0,
        hwnd,
        null_mut(),
        instance,
        null_mut(),
    );
    if tooltip.is_null() {
        DestroyWindow(hwnd);
        return Err("Tooltip-Fenster konnte nicht erstellt werden".into());
    }
    for (index, action) in ACTIONS.iter().enumerate() {
        let (x, y, w, h) = (0, 0, 36, 36);
        let label = wide(action.label);
        let mut tooltip_label = wide(match action.id {
            ID_CAPTURE => "Neue Aufnahme (Strg+N)",
            ID_COPY => "Kopieren (Strg+C)",
            ID_SAVE => "Speichern (Strg+S)",
            ID_UNDO => "Zurück (Strg+Z)",
            ID_REDO => "Wiederholen (Strg+Y)",
            ID_FONT_SIZE => "Schriftgröße / Stempelgröße…",
            _ => action.label,
        });
        let button_style = BS_PUSHBUTTON;
        let button = CreateWindowExW(
            0,
            button_class.as_ptr(),
            label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | button_style,
            x,
            y,
            w,
            h,
            hwnd,
            action.id as Hwnd,
            instance,
            null_mut(),
        );
        if button.is_null() {
            DestroyWindow(hwnd);
            return Err(format!(
                "Toolbar-Schaltfläche {} konnte nicht erstellt werden",
                action.label
            ));
        }
        SendMessageW(button, 0x0030, font as usize, 1); // WM_SETFONT
        if index == 0 {
            if let Ok(mut active) = ACTIVE_TOOLS
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                active.insert(parent as usize, (hwnd as usize, ID_SELECT));
            }
            if let Ok(mut colors) = CURRENT_COLORS
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                let color = shotkit::shapes::Style::default().color;
                colors.insert(
                    parent as usize,
                    u32::from(color.r) | (u32::from(color.g) << 8) | (u32::from(color.b) << 16),
                );
            }
        }
        if !tooltip.is_null() {
            // Version 5 common controls require the V2 size (without lpReserved).
            let mut info = ToolInfoW {
                size: std::mem::offset_of!(ToolInfoW, reserved) as u32,
                flags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd,
                id: button as usize,
                rect: std::mem::zeroed(),
                instance,
                text: tooltip_label.as_mut_ptr(),
                lparam: 0,
                reserved: null_mut(),
            };
            if SendMessageW(tooltip, 0x0432, 0, &mut info as *mut _ as isize) == 0 {
                // TTM_ADDTOOLW
                DestroyWindow(tooltip);
                DestroyWindow(hwnd);
                return Err(format!(
                    "Tooltip für {} konnte nicht registriert werden",
                    action.label
                ));
            }
        }
        tip_text.push(tooltip_label);
        buttons.push(button);
    }
    let toolbar = Toolbar {
        hwnd,
        buttons,
        height: Cell::new(height),
        _tip_text: tip_text,
        resizing: Cell::new(false),
        pending_width: Cell::new(None),
    };
    toolbar.resize(width);
    Ok(toolbar)
}
