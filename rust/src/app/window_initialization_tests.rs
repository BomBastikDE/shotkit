//! Hidden HWND tests: no message loop, user interaction or visible windows.
//! Run on Windows: cargo test --bin shotkit window::initialization_tests
use super::*;

#[link(name = "user32")]
extern "system" {
    fn SendMessageW(h: H, m: u32, w: W, l: L) -> L;
}

static BLOCKED_CLOSES: AtomicUsize = AtomicUsize::new(0);

#[test]
fn handoff_lifetime_token_rejects_destroyed_and_reused_handles() {
    let original = 1usize as H;
    let lifetime = Rc::new(Cell::new(original));
    let guard = EditorInitializationGuard(Some(Rc::new(EditorCreationProtection {
        close_blocked: Rc::new(Cell::new(true)),
        lifetime: lifetime.clone(),
    })));
    assert!(guard.still_owns(original));
    lifetime.set(null_mut());
    assert!(!guard.still_owns(original));
    assert!(!guard.still_owns(null_mut()));
}

fn fixture_guard(token: Rc<Cell<bool>>) -> EditorInitializationGuard {
    EditorInitializationGuard(Some(Rc::new(EditorCreationProtection {
        close_blocked: token,
        lifetime: Rc::new(Cell::new(null_mut())),
    })))
}

// Use the production procedure for state installation/removal and protected
// WM_CLOSE, with DefWindowProcW for everything else to avoid application-wide
// quit/document bookkeeping in this isolated HWND fixture.
unsafe extern "system" fn fixture_proc(h: H, m: u32, w: W, l: L) -> L {
    if m == 0x0081 {
        let result = proc(h, m, w, l);
        if window_state(h).initialization_close_blocked.is_some() {
            // Reenter before CreateWindowExW has returned the HWND.
            SendMessageW(h, 0x0010, 0, 0);
        }
        return result;
    }
    if m == 0x0082 {
        return proc(h, m, w, l);
    }
    if m == 0x0210 && w & 0xffff == 1 {
        // WM_PARENTNOTIFY/WM_CREATE: model the synchronous child creation that
        // happens while building the toolbar, before initialization completes.
        SendMessageW(h, 0x0010, 0, 0);
        return 0;
    }
    if m == 0x0010
        && window_state(h)
            .initialization_close_blocked
            .as_ref()
            .is_some_and(|blocked| blocked.get())
    {
        BLOCKED_CLOSES.fetch_add(1, Ordering::Relaxed);
        return proc(h, m, w, l);
    }
    DefWindowProcW(h, m, w, l)
}

#[test]
fn protection_starts_during_creation_is_local_and_outlives_destroy_safely() {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("ShotKitInitializationProtectionTest");
        let wc = WndClass {
            style: 0,
            proc: Some(fixture_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: null_mut(),
            cursor: null_mut(),
            background: null_mut(),
            menu: std::ptr::null(),
            class: class.as_ptr(),
        };
        assert_ne!(RegisterClassW(&wc), 0);
        let create = |marker: Option<&EditorCreationProtection>| {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                100,
                100,
                null_mut(),
                null_mut(),
                instance,
                marker.map_or(null_mut(), |marker| {
                    std::ptr::from_ref(marker).cast_mut().cast()
                }),
            )
        };
        let token = Rc::new(Cell::new(true));
        let guard = fixture_guard(token.clone());
        let h = {
            let marker = EditorCreationProtection {
                close_blocked: token.clone(),
                lifetime: Rc::new(Cell::new(null_mut())),
            };
            let h = create(Some(&marker));
            assert!(!h.is_null());
            h
        }; // The WM_NCCREATE marker is already gone; only the token remains.
        assert_eq!(BLOCKED_CLOSES.load(Ordering::Relaxed), 1);
        assert_ne!(GetWindowLongPtrW(h, -21), 0);
        let child = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            std::ptr::null(),
            0x40000000, // WS_CHILD; sends synchronous WM_PARENTNOTIFY.
            0,
            0,
            10,
            10,
            h,
            null_mut(),
            instance,
            null_mut(),
        );
        assert!(!child.is_null());
        assert_eq!(BLOCKED_CLOSES.load(Ordering::Relaxed), 2);
        assert_ne!(GetWindowLongPtrW(h, -21), 0);

        // Another ordinary HWND must close while the protected HWND lives.
        let ordinary = create(None);
        assert!(!ordinary.is_null());
        SendMessageW(ordinary, 0x0010, 0, 0);
        assert_eq!(GetWindowLongPtrW(ordinary, -21), 0);
        assert!(token.get());

        // Releasing the handoff enables normal WM_CLOSE handling.
        drop(guard);
        assert!(!token.get());
        SendMessageW(h, 0x0010, 0, 0);
        assert_eq!(GetWindowLongPtrW(h, -21), 0);

        // Initialization failure destroys the HWND first. The guard still
        // releases only its token; it cannot dereference freed userdata or
        // affect a later HWND, even if Windows reuses the numeric handle.
        let failed_token = Rc::new(Cell::new(true));
        let failed_guard = fixture_guard(failed_token.clone());
        let failed = create(Some(&EditorCreationProtection {
            close_blocked: failed_token.clone(),
            lifetime: Rc::new(Cell::new(null_mut())),
        }));
        assert!(!failed.is_null());
        assert_ne!(DestroyWindow(failed), 0);
        let next_token = Rc::new(Cell::new(true));
        let next_guard = fixture_guard(next_token.clone());
        let next = create(Some(&EditorCreationProtection {
            close_blocked: next_token.clone(),
            lifetime: Rc::new(Cell::new(null_mut())),
        }));
        assert!(!next.is_null());
        drop(failed_guard);
        assert!(!failed_token.get());
        assert!(next_token.get());
        assert_ne!(DestroyWindow(next), 0);
        drop(next_guard);
        assert!(!next_token.get());

        // Simulate a toolbar failure followed by a DestroyWindow failure:
        // ownership must move from Creating to Pending, without a guard drop.
        let pending = begin_protected_editor_creation().unwrap();
        let failed = create(Some(&pending));
        assert!(!failed.is_null());
        finish_protected_editor_creation(&pending, false);
        assert!(retry_protected_editor_cleanup_with(|candidate| {
            assert_eq!(candidate, failed);
            // Inject failed DestroyWindow: no WM_NCDESTROY is delivered.
        })
        .is_err());
        assert!(pending.close_blocked.get());
        assert_eq!(pending.lifetime.get(), failed);
        SendMessageW(failed, 0x0010, 0, 0);
        assert_eq!(pending.lifetime.get(), failed);

        // A repeated failure retains ownership. Nested creation and quit see
        // Retrying and return immediately, without borrowing across Win32.
        assert!(retry_protected_editor_cleanup_with(|_| {
            assert!(begin_protected_editor_creation().is_err());
            quit_application(null_mut());
            assert!(!QUITTING.load(Ordering::SeqCst));
        })
        .is_err());
        retry_protected_editor_cleanup_with(|candidate| {
            assert!(begin_protected_editor_creation().is_err());
            assert_ne!(DestroyWindow(candidate), 0);
            assert!(pending.lifetime.get().is_null());
        })
        .unwrap();

        // A stale token cannot destroy a reused HWND: reinsert the now-dead
        // token, create an independent window, and prove no destroy is called.
        let replacement = create(None);
        assert!(!replacement.is_null());
        PROTECTED_EDITOR_CLEANUP.with(|slot| {
            *slot.borrow_mut() = ProtectedEditorCleanup::Pending(pending.clone());
        });
        retry_protected_editor_cleanup_with(|_| panic!("dead lifetime retried")).unwrap();
        assert_ne!(GetWindowLongPtrW(replacement, -21), 0);
        assert_ne!(DestroyWindow(replacement), 0);
        let resumed = begin_protected_editor_creation().unwrap();
        finish_protected_editor_creation(&resumed, false);

        // A post-creation failure consumes the guard into the same reserved
        // slot, without requiring an editor Vec allocation.
        let handoff = begin_protected_editor_creation().unwrap();
        let handoff_h = create(Some(&handoff));
        assert!(!handoff_h.is_null());
        finish_protected_editor_creation(&handoff, true);
        assert!(begin_protected_editor_creation().is_err());
        assert!(abandon_protected_editor_with(
            EditorInitializationGuard(Some(handoff.clone())),
            |_| {}, // injected DestroyWindow failure after handoff error
        )
        .is_err());
        assert_eq!(handoff.lifetime.get(), handoff_h);
        assert!(handoff.close_blocked.get());
        retry_protected_editor_cleanup().unwrap();
        assert!(handoff.lifetime.get().is_null());
        assert!(handoff.close_blocked.get());

        let complete = begin_protected_editor_creation().unwrap();
        let complete_h = create(Some(&complete));
        assert!(!complete_h.is_null());
        finish_protected_editor_creation(&complete, true);
        drop(EditorInitializationGuard(Some(complete.clone())));
        assert!(!complete.close_blocked.get());
        retry_protected_editor_cleanup_with(|_| panic!("completed handoff cleaned up")).unwrap();
        assert_ne!(DestroyWindow(complete_h), 0);
    }
}
