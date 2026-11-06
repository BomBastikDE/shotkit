use super::*;
use std::{
    mem::size_of,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

const WM_DPICHANGED: u32 = 0x02e0;
const WM_GETFONT: u32 = 0x0031;
const WM_GETTEXT: u32 = 0x000d;
const WM_GETTEXTLENGTH: u32 = 0x000e;
const WM_CHAR: u32 = 0x0102;
const WM_QUIT: u32 = 0x0012;
const GW_OWNER: u32 = 4;
const GWL_STYLE: i32 = -16;
const ES_MULTILINE: isize = 0x0004;
const SMTO_BLOCK: u32 = 0x0001;
const SMTO_ABORTIFHUNG: u32 = 0x0002;

#[repr(C)]
#[derive(Clone, Copy)]
struct TestPoint {
    x: i32,
    y: i32,
}

#[repr(C)]
struct LogFontW {
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u8,
    underline: u8,
    strike_out: u8,
    char_set: u8,
    out_precision: u8,
    clip_precision: u8,
    quality: u8,
    pitch_and_family: u8,
    face_name: [u16; 32],
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(
        callback: Option<unsafe extern "system" fn(Hwnd, isize) -> i32>,
        data: isize,
    ) -> i32;
    fn EnumChildWindows(
        parent: Hwnd,
        callback: Option<unsafe extern "system" fn(Hwnd, isize) -> i32>,
        data: isize,
    ) -> i32;
    fn GetWindow(hwnd: Hwnd, command: u32) -> Hwnd;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn GetParent(hwnd: Hwnd) -> Hwnd;
    fn GetClassNameW(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
    fn GetDlgCtrlID(hwnd: Hwnd) -> i32;
    fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    fn GetCurrentThreadId() -> u32;
    fn GetDpiForWindow(hwnd: Hwnd) -> u32;
    fn SetThreadDpiAwarenessContext(context: Hwnd) -> Hwnd;
    fn MapWindowPoints(from: Hwnd, to: Hwnd, points: *mut c_void, count: u32) -> i32;
    fn SendMessageTimeoutW(
        hwnd: Hwnd,
        message: u32,
        wparam: Wparam,
        lparam: Lparam,
        flags: u32,
        timeout_ms: u32,
        result: *mut usize,
    ) -> i32;
    fn PostMessageW(hwnd: Hwnd, message: u32, wparam: Wparam, lparam: Lparam) -> i32;
    fn PostThreadMessageW(thread_id: u32, message: u32, wparam: Wparam, lparam: Lparam) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetObjectW(object: Hwnd, bytes: i32, output: *mut c_void) -> i32;
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Rect {
    fn width(self) -> i32 {
        self.right - self.left
    }

    fn height(self) -> i32 {
        self.bottom - self.top
    }
}

#[derive(Debug)]
struct ControlSnapshot {
    id: i32,
    class: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    font: usize,
    font_height: i32,
}

#[derive(Clone, Copy)]
struct OwnerWindows {
    owner: Hwnd,
    found: Hwnd,
}

unsafe extern "system" fn find_owned_dialog(hwnd: Hwnd, data: isize) -> i32 {
    let windows = &mut *(data as *mut OwnerWindows);
    let mut class = [0u16; 96];
    let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
    if len > 0
        && String::from_utf16_lossy(&class[..len as usize]) == "ShotKitRustTextEntryDialog"
        && GetWindow(hwnd, GW_OWNER) == windows.owner
    {
        windows.found = hwnd;
        return 0;
    }
    1
}

unsafe fn dialog_for_owner(owner: Hwnd) -> Hwnd {
    let mut windows = OwnerWindows {
        owner,
        found: null_mut(),
    };
    EnumWindows(
        Some(find_owned_dialog),
        (&mut windows as *mut OwnerWindows) as isize,
    );
    windows.found
}

unsafe extern "system" fn collect_control(hwnd: Hwnd, data: isize) -> i32 {
    let (parent, controls) = &mut *(data as *mut (Hwnd, Vec<ControlSnapshot>));
    if GetParent(hwnd) != *parent {
        return 1;
    }
    let mut class = [0u16; 64];
    let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
    if len <= 0 {
        return 1;
    }
    let class = String::from_utf16_lossy(&class[..len as usize]);
    let id = GetDlgCtrlID(hwnd);
    let mut rect: NativeRect = zeroed();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return 1;
    }
    let mut points = [
        TestPoint {
            x: rect.left,
            y: rect.top,
        },
        TestPoint {
            x: rect.right,
            y: rect.bottom,
        },
    ];
    MapWindowPoints(null_mut(), *parent, points.as_mut_ptr().cast(), 2);

    let font = send_timeout(hwnd, WM_GETFONT, 0, 0, "WM_GETFONT");
    let mut log_font: LogFontW = zeroed();
    let font_bytes = if font == 0 {
        0
    } else {
        GetObjectW(
            font as Hwnd,
            size_of::<LogFontW>() as i32,
            (&mut log_font as *mut LogFontW).cast(),
        )
    };
    controls.push(ControlSnapshot {
        id,
        class,
        x: points[0].x,
        y: points[0].y,
        width: points[1].x - points[0].x,
        height: points[1].y - points[0].y,
        font,
        font_height: if font_bytes == size_of::<LogFontW>() as i32 {
            log_font.height
        } else {
            0
        },
    });
    1
}

unsafe fn snapshot(hwnd: Hwnd) -> (Rect, Vec<ControlSnapshot>) {
    let mut rect: NativeRect = zeroed();
    assert_ne!(
        GetWindowRect(hwnd, &mut rect),
        0,
        "GetWindowRect(dialog) failed"
    );
    let parent_rect = Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    let mut data = (hwnd, Vec::new());
    EnumChildWindows(
        hwnd,
        Some(collect_control),
        (&mut data as *mut (Hwnd, Vec<ControlSnapshot>)) as isize,
    );
    (parent_rect, data.1)
}

unsafe fn send_timeout(
    hwnd: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
    what: &str,
) -> usize {
    let mut result = 0usize;
    let ok = SendMessageTimeoutW(
        hwnd,
        message,
        wparam,
        lparam,
        SMTO_BLOCK | SMTO_ABORTIFHUNG,
        2_000,
        &mut result,
    );
    assert_ne!(ok, 0, "{what}: SendMessageTimeoutW failed or timed out");
    result
}

unsafe fn read_control_text(hwnd: Hwnd) -> String {
    let length = send_timeout(hwnd, WM_GETTEXTLENGTH, 0, 0, "WM_GETTEXTLENGTH");
    assert!(
        length < 4_096,
        "test Edit text unexpectedly exceeds buffer bound"
    );
    let mut text = Box::new(vec![0u16; length + 1]);
    let mut copied = 0usize;
    let sent = SendMessageTimeoutW(
        hwnd,
        WM_GETTEXT,
        text.len(),
        text.as_mut_ptr() as isize,
        SMTO_BLOCK | SMTO_ABORTIFHUNG,
        2_000,
        &mut copied,
    );
    if sent == 0 {
        let _ = Box::leak(text);
        panic!("WM_GETTEXT failed or timed out; output buffer intentionally retained");
    }
    assert!(
        copied <= length,
        "WM_GETTEXT returned more characters than capacity"
    );
    String::from_utf16_lossy(&text[..copied])
}

fn scaled_from(value: i32, dpi: i32, base_dpi: i32) -> i32 {
    ((i64::from(value) * i64::from(dpi) + i64::from(base_dpi / 2)) / i64::from(base_dpi)) as i32
}

fn assert_scaled_controls(
    base: &[ControlSnapshot],
    current: &[ControlSnapshot],
    dpi: i32,
    base_dpi: i32,
) {
    assert_eq!(
        current.len(),
        3,
        "expected exactly Edit, OK, Cancel direct controls: {current:?}"
    );
    for before in base {
        let after = current
            .iter()
            .find(|control| control.id == before.id)
            .unwrap_or_else(|| panic!("control id {} disappeared: {current:?}", before.id));
        assert_eq!(
            after.class, before.class,
            "control class changed for id {}",
            before.id
        );
        assert!(
            after.width > 0 && after.height > 0,
            "invalid control rect: {after:?}"
        );
        for (field, actual, expected) in [
            ("x", after.x, scaled_from(before.x, dpi, base_dpi)),
            ("y", after.y, scaled_from(before.y, dpi, base_dpi)),
            (
                "width",
                after.width,
                scaled_from(before.width, dpi, base_dpi),
            ),
            (
                "height",
                after.height,
                scaled_from(before.height, dpi, base_dpi),
            ),
        ] {
            assert!((actual - expected).abs() <= 1,
                    "control {} {field} at {dpi} DPI: actual {actual}, expected {expected} (base {before:?}, current {after:?})",
                    before.id);
        }
        assert_ne!(
            after.font, 0,
            "control {} lost WM_GETFONT handle",
            before.id
        );
        assert_ne!(
            after.font_height, 0,
            "GetObjectW failed for control {} font",
            before.id
        );
        let expected_font = scaled_from(before.font_height.abs(), dpi, base_dpi);
        assert!(
            (after.font_height.abs() - expected_font).abs() <= 1,
            "control {} font height at {dpi} DPI: actual {}, expected about {expected_font}",
            before.id,
            after.font_height
        );
    }
}

#[derive(Debug)]
enum WorkerEvent {
    Ready {
        owner: usize,
        thread_id: u32,
        stage: u8,
    },
    Returned {
        value: Option<String>,
        owner_enabled: bool,
        stage: u8,
    },
}

const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

struct DpiContext {
    previous: Hwnd,
}

impl DpiContext {
    fn enter() -> Option<Self> {
        let previous = unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 as Hwnd)
        };
        if previous.is_null() {
            None
        } else {
            Some(Self { previous })
        }
    }
}

impl Drop for DpiContext {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.previous);
        }
    }
}

fn recv(receiver: &Receiver<WorkerEvent>) -> WorkerEvent {
    match receiver.recv_timeout(Duration::from_secs(8)) {
        Ok(event) => event,
        Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for isolated dialog worker"),
        Err(RecvTimeoutError::Disconnected) => panic!("isolated dialog worker disconnected"),
    }
}

include!("text_dialog_test_case.rs");

#[test]
fn multiline_prompt_enters_crlf_returns_exact_text_and_cancels() {
    let (tx, rx): (Sender<WorkerEvent>, Receiver<WorkerEvent>) = mpsc::channel();
    let worker = thread::spawn(move || unsafe {
        let _dpi_context = DpiContext::enter().expect("could not set worker thread to PMv2");
        let class = wide("STATIC");
        let title = wide("ShotKit multiline test owner");
        let owner = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            40,
            40,
            100,
            80,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null_mut(),
        );
        assert!(!owner.is_null(), "could not create multiline test owner");
        let _owner_guard = TestOwner(owner);
        EnableWindow(owner, 1);
        let thread_id = GetCurrentThreadId();
        tx.send(WorkerEvent::Ready {
            owner: owner as usize,
            thread_id,
            stage: 3,
        })
        .unwrap();
        let accepted = prompt_multiline(owner, "first");
        tx.send(WorkerEvent::Returned {
            value: accepted,
            owner_enabled: IsWindowEnabled(owner) != 0,
            stage: 3,
        })
        .unwrap();

        tx.send(WorkerEvent::Ready {
            owner: owner as usize,
            thread_id,
            stage: 4,
        })
        .unwrap();
        let cancelled = prompt_multiline(owner, "cancel me");
        tx.send(WorkerEvent::Returned {
            value: cancelled,
            owner_enabled: IsWindowEnabled(owner) != 0,
            stage: 4,
        })
        .unwrap();
        DestroyWindow(owner);
    });

    let (owner, thread_id) = match recv(&rx) {
        WorkerEvent::Ready {
            owner,
            thread_id,
            stage: 3,
        } => (owner as Hwnd, thread_id),
        other => panic!("unexpected multiline worker event: {other:?}"),
    };
    let mut latest_dialog = null_mut();
    let mut cleanup = DialogThreadCleanup {
        thread_id,
        owner,
        dialog: &mut latest_dialog,
        armed: true,
    };
    let dialog = wait_dialog(owner, &mut latest_dialog);
    unsafe {
        assert_eq!(
            IsWindowEnabled(owner),
            0,
            "owner should be disabled while multiline prompt is modal"
        );
        let edit = control(dialog, ID_EDIT);
        let edit_style = GetWindowLongPtrW(edit, GWL_STYLE);
        assert_ne!(
            edit_style & ES_MULTILINE,
            0,
            "prompt_multiline must create a native multiline EDIT"
        );
        assert_eq!(read_control_text(edit), "first");

        // Exercise the native EDIT's Enter handling, then type a second line.
        let mut ignored = 0usize;
        for ch in "\rsecond".encode_utf16() {
            assert_ne!(
                SendMessageTimeoutW(
                    edit,
                    WM_CHAR,
                    ch as usize,
                    0,
                    SMTO_BLOCK | SMTO_ABORTIFHUNG,
                    2_000,
                    &mut ignored
                ),
                0,
                "WM_CHAR {ch:#x} timed out"
            );
        }
        assert_eq!(
            read_control_text(edit),
            "first\r\nsecond",
            "Enter should insert a CRLF line break"
        );

        // Also check the multiline-specific layout across synthetic 96/144 DPI changes.
        let (base_window, base_controls) = snapshot(dialog);
        assert_eq!(
            base_controls.len(),
            3,
            "unexpected multiline controls: {base_controls:?}"
        );
        assert_eq!(
            base_controls
                .iter()
                .find(|item| item.id == ID_EDIT as i32)
                .unwrap()
                .class,
            "Edit",
            "multiline input must be a native EDIT control"
        );
        let base_dpi = GetDpiForWindow(dialog) as i32;
        assert!(
            (96..=480).contains(&base_dpi),
            "invalid baseline DPI: {base_dpi}"
        );
        for dpi in [144, 96] {
            let mut suggested = Box::new(NativeRect {
                left: base_window.left + dpi / 4,
                top: base_window.top + dpi / 6,
                right: base_window.left + dpi / 4 + scaled_from(base_window.width(), dpi, base_dpi),
                bottom: base_window.top
                    + dpi / 6
                    + scaled_from(base_window.height(), dpi, base_dpi),
            });
            let mut result = 0usize;
            let sent = SendMessageTimeoutW(
                dialog,
                WM_DPICHANGED,
                ((dpi as usize) << 16) | dpi as usize,
                (&mut *suggested as *mut NativeRect) as isize,
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                2_000,
                &mut result,
            );
            if sent == 0 {
                let _ = Box::leak(suggested);
                panic!("multiline WM_DPICHANGED {dpi} timed out; RECT intentionally retained");
            }
            let (actual, controls) = snapshot(dialog);
            assert_eq!(
                actual.width(),
                suggested.right - suggested.left,
                "multiline outer width at {dpi} DPI"
            );
            assert_eq!(
                actual.height(),
                suggested.bottom - suggested.top,
                "multiline outer height at {dpi} DPI"
            );
            assert_scaled_controls(&base_controls, &controls, dpi, base_dpi);
            assert_eq!(
                read_control_text(edit),
                "first\r\nsecond",
                "DPI change damaged multiline text"
            );
        }
        assert_ne!(
            SendMessageTimeoutW(
                dialog,
                WM_COMMAND,
                ID_OK,
                0,
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                2_000,
                &mut 0usize
            ),
            0,
            "multiline OK command timed out"
        );
    }
    match recv(&rx) {
        WorkerEvent::Returned {
            value,
            owner_enabled,
            stage: 3,
        } => {
            assert_eq!(value.as_deref(), Some("first\r\nsecond"));
            assert!(owner_enabled, "owner was not restored after multiline OK");
        }
        other => panic!("unexpected multiline result: {other:?}"),
    }
    match recv(&rx) {
        WorkerEvent::Ready {
            owner: next_owner,
            stage: 4,
            ..
        } => assert_eq!(next_owner as Hwnd, owner),
        other => panic!("unexpected worker event before multiline Cancel: {other:?}"),
    }
    let cancel_dialog = wait_dialog(owner, &mut latest_dialog);
    unsafe {
        assert_ne!(
            PostMessageW(cancel_dialog, WM_CLOSE, 0, 0),
            0,
            "could not post multiline Cancel/close"
        );
    }
    match recv(&rx) {
        WorkerEvent::Returned {
            value,
            owner_enabled,
            stage: 4,
        } => {
            assert_eq!(value, None, "WM_CLOSE should cancel multiline prompt");
            assert!(
                owner_enabled,
                "owner was not restored after multiline Cancel"
            );
        }
        other => panic!("unexpected multiline Cancel result: {other:?}"),
    }
    wait_thread(&worker);
    cleanup.armed = false;
    worker.join().expect("multiline dialog worker panicked");
    assert!(
        unsafe { IsWindow(owner) } == 0,
        "multiline test owner was not destroyed"
    );
}
