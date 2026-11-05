#[test]
fn synthetic_dpi_change_scales_dialog_controls_and_preserves_state() {
    let _test_dpi_context = DpiContext::enter().expect("could not set test thread to PMv2");
    let (tx, rx): (Sender<WorkerEvent>, Receiver<WorkerEvent>) = mpsc::channel();
    let worker = thread::spawn(move || unsafe {
        let _dpi_context = DpiContext::enter().expect("could not set worker thread to PMv2");
        let class = wide("STATIC");
        let title = wide("ShotKit DPI test owner");
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
        assert!(
            !owner.is_null(),
            "could not create test-owned STATIC window"
        );
        let _owner_guard = TestOwner(owner);
        EnableWindow(owner, 1);
        let thread_id = GetCurrentThreadId();
        tx.send(WorkerEvent::Ready {
            owner: owner as usize,
            thread_id,
            stage: 1,
        })
        .unwrap();
        let accepted = prompt(owner, "initial 日本語");
        let owner_enabled_after_accept = IsWindowEnabled(owner) != 0;
        tx.send(WorkerEvent::Returned {
            value: accepted,
            owner_enabled: owner_enabled_after_accept,
            stage: 1,
        })
        .unwrap();

        tx.send(WorkerEvent::Ready {
            owner: owner as usize,
            thread_id,
            stage: 2,
        })
        .unwrap();
        let cancelled = prompt(owner, "cancel me");
        let owner_enabled_after_cancel = IsWindowEnabled(owner) != 0;
        tx.send(WorkerEvent::Returned {
            value: cancelled,
            owner_enabled: owner_enabled_after_cancel,
            stage: 2,
        })
        .unwrap();
        DestroyWindow(owner);
    });

    let first = recv(&rx);
    let (owner, thread_id) = match first {
        WorkerEvent::Ready {
            owner,
            thread_id,
            stage: 1,
        } => (owner as Hwnd, thread_id),
        other => panic!("unexpected first worker event: {other:?}"),
    };
    let mut latest_dialog = null_mut();
    let mut cleanup = DialogThreadCleanup {
        thread_id,
        owner,
        dialog: &mut latest_dialog,
        armed: true,
    };

    let first_dialog = wait_dialog(owner, &mut latest_dialog);
    unsafe {
        assert_eq!(
            IsWindowEnabled(owner),
            0,
            "owner should be disabled while prompt is modal"
        );
        assert_eq!(
            read_control_text(control(first_dialog, ID_EDIT)),
            "initial 日本語"
        );
        let (base_window, base_controls) = snapshot(first_dialog);
        assert_eq!(
            base_controls.len(),
            3,
            "unexpected initial controls: {base_controls:?}"
        );
        let base_dpi = GetDpiForWindow(first_dialog) as i32;
        assert!(
            (96..=480).contains(&base_dpi),
            "invalid baseline GetDpiForWindow value: {base_dpi}"
        );

        // Keep this allocation alive through SendMessageTimeout. On timeout, leak it:
        // the target may still enter its procedure and dereference lParam later.
        let mut rect_144 = Box::new(NativeRect {
            left: base_window.left + 23,
            top: base_window.top + 17,
            right: base_window.left + 23 + scaled_from(base_window.width(), 144, base_dpi),
            bottom: base_window.top + 17 + scaled_from(base_window.height(), 144, base_dpi),
        });
        let dpi_144 = ((144usize) << 16) | 144usize;
        let mut message_result = 0usize;
        let sent = SendMessageTimeoutW(
            first_dialog,
            WM_DPICHANGED,
            dpi_144,
            (&mut *rect_144 as *mut NativeRect) as isize,
            SMTO_BLOCK | SMTO_ABORTIFHUNG,
            2_000,
            &mut message_result,
        );
        if sent == 0 {
            let _ = Box::leak(rect_144);
            panic!("WM_DPICHANGED 144: SendMessageTimeoutW failed or timed out; RECT intentionally retained");
        }
        let (at_144, controls_144) = snapshot(first_dialog);
        assert_eq!(
            at_144.left, rect_144.left,
            "144-DPI suggested left was not applied"
        );
        assert_eq!(
            at_144.top, rect_144.top,
            "144-DPI suggested top was not applied"
        );
        assert_eq!(
            at_144.width(),
            rect_144.right - rect_144.left,
            "144-DPI suggested width was not applied"
        );
        assert_eq!(
            at_144.height(),
            rect_144.bottom - rect_144.top,
            "144-DPI suggested height was not applied"
        );
        assert_scaled_controls(&base_controls, &controls_144, 144, base_dpi);
        assert_eq!(
            read_control_text(control(first_dialog, ID_EDIT)),
            "initial 日本語",
            "DPI change damaged the current Edit text"
        );
        drop(rect_144); // synchronous SendMessageTimeout completed, so the receiver is done with lParam.

        let mut rect_96 = Box::new(NativeRect {
            left: base_window.left - 11,
            top: base_window.top + 29,
            right: base_window.left - 11 + scaled_from(base_window.width(), 96, base_dpi),
            bottom: base_window.top + 29 + scaled_from(base_window.height(), 96, base_dpi),
        });
        let sent = SendMessageTimeoutW(
            first_dialog,
            WM_DPICHANGED,
            96usize << 16 | 96usize,
            (&mut *rect_96 as *mut NativeRect) as isize,
            SMTO_BLOCK | SMTO_ABORTIFHUNG,
            2_000,
            &mut message_result,
        );
        if sent == 0 {
            let _ = Box::leak(rect_96);
            panic!("WM_DPICHANGED 96: SendMessageTimeoutW failed or timed out; RECT intentionally retained");
        }
        let (back_at_96, controls_96) = snapshot(first_dialog);
        assert_eq!(
            back_at_96.left, rect_96.left,
            "96-DPI suggested left was not applied on return"
        );
        assert_eq!(
            back_at_96.top, rect_96.top,
            "96-DPI suggested top was not applied on return"
        );
        assert_eq!(
            back_at_96.width(),
            scaled_from(base_window.width(), 96, base_dpi),
            "96-DPI outer width is wrong"
        );
        assert_eq!(
            back_at_96.height(),
            scaled_from(base_window.height(), 96, base_dpi),
            "96-DPI outer height is wrong"
        );
        assert_scaled_controls(&base_controls, &controls_96, 96, base_dpi);
        assert_eq!(
            read_control_text(control(first_dialog, ID_EDIT)),
            "initial 日本語"
        );
        drop(rect_96);

        // Exercise successful completion after both synchronous DPI notifications.
        assert_ne!(
            SendMessageTimeoutW(
                first_dialog,
                WM_COMMAND,
                ID_OK,
                0,
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                2_000,
                &mut message_result
            ),
            0,
            "OK command timed out"
        );
    }

    match recv(&rx) {
        WorkerEvent::Returned {
            value,
            owner_enabled,
            stage: 1,
        } => {
            assert_eq!(value.as_deref(), Some("initial 日本語"));
            assert!(owner_enabled, "owner was not restored after OK");
        }
        other => panic!("unexpected worker result after OK: {other:?}"),
    }
    match recv(&rx) {
        WorkerEvent::Ready {
            owner: next_owner,
            stage: 2,
            ..
        } => assert_eq!(next_owner as Hwnd, owner),
        other => panic!("unexpected worker event before Cancel: {other:?}"),
    }
    let cancel_dialog = wait_dialog(owner, &mut latest_dialog);
    unsafe {
        assert_eq!(
            IsWindowEnabled(owner),
            0,
            "owner should be disabled during the Cancel prompt"
        );
        // PostMessage avoids blocking on teardown; the worker's own modal loop consumes WM_CLOSE.
        assert_ne!(
            PostMessageW(cancel_dialog, WM_CLOSE, 0, 0),
            0,
            "could not post Cancel/close"
        );
    }
    match recv(&rx) {
        WorkerEvent::Returned {
            value,
            owner_enabled,
            stage: 2,
        } => {
            assert_eq!(value, None, "WM_CLOSE should cancel the prompt");
            assert!(owner_enabled, "owner was not restored after Cancel");
        }
        other => panic!("unexpected worker result after Cancel: {other:?}"),
    }
    wait_thread(&worker);
    cleanup.armed = false;
    worker.join().expect("isolated dialog worker panicked");
    assert!(
        unsafe { IsWindow(owner) } == 0,
        "test owner window was not destroyed"
    );
}

struct TestOwner(Hwnd);

impl Drop for TestOwner {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() && IsWindow(self.0) != 0 {
                DestroyWindow(self.0);
            }
        }
    }
}

struct DialogThreadCleanup {
    thread_id: u32,
    owner: Hwnd,
    dialog: *mut Hwnd,
    armed: bool,
}

impl Drop for DialogThreadCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        unsafe {
            let dialog = *self.dialog;
            if !dialog.is_null() {
                let _ = PostMessageW(dialog, WM_CLOSE, 0, 0);
            }
            // WM_QUIT exits prompt's GetMessage loop even if the dialog could not be found.
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
            let _ = self.owner;
        }
    }
}

fn wait_dialog(owner: Hwnd, latest: &mut Hwnd) -> Hwnd {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let hwnd = unsafe { dialog_for_owner(owner) };
        if !hwnd.is_null() {
            *latest = hwnd;
            // The HWND can be enumerated while WM_CREATE is still constructing
            // controls. prompt shows the window only after CreateWindowExW and
            // its initial positioning have completed.
            if unsafe { IsWindowVisible(hwnd) } != 0 {
                return hwnd;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for a shown owned ShotKit text dialog");
}

fn wait_thread(worker: &thread::JoinHandle<()>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && !worker.is_finished() {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        worker.is_finished(),
        "isolated dialog thread did not finish within 3 seconds"
    );
}

unsafe fn control(parent: Hwnd, id: usize) -> Hwnd {
    struct Find {
        parent: Hwnd,
        id: i32,
        found: Hwnd,
    }
    unsafe extern "system" fn callback(hwnd: Hwnd, data: isize) -> i32 {
        let find = &mut *(data as *mut Find);
        if GetParent(hwnd) == find.parent && GetDlgCtrlID(hwnd) == find.id {
            find.found = hwnd;
            return 0;
        }
        1
    }
    let mut find = Find {
        parent,
        id: id as i32,
        found: null_mut(),
    };
    EnumChildWindows(parent, Some(callback), (&mut find as *mut Find) as isize);
    assert!(!find.found.is_null(), "dialog control ID {id} not found");
    find.found
}
