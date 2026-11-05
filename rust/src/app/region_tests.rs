use super::*;
use std::{
    sync::{mpsc, Arc},
    time::Duration,
};

#[link(name = "user32")]
extern "system" {
    fn UnregisterClassW(class: *const u16, instance: H) -> i32;
}

const STOP: u32 = 0x8000;

unsafe extern "system" fn target_proc(h: H, m: u32, w: W, l: L) -> L {
    if m == 0 {
        let messages = GetWindowLongPtrW(h, -21) as *const AtomicUsize;
        if !messages.is_null() {
            (*messages).fetch_add(1, Ordering::SeqCst);
        }
    }
    DefWindowProcW(h, m, w, l)
}

struct Target {
    handle: usize,
    messages: Arc<AtomicUsize>,
    resume: Option<mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Target {
    fn new(paused: bool) -> Self {
        let messages = Arc::new(AtomicUsize::new(0));
        let target_messages = Arc::clone(&messages);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || unsafe {
            let class = wide(&format!(
                "ShotKitScrollActivationTest{:?}",
                std::thread::current().id()
            ));
            let instance = GetModuleHandleW(std::ptr::null());
            let mut wc: WndClass = zeroed();
            wc.proc = Some(target_proc);
            wc.instance = instance;
            wc.class = class.as_ptr();
            assert_ne!(RegisterClassW(&wc), 0);
            // A message-only window processes real cross-thread messages
            // without displaying UI. Explicit activation may still make it the
            // foreground HWND; tests must observe or inject that query.
            let h = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                -3isize as H, // HWND_MESSAGE
                null_mut(),
                instance,
                null_mut(),
            );
            assert!(!h.is_null());
            SetWindowLongPtrW(h, -21, Arc::as_ptr(&target_messages) as L);
            ready_tx.send(h as usize).unwrap();
            if paused {
                // Keep the target alive but unable to process the barrier.
                // The timeout also allows cleanup if the test thread panics.
                let _ = resume_rx.recv_timeout(Duration::from_secs(10));
            }
            let mut msg: Msg = zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                if msg.msg == STOP {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            DestroyWindow(h);
            assert_ne!(UnregisterClassW(class.as_ptr(), instance), 0);
        });
        Self {
            handle: ready_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            messages,
            resume: Some(resume_tx),
            worker: Some(worker),
        }
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        if let Some(resume) = self.resume.take() {
            let _ = resume.send(());
        }
        unsafe {
            PostMessageW(self.handle as H, STOP, 0, 0);
        }
        if let Some(worker) = self.worker.take() {
            // Preserve the original assertion if unwinding through cleanup.
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}

#[test]
fn activation_barrier_processes_the_target_queue_and_requires_exact_foreground() {
    let target = Target::new(false);
    let root = target.handle as H;
    let mut observations = 0;
    let accepted = unsafe {
        activate_scroll_target_with_foreground(root, || {
            observations += 1;
            assert!(target.messages.load(Ordering::SeqCst) > 0);
            root
        })
    };
    assert!(
        accepted,
        "a processed barrier and exact foreground must succeed"
    );
    assert_eq!(observations, 1);

    target.messages.store(0, Ordering::SeqCst);
    let rejected = unsafe {
        activate_scroll_target_with_foreground(root, || {
            observations += 1;
            assert!(target.messages.load(Ordering::SeqCst) > 0);
            null_mut()
        })
    };
    assert!(
        !rejected,
        "a processed barrier must still reject another foreground"
    );
    assert_eq!(observations, 2);
}

#[test]
fn activation_barrier_observes_the_native_foreground_without_assuming_desktop_policy() {
    let target = Target::new(false);
    let root = target.handle as H;
    let mut observed = None;
    let accepted = unsafe {
        activate_scroll_target_with_foreground(root, || {
            assert!(target.messages.load(Ordering::SeqCst) > 0);
            let actual = GetForegroundWindow();
            observed = Some(actual);
            actual
        })
    };
    // Check the exact native observation used by production, not a later
    // desktop-wide snapshot that could change after this function returns.
    assert_eq!(accepted, observed.expect("foreground was queried") == root);
}

#[test]
fn activation_barrier_rejects_an_unresponsive_target_without_waiting_for_its_release() {
    let target = Target::new(true);
    assert!(!unsafe { activate_scroll_target(target.handle as H) });
    // Returning before Drop releases the target proves the send is bounded;
    // the responsive-target test proves the barrier is actually sent.
    assert_eq!(target.messages.load(Ordering::SeqCst), 0);
}

#[test]
fn activation_barrier_rejects_a_destroyed_target() {
    let target = Target::new(false);
    let handle = target.handle;
    drop(target);
    assert!(!unsafe { activate_scroll_target(handle as H) });
}
