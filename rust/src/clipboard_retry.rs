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
    }

    // Open and close on the same test thread, without reading or writing data.
    struct ClipboardLock(bool);

    impl ClipboardLock {
        fn try_open() -> Option<Self> {
            // A null owner is valid when the clipboard is not being emptied.
            (unsafe { OpenClipboard(std::ptr::null_mut()) } != 0).then(|| Self(true))
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

    struct LockChild {
        child: Child,
        messages: Receiver<&'static str>,
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
                    let message = match line.as_str() {
                        LOCKED => LOCKED,
                        CLOSED => CLOSED,
                        _ => continue,
                    };
                    if sender.send(message).is_err() {
                        break;
                    }
                }
            }));
            holder
        }

        fn expect_message(&self, expected: &str) {
            assert_eq!(
                self.messages
                    .recv_timeout(TIMEOUT)
                    .expect("child handshake"),
                expected
            );
        }

        fn release(&mut self) {
            self.child
                .stdin
                .as_mut()
                .expect("child stdin pipe")
                .write_all(b"R")
                .expect("request clipboard release");
            // The child sends this only after CloseClipboard succeeds.
            self.expect_message(CLOSED);
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
        let deadline = Instant::now() + TIMEOUT;
        let mut clipboard = loop {
            if let Some(clipboard) = ClipboardLock::try_open() {
                break clipboard;
            }
            assert!(Instant::now() < deadline, "could not acquire clipboard");
            thread::sleep(Duration::from_millis(10));
        };
        signal(LOCKED);
        assert_eq!(
            release
                .recv_timeout(TIMEOUT)
                .expect("bounded release wait")
                .expect("read release command"),
            b'R'
        );
        assert!(clipboard.close(), "child CloseClipboard failed");
        signal(CLOSED);
    }

    #[test]
    fn real_clipboard_contention_retries_after_child_releases() {
        let mut holder = LockChild::spawn();
        holder.expect_message(LOCKED);
        let mut attempts = 0;
        let mut clipboard = None;
        assert!(open_with_retry(|| {
            attempts += 1;
            clipboard = ClipboardLock::try_open();
            if attempts == 1 {
                // This is a real OpenClipboard failure while a different
                // process holds the lock, not a fabricated closure result.
                assert!(clipboard.is_none(), "clipboard holder did not exclude us");
                holder.release();
                false
            } else {
                clipboard.is_some()
            }
        }));
        assert!(attempts >= 2, "the real open operation was not retried");
        assert!(
            clipboard
                .as_mut()
                .expect("parent acquired clipboard")
                .close(),
            "parent CloseClipboard failed"
        );
        holder.expect_success();
    }
}
