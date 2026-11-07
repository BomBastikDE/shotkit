use super::*;

// Called only on the owning UI thread. Registry membership is checked before
// reading user data, then the non-reused generation rejects a recycled HWND.
unsafe fn live_owner(identity: WindowIdentity) -> Option<H> {
    let h = identity.handle as H;
    if (h == PRIMARY || editor_windows().contains(&h))
        && GetWindowLongPtrW(h, -21) != 0
        && window_state(h).generation == identity.generation
        && identity.generation != 0
    {
        Some(h)
    } else {
        None
    }
}

/// Worker-side ownership transfer. A failed notification leaves the result for
/// the UI timer; no window state, visibility or busy flag is touched here.
pub(super) fn publish_scroll(
    ticket: ScrollTicket,
    result: Result<Outcome, String>,
    notify: impl FnOnce() -> i32,
) {
    let published = SCROLL_COMPLETION
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .publish(ticket, result);
    if published {
        let _ = notify();
    }
}

// SAFETY: called on PRIMARY's owning thread while PRIMARY is alive. All locks
// and WindowState borrows end before KillTimer, ShowWindow, dialogs or output.
// Guards keep both coordinator and original editor alive across nested pumps.
unsafe fn drain_scroll(
    ticket: ScrollTicket,
    consume: impl FnOnce(H, Result<Outcome, String>),
) -> bool {
    // Keep the coordinator's owned result if a recovery is already present;
    // the timer can retry after the user resolves that slot. Never overwrite it.
    if scroll_recovery_pending() {
        return false;
    }
    let completion = SCROLL_COMPLETION
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take(ticket);
    let Some(completion) = completion else {
        return false;
    };
    KillTimer(PRIMARY, SCROLL_POLL_TIMER);
    let _busy = BusyGuard(true);
    let _coordinator = ModalGuard::new(PRIMARY);
    let original = live_owner(completion.owner);
    let h = original.unwrap_or(PRIMARY);
    let _owner = (h != PRIMARY).then(|| ModalGuard::new(h));
    if original.is_some() && completion.was_visible {
        ShowWindow(h, 5);
    }
    // A destroyed original owner cannot receive the result. The persistent
    // primary follows the existing tray-capture output/recovery path instead.
    consume(h, completion.result);
    true
}

fn route_outcome(
    outcome: Outcome,
    ready: impl FnOnce(Image, String),
    recovery: impl FnOnce(Image, Rotation, String),
) {
    match outcome.pending_rotation {
        Some(rotation) => recovery(outcome.image, rotation, outcome.reason),
        None => ready(outcome.image, outcome.reason),
    }
}

pub(super) unsafe fn finish_scroll(ticket: ScrollTicket) {
    drain_scroll(ticket, |h, result| match result {
        Ok(outcome) => route_outcome(
            outcome,
            |image, message| {
                if let Err(error) = handle_capture(h, image, false) {
                    MessageBoxW(h, wide(&error).as_ptr(), wide("ShotKit").as_ptr(), 0x10);
                } else {
                    MessageBoxW(
                        h,
                        wide(&message).as_ptr(),
                        wide("Scroll-Aufnahme beendet").as_ptr(),
                        0,
                    );
                }
            },
            |image, rotation, message| {
                // SCROLL_BUSY blocks a new job until this drain ends, and the
                // slot gate above excludes an unresolved previous recovery.
                *SCROLL_RECOVERY.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(PendingScrollRecovery {
                        image,
                        pending_rotation: Some(rotation),
                    });
                MessageBoxW(
                    h,
                    wide(&format!("{message}\n\nDie Rückdrehung der horizontalen Aufnahme konnte nicht abgeschlossen werden. Das Bild bleibt mit der nötigen Drehrichtung im Speicher. Bitte im Tray-Menü „Ungesicherte Aufnahme speichern …“ wählen; vor dem Speichern wird die Drehung erneut versucht.")).as_ptr(),
                    wide("Scroll-Aufnahme sichern").as_ptr(),
                    0x30,
                );
            },
        ),
        Err(error) => {
            MessageBoxW(
                h,
                wide(&error).as_ptr(),
                wide("Scroll-Aufnahme fehlgeschlagen").as_ptr(),
                0x10,
            );
        }
    });
}

// Shared production routing, independent of the final output dialogs. Both
// wakeup paths preserve ticket validation and target only the coordinator.
pub(super) unsafe fn dispatch(h: H, m: u32, w: W, finish: impl FnOnce(ScrollTicket)) -> Option<L> {
    if h != PRIMARY {
        return None;
    }
    let ticket = match m {
        SCROLL_DONE => ScrollTicket::from_wire(w),
        0x0113 if w == SCROLL_POLL_TIMER => SCROLL_COMPLETION
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .ready_ticket(),
        _ => return None,
    };
    if let Some(ticket) = ticket {
        finish(ticket);
    }
    Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn pending_rotation_never_routes_to_automatic_output() {
        use std::cell::Cell;
        for rotation in [Rotation::Clockwise, Rotation::Counterclockwise] {
            let sent_to_output = Cell::new(false);
            let recovered = Cell::new(false);
            let image = Image::new(1, 2, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
            let expected = image.clone();
            route_outcome(
                Outcome {
                    image,
                    reason: "rotation failed".to_owned(),
                    fallback_count: 0,
                    pending_rotation: Some(rotation),
                },
                |_, _| sent_to_output.set(true),
                |image, direction, reason| {
                    assert_eq!(image, expected);
                    assert_eq!(direction, rotation);
                    assert_eq!(reason, "rotation failed");
                    recovered.set(true);
                },
            );
            assert!(!sent_to_output.get());
            assert!(recovered.get());
        }
    }

    #[link(name = "user32")]
    extern "system" {
        fn PeekMessageW(msg: *mut Msg, h: H, min: u32, max: u32, remove: u32) -> i32;
        fn SendMessageW(h: H, m: u32, w: W, l: L) -> L;
    }

    struct Observed {
        thread: std::thread::ThreadId,
        owner: usize,
        visible: bool,
        busy: bool,
        guarded: bool,
        survived_close: bool,
        result: Result<Vec<u8>, String>,
    }
    thread_local! {
        static OBSERVED: RefCell<Vec<Observed>> = const { RefCell::new(Vec::new()) };
    }

    unsafe fn test_drain(ticket: ScrollTicket) -> bool {
        drain_scroll(ticket, |h, result| {
            let guarded = window_state(h).modal_depth > 0 && window_state(PRIMARY).modal_depth > 0;
            // Real synchronous reentrancy enters the production close gate.
            SendMessageW(h, 0x0010, 0, 0);
            let observed = Observed {
                thread: std::thread::current().id(),
                owner: h as usize,
                visible: IsWindowVisible(h) != 0,
                busy: SCROLL_BUSY.load(Ordering::SeqCst),
                guarded,
                survived_close: GetWindowLongPtrW(h, -21) != 0,
                result: result.map(|outcome| outcome.image.pixels),
            };
            OBSERVED.with(|values| values.borrow_mut().push(observed));
        })
    }

    unsafe extern "system" fn test_proc(h: H, m: u32, w: W, l: L) -> L {
        if let Some(result) = dispatch(h, m, w, |ticket| {
            test_drain(ticket);
        }) {
            return result;
        }
        match m {
            0x0081 => {
                SetWindowLongPtrW(h, -21, Box::into_raw(Box::new(WindowState::new())) as L);
            }
            0x0082 => {
                let state = GetWindowLongPtrW(h, -21) as *mut WindowState;
                SetWindowLongPtrW(h, -21, 0);
                drop(Box::from_raw(state));
            }
            0x0010 => return window::proc(h, m, w, l),
            _ => {}
        }
        DefWindowProcW(h, m, w, l)
    }

    unsafe fn new_test_window(class: &[u16], instance: H) -> H {
        let h = CreateWindowExW(
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
            null_mut(),
        );
        assert!(!h.is_null());
        h
    }

    unsafe fn await_completion() -> Observed {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut msg: Msg = zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, 1) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if let Some(observed) = OBSERVED.with(|values| values.borrow_mut().pop()) {
                return observed;
            }
            assert!(
                Instant::now() < deadline,
                "UI timer did not drain retained result"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    unsafe fn begin(owner: WindowIdentity, visible: bool) -> ScrollTicket {
        SCROLL_BUSY.store(true, Ordering::SeqCst);
        let ticket = SCROLL_COMPLETION
            .lock()
            .unwrap()
            .begin(owner, visible)
            .unwrap();
        assert_ne!(SetTimer(PRIMARY, SCROLL_POLL_TIMER, 10, None), 0);
        ticket
    }

    #[test]
    fn real_ui_timer_recovers_failed_posts_without_touching_recycled_or_dead_windows() {
        // This test creates and pumps all HWNDs on this actual owner thread;
        // the worker only publishes. It runs in the existing Windows --all-targets CI.
        unsafe {
            let ui_thread = std::thread::current().id();
            let class = wide("ShotKitScrollCompletionUnitTest");
            let instance = GetModuleHandleW(std::ptr::null());
            let mut wc: WndClass = zeroed();
            wc.proc = Some(test_proc);
            wc.instance = instance;
            wc.class = class.as_ptr();
            assert_ne!(RegisterClassW(&wc), 0);
            PRIMARY = new_test_window(&class, instance);
            let original = new_test_window(&class, instance);
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).push(original);
            ShowWindow(original, 5);
            let identity = WindowIdentity {
                handle: original as usize,
                generation: window_state(original).generation,
            };
            ShowWindow(original, 0);
            let ticket = begin(identity, true);
            std::thread::spawn(move || {
                publish_scroll(
                    ticket,
                    Ok(Outcome {
                        image: Image::new(1, 1, vec![1, 2, 3, 255]).unwrap(),
                        reason: "test".into(),
                        fallback_count: 0,
                        pending_rotation: None,
                    }),
                    || 0,
                ); // deterministic permanent notification failure
            })
            .join()
            .unwrap();
            // An occupied recovery slot keeps a newer coordinator payload
            // pending, rather than overwriting or dropping either image.
            *SCROLL_RECOVERY.lock().unwrap() = Some(PendingScrollRecovery {
                image: Image::new(1, 1, vec![9, 8, 7, 255]).unwrap(),
                pending_rotation: Some(Rotation::Counterclockwise),
            });
            assert!(!test_drain(ticket));
            assert_eq!(
                SCROLL_COMPLETION.lock().unwrap().ready_ticket(),
                Some(ticket)
            );
            let retained = SCROLL_RECOVERY.lock().unwrap().take().unwrap();
            assert_eq!(retained.image.pixels, vec![9, 8, 7, 255]);
            assert_eq!(retained.pending_rotation, Some(Rotation::Counterclockwise));
            let observed = await_completion();
            assert_eq!(observed.thread, ui_thread);
            assert_eq!(observed.owner, original as usize);
            assert!(
                observed.visible && observed.busy && observed.guarded && observed.survived_close
            );
            assert_eq!(observed.result.unwrap(), vec![1, 2, 3, 255]);
            assert!(!SCROLL_BUSY.load(Ordering::SeqCst));
            assert!(!test_drain(ticket)); // late successful/duplicate wakeup

            // An originally hidden editor remains hidden; errors still reach UI.
            ShowWindow(original, 0);
            let next = begin(identity, false);
            publish_scroll(next, Err("capture failed".into()), || 0);
            assert!(!test_drain(ticket)); // old ticket cannot take the new result
            let observed = await_completion();
            assert_eq!(observed.owner, original as usize);
            assert!(!observed.visible);
            assert_eq!(observed.result.unwrap_err(), "capture failed");

            // Exercise the posted-message branch of the same production router.
            let posted = begin(identity, false);
            assert_ne!(KillTimer(PRIMARY, SCROLL_POLL_TIMER), 0);
            // KillTimer leaves already queued WM_TIMER messages intact. Remove
            // them before publication so fallback polling cannot satisfy this case.
            let mut stale_timer: Msg = zeroed();
            while PeekMessageW(&mut stale_timer, PRIMARY, 0x0113, 0x0113, 1) != 0 {}
            let mut post_result = 0;
            publish_scroll(
                posted,
                Ok(Outcome {
                    image: Image::new(1, 1, vec![4, 5, 6, 255]).unwrap(),
                    reason: "posted completion".into(),
                    fallback_count: 0,
                    pending_rotation: None,
                }),
                || {
                    post_result = PostMessageW(PRIMARY, SCROLL_DONE, posted.to_wire(), 0);
                    post_result
                },
            );
            assert_ne!(post_result, 0, "completion notification must be queued");
            // Completion messages to another HWND cannot consume the payload.
            SendMessageW(original, SCROLL_DONE, posted.to_wire(), 0);
            assert_eq!(
                SCROLL_COMPLETION.lock().unwrap().ready_ticket(),
                Some(posted)
            );
            let observed = await_completion();
            assert_eq!(observed.owner, original as usize);
            assert_eq!(observed.thread, ui_thread);
            assert_eq!(observed.result.unwrap(), vec![4, 5, 6, 255]);

            // Destroyed owner: persistent coordinator receives the retained result.
            DestroyWindow(original);
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).clear();
            let dead = begin(identity, true);
            let dead_pixels = vec![10, 20, 30, 255, 40, 50, 60, 128];
            publish_scroll(
                dead,
                Ok(Outcome {
                    image: Image::new(2, 1, dead_pixels.clone()).unwrap(),
                    reason: "owner destroyed".into(),
                    fallback_count: 0,
                    pending_rotation: None,
                }),
                || 0,
            );
            let observed = await_completion();
            assert_eq!(observed.owner, PRIMARY as usize);
            assert_eq!(observed.thread, ui_thread);
            assert!(!observed.visible);
            assert_eq!(observed.result.unwrap(), dead_pixels);

            // Simulate handle recycling deterministically with a live replacement
            // handle and the old generation; do not depend on OS reuse timing.
            let replacement = new_test_window(&class, instance);
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).push(replacement);
            assert_ne!(window_state(replacement).generation, identity.generation);
            let recycled = begin(
                WindowIdentity {
                    handle: replacement as usize,
                    generation: identity.generation,
                },
                true,
            );
            let recycled_pixels = vec![70, 80, 90, 0, 100, 110, 120, 255];
            publish_scroll(
                recycled,
                Ok(Outcome {
                    image: Image::new(1, 2, recycled_pixels.clone()).unwrap(),
                    reason: "recycled owner".into(),
                    fallback_count: 0,
                    pending_rotation: None,
                }),
                || 0,
            );
            let observed = await_completion();
            assert_eq!(observed.owner, PRIMARY as usize);
            assert_eq!(observed.thread, ui_thread);
            assert_eq!(observed.result.unwrap(), recycled_pixels);
            assert_eq!(IsWindowVisible(replacement), 0);
            assert!(!SCROLL_BUSY.load(Ordering::SeqCst));
            DestroyWindow(replacement);
            (&mut *std::ptr::addr_of_mut!(EDITOR_WINDOWS)).clear();
            DestroyWindow(PRIMARY);
            PRIMARY = null_mut();
        }
    }
}
