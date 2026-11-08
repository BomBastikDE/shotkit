//! Confirm the end of a scroll capture through UI Automation (#88).
//!
//! Only asked after an automatic stop without further motion. The scrollable
//! element under the capture's mouse position (or its nearest scrollable
//! ancestor) must report its scroll position at the end in the capture
//! direction. Anything else (no ScrollPattern, not scrollable on that axis,
//! an error or a timeout) answers "not confirmed", which keeps the warning.
//!
//! The query runs on its own COM (MTA) thread with UIA timeouts and an overall
//! deadline, so a slow or hung provider can never delay the capture result.
use shotkit::scroll_capture::{scroll_percent_at_end, Direction};
use std::{sync::mpsc, thread, time::Duration};

const UIA_TIMEOUT_MS: u32 = 700;
const ANSWER_TIMEOUT: Duration = Duration::from_millis(1500);
const MAX_ANCESTORS: usize = 32;

/// `true` only if UI Automation positively reports the end for `direction`.
pub fn confirm_end(point: (i32, i32), direction: Direction) -> bool {
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("shotkit-scroll-end".into())
        .spawn(move || {
            // SAFETY: the thread owns its COM apartment and every interface
            // created by `query`; nothing crosses the thread boundary but the answer.
            let answer = unsafe { query(point, direction) };
            let _ = sender.send(answer);
        });
    if spawned.is_err() {
        return false;
    }
    // On timeout the detached thread finishes on its own; its answer is ignored.
    matches!(receiver.recv_timeout(ANSWER_TIMEOUT), Ok(Some(true)))
}

// SAFETY CONTRACT:
// Voraussetzung: Aufruf auf einem eigenen Thread ohne COM-Initialisierung.
// Warum erfüllt: confirm_end startet dafür einen neuen Thread; CoUninitialize läuft
// erst, nachdem alle Schnittstellen dieses Threads (lokale Variablen) freigegeben sind.
// Nachrichten: keine Fenster dieses Prozesses; UIA-Aufrufe sind durch Zeitlimits begrenzt.
unsafe fn query(point: (i32, i32), direction: Direction) -> Option<bool> {
    use windows::{
        core::Interface,
        Win32::{
            Foundation::POINT,
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_MULTITHREADED,
            },
            UI::Accessibility::{
                CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationScrollPattern,
                UIA_ScrollPatternId, UIA_ScrollPatternNoScroll,
            },
        },
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: balances the successful CoInitializeEx of this thread.
            unsafe { CoUninitialize() };
        }
    }
    if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
        return None;
    }
    let _apartment = Apartment;
    // All interfaces live inside the closure and are released before
    // `_apartment` uninitializes COM at the end of this function.
    (|| -> Option<bool> {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
        if let Ok(bounded) = automation.cast::<IUIAutomation2>() {
            let _ = bounded.SetTransactionTimeout(UIA_TIMEOUT_MS);
            let _ = bounded.SetConnectionTimeout(UIA_TIMEOUT_MS);
        }
        let walker = automation.ControlViewWalker().ok()?;
        let mut element = automation
            .ElementFromPoint(POINT {
                x: point.0,
                y: point.1,
            })
            .ok()?;
        for depth in 0..MAX_ANCESTORS {
            // Test diagnostics only (SHOTKIT_UIA_DIAGNOSTICS_PATH): control
            // type and class of each visited node, never names or contents.
            let control_type = element.CurrentControlType().map(|t| t.0).unwrap_or(-1);
            let class = element
                .CurrentClassName()
                .map(|name| name.to_string())
                .unwrap_or_default();
            crate::uia_picker::log_diagnostic(&format!(
                "scroll_end node depth={depth} control_type={control_type} class={class:.64}"
            ));
            if let Ok(pattern) =
                element.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)
            {
                let (scrollable, percent) = match direction {
                    Direction::Down => (
                        pattern.CurrentVerticallyScrollable().ok()?.as_bool(),
                        pattern.CurrentVerticalScrollPercent().ok()?,
                    ),
                    Direction::Right | Direction::Left => (
                        pattern.CurrentHorizontallyScrollable().ok()?.as_bool(),
                        pattern.CurrentHorizontalScrollPercent().ok()?,
                    ),
                };
                crate::uia_picker::log_diagnostic(&format!(
                    "scroll_end pattern depth={depth} scrollable={scrollable} percent={percent}"
                ));
                // The first element scrollable on this axis decides.
                if scrollable && percent != UIA_ScrollPatternNoScroll {
                    let at_end = scroll_percent_at_end(direction, percent);
                    crate::uia_picker::log_diagnostic(&format!(
                        "scroll_end decision at_end={at_end}"
                    ));
                    return Some(at_end);
                }
            }
            element = walker.GetParentElement(&element).ok()?;
        }
        None
    })()
}
