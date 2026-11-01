use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tauri::{AppHandle, Manager};

const TITLE_PREFIX: &str = "YAOP";
// 25 Hz: every tick is a Win32 read on the window the mirror draws into, and the only event it
// can catch that the page does not report itself is scrcpy resizing its own window (a rotation).
// At 60 Hz that idle loop was pure overhead next to the video stream.
const TICK: Duration = Duration::from_millis(40);

struct Handle {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// Owns the background thread that keeps the embedded video inside the box the page reported.
/// scrcpy is a separate process, so the window we draw into is found by the title we give it,
/// "YAOP - <serial>", and it can change that window's size without telling anyone.
#[derive(Default)]
pub struct MirrorTracker {
    inner: Mutex<Option<Handle>>,
}

impl MirrorTracker {
    fn ensure_running(&self, app: AppHandle) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        if let Some(handle) = guard.as_ref() {
            if !handle.stop.load(Ordering::Relaxed) {
                return;
            }
            if let Some(old) = guard.take() {
                old.stop.store(true, Ordering::Relaxed);
                let _ = old.thread.join();
            }
        }

        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let app = app.clone();
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || run(app, stop))
        };
        *guard = Some(Handle { stop, thread });
    }

    fn halt(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(old) = guard.take() {
                old.stop.store(true, Ordering::Relaxed);
                let _ = old.thread.join();
            }
        }
    }
}

pub fn start(app: &AppHandle) {
    app.state::<MirrorTracker>().ensure_running(app.clone());
}

pub fn stop(app: &AppHandle) {
    app.state::<MirrorTracker>().halt();
}

fn run(app: AppHandle, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(TICK);
        // The session decides whether a mirror still exists; once it is gone there is nothing
        // left to follow and this thread is finished.
        if !crate::host::wanted() {
            break;
        }
        // A no-op while the window is still being waited for, or after it refused to embed:
        // `sync` only writes geometry to a window this module actually owns.
        let _ = crate::host::sync(&app);
    }
}

#[cfg(windows)]
pub(crate) fn mirror_hwnd(serial: &str) -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    let title = format!("{TITLE_PREFIX} - {serial}");
    unsafe { FindWindowW(None, &HSTRING::from(&title)) }.ok()
}
