//! Bounded retry policy for opening the Windows clipboard.

use std::time::Duration;

const RETRY_DELAYS_MS: [u64; 4] = [10, 20, 40, 80];

/// Retry only the open operation, without dispatching window messages.
/// The caller must not include clipboard writes in `open`.
pub fn open_with_retry(open: impl FnMut() -> bool) -> bool {
    retry(open, std::thread::sleep)
}

fn retry(mut open: impl FnMut() -> bool, mut sleep: impl FnMut(Duration)) -> bool {
    for delay in RETRY_DELAYS_MS {
        if open() {
            return true;
        }
        sleep(Duration::from_millis(delay));
    }
    open()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn immediate_success_does_not_sleep_or_retry() {
        let calls = Cell::new(0);
        assert!(retry(
            || {
                calls.set(calls.get() + 1);
                true
            },
            |_| panic!("unexpected sleep")
        ));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn temporary_contention_retries_only_until_success() {
        let calls = Cell::new(0);
        let mut sleeps = Vec::new();
        assert!(retry(
            || {
                calls.set(calls.get() + 1);
                calls.get() == 3
            },
            |delay| sleeps.push(delay)
        ));
        assert_eq!(calls.get(), 3);
        assert_eq!(
            sleeps,
            [Duration::from_millis(10), Duration::from_millis(20)]
        );
    }

    #[test]
    fn persistent_contention_has_finite_attempts_and_wait() {
        let calls = Cell::new(0);
        let mut sleeps = Vec::new();
        assert!(!retry(
            || {
                calls.set(calls.get() + 1);
                false
            },
            |delay| sleeps.push(delay)
        ));
        assert_eq!(calls.get(), 5);
        assert_eq!(sleeps.iter().sum::<Duration>(), Duration::from_millis(150));
    }
}

#[cfg(all(test, windows))]
mod windows_contention {
    use super::*;
    use std::{
        ffi::c_void,
        io::{BufRead, BufReader, Read, Write},
        process::{Child, Command, Stdio},
        sync::mpsc::{self, Receiver},
        thread::{self, JoinHandle},
        time::Instant,
    };

    const CHILD_TEST: &str = "clipboard_retry::windows_contention::clipboard_lock_child";
    const CHILD_ENV: &str = "SHOTKIT_CLIPBOARD_RETRY_TEST_CHILD";
    const TIMEOUT: Duration = Duration::from_secs(5);
    const LOCKED: &str = "SHOTKIT_CLIPBOARD_TEST_LOCKED";
    const CLOSED: &str = "SHOTKIT_CLIPBOARD_TEST_CLOSED";

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn GetOpenClipboardWindow() -> *mut c_void;
        fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
        fn CreateWindowExW(
            ex_style: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            parameter: *mut c_void,
        ) -> *mut c_void;
        fn DestroyWindow(window: *mut c_void) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    struct TestWindow(*mut c_void);

    impl TestWindow {
        fn new() -> Self {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
            assert!(
                !instance.is_null(),
                "could not identify test executable module"
            );
            // A predefined system class needs no registration. HWND_MESSAGE
            // creates an invisible window owned by this process/test thread.
            let window = Self(unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    -3isize as *mut c_void,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null_mut(),
                )
            });
            assert_window_process(window.0, std::process::id());
            window
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // Runs on the creating thread, after its clipboard guard.
                unsafe { DestroyWindow(self.0) };
            }
        }
    }

    fn assert_window_process(window: *mut c_void, expected_process: u32) {
        assert!(!window.is_null(), "expected a real clipboard opener window");
        let mut process = 0;
        assert_ne!(
            unsafe { GetWindowThreadProcessId(window, &mut process) },
            0,
            "could not identify clipboard opener"
        );
        assert_eq!(process, expected_process, "wrong clipboard opener process");
    }

    fn assert_open_window(window: *mut c_void, expected_process: u32) {
        assert_eq!(
            unsafe { GetOpenClipboardWindow() },
            window,
            "wrong window has the clipboard open"
        );
        assert_window_process(window, expected_process);
    }

    // Open and close on the same test thread, without reading or writing data.
    struct ClipboardLock(bool);

    impl ClipboardLock {
        fn try_open(window: &TestWindow) -> Option<Self> {
            // Distinct real HWNDs exercise exclusion between different windows;
            // NULL in both processes does not provide distinct opener identity.
            (unsafe { OpenClipboard(window.0) } != 0).then(|| Self(true))
        }

        fn close(&mut self) -> bool {
            if self.0 && unsafe { CloseClipboard() } != 0 {
                self.0 = false;
            }
            !self.0
        }
    }

    impl Drop for ClipboardLock {
        fn drop(&mut self) {
            self.close();
        }
    }

    #[derive(Debug, PartialEq)]
    enum ChildMessage {
        Locked(usize),
        Closed,
    }

    struct LockChild {
        child: Child,
        messages: Receiver<ChildMessage>,
        reader: Option<JoinHandle<()>>,
    }

    impl LockChild {
        fn spawn() -> Self {
            let (sender, messages) = mpsc::channel();
            let child = Command::new(std::env::current_exe().expect("test executable"))
                .args(["--ignored", "--exact", CHILD_TEST, "--test-threads=1"])
                // Scoped to this child; parallel tests never share environment
                // mutations, files, ports or clipboard-holder processes.
                .env(CHILD_ENV, "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn clipboard holder");
            // Establish cleanup before any subsequent operation can panic.
            let mut holder = Self {
                child,
                messages,
                reader: None,
            };
            let stdout = holder.child.stdout.take().expect("child stdout pipe");
            holder.reader = Some(thread::spawn(move || {
                // libtest also writes stdout. Only our isolated protocol lines
                // count as acknowledgements; harness progress is ignored.
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let message = if line == CLOSED {
                        ChildMessage::Closed
                    } else if let Some(window) = line.strip_prefix(&format!("{LOCKED}:")) {
                        let Ok(window) = window.parse::<usize>() else {
                            break;
                        };
                        ChildMessage::Locked(window)
                    } else {
                        continue;
                    };
                    if sender.send(message).is_err() {
                        break;
                    }
                }
            }));
            holder
        }

        fn next_message(&self) -> ChildMessage {
            self.messages
                .recv_timeout(TIMEOUT)
                .expect("child handshake")
        }

        fn expect_locked(&self) -> *mut c_void {
            match self.next_message() {
                ChildMessage::Locked(window) => window as *mut c_void,
                message => panic!("expected child lock acknowledgement, got {message:?}"),
            }
        }

        fn release(&mut self) {
            self.child
                .stdin
                .as_mut()
                .expect("child stdin pipe")
                .write_all(b"R")
                .expect("request clipboard release");
            // The child sends this only after CloseClipboard succeeds.
            assert_eq!(self.next_message(), ChildMessage::Closed);
        }

        fn expect_success(&mut self) {
            let deadline = Instant::now() + TIMEOUT;
            loop {
                if let Some(status) = self.child.try_wait().expect("poll clipboard holder") {
                    assert!(status.success(), "clipboard holder failed: {status}");
                    return;
                }
                assert!(Instant::now() < deadline, "clipboard holder did not exit");
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    impl Drop for LockChild {
        fn drop(&mut self) {
            // Only the owned process is terminated. Killing it closes stdout,
            // so the reader cannot remain blocked if an assertion panics.
            self.child.stdin.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
    }

    fn signal(message: &str) {
        let mut stdout = std::io::stdout().lock();
        // The leading newline separates this from libtest's progress prefix.
        writeln!(stdout, "\n{message}").expect("send child handshake");
        stdout.flush().expect("flush child handshake");
    }

    #[test]
    #[ignore = "helper process selected only by the real contention test"]
    fn clipboard_lock_child() {
        if std::env::var_os(CHILD_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
            return;
        }
        // A detached reader keeps a missing release command from holding the
        // clipboard forever. On timeout this test unwinds and releases it;
        // libtest process exit also terminates the blocked reader thread.
        let (sender, release) = mpsc::channel();
        thread::spawn(move || {
            let mut command = [0];
            let result = std::io::stdin()
                .read_exact(&mut command)
                .map(|()| command[0]);
            let _ = sender.send(result);
        });
        let window = TestWindow::new();
        let deadline = Instant::now() + TIMEOUT;
        let mut clipboard = loop {
            if let Some(clipboard) = ClipboardLock::try_open(&window) {
                break clipboard;
            }
            assert!(Instant::now() < deadline, "could not acquire clipboard");
            thread::sleep(Duration::from_millis(10));
        };
        assert_open_window(window.0, std::process::id());
        signal(&format!("{LOCKED}:{}", window.0 as usize));
        assert_eq!(
            release
                .recv_timeout(TIMEOUT)
                .expect("bounded release wait")
                .expect("read release command"),
            b'R'
        );
        assert_open_window(window.0, std::process::id());
        assert!(clipboard.close(), "child CloseClipboard failed");
        assert_ne!(unsafe { GetOpenClipboardWindow() }, window.0);
        signal(CLOSED);
    }

    #[test]
    fn real_clipboard_contention_retries_after_child_releases() {
        let window = TestWindow::new();
        let mut holder = LockChild::spawn();
        let child_window = holder.expect_locked();
        let child_process = holder.child.id();
        assert_ne!(child_process, std::process::id());
        assert_open_window(child_window, child_process);
        assert_ne!(child_window, window.0);
        let mut attempts = 0;
        let mut clipboard = None;
        assert!(open_with_retry(|| {
            attempts += 1;
            if attempts == 1 {
                assert_open_window(child_window, child_process);
            }
            clipboard = ClipboardLock::try_open(&window);
            if attempts == 1 {
                // This is a real OpenClipboard failure while a different
                // process holds the lock, not a fabricated closure result.
                assert!(clipboard.is_none(), "clipboard holder did not exclude us");
                assert_open_window(child_window, child_process);
                holder.release();
                false
            } else {
                clipboard.is_some()
            }
        }));
        assert!(attempts >= 2, "the real open operation was not retried");
        assert_open_window(window.0, std::process::id());
        assert!(
            clipboard
                .as_mut()
                .expect("parent acquired clipboard")
                .close(),
            "parent CloseClipboard failed"
        );
        assert_ne!(unsafe { GetOpenClipboardWindow() }, window.0);
        holder.expect_success();
    }
}
