use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

/// Private message posted from the click notification, so the focus change runs after the
/// cross-thread `WM_PARENTNOTIFY` dispatch has returned to scrcpy's input thread.
const FOCUS_MESSAGE: u32 = 0x8000 + 0x59;
/// Re-run the hand-off after activation without claiming the keyboard; if the page took it
/// while the frame was reactivating, the owner is left alone.
const REASSERT_FOCUS_MESSAGE: u32 = FOCUS_MESSAGE + 1;
const CLICK_FOCUS_ID: usize = 0x5941_4F50;
/// Button mask (left/right/middle) last seen by the fallback check in `sync`; it only covers
/// a notification the system fails to deliver to a cross-process parent.
static BUTTONS_DOWN: AtomicU32 = AtomicU32::new(0);
/// Last time `sync` asked the UI thread to move the keyboard to the frame. A refused
/// transfer (another app taking the focus in the same instant) must not become one attempt
/// per 40 ms tick.
static LAST_FOCUS_PUSH: Mutex<Option<Instant>> = Mutex::new(None);
/// Last time the video was lifted back above the webview that shares its client area. The lift
/// is a z-order no-op, so one per second is enough to undo a lost order without bothering the
/// renderer.
static LAST_TOP: Mutex<Option<Instant>> = Mutex::new(None);
/// Whether the frame is holding the physical keyboard for the phone: every key it receives is
/// relayed to the embedded SDL window. Cleared by `reset`, which is the last point where a
/// relay target could disappear.
static FORWARD_KEYS: AtomicBool = AtomicBool::new(false);

/// The device whose mirror this module owns, and whether a session still wants one. `begin`
/// sets both; `end` clears them so a late scrcpy window from a cancelled session is never
/// adopted.
static SERIAL: Mutex<Option<String>> = Mutex::new(None);
static WANTED: AtomicBool = AtomicBool::new(false);
/// Keyguard verdict when the session started. A locked phone keeps the caret in the page,
/// where the credential field lives; an open one hands the keyboard to the mirror right away.
static LOCKED_AT_START: AtomicBool = AtomicBool::new(false);

/// Where the page says the video goes: a box in the parent's client area, in logical pixels,
/// reported by `set_mirror_rect`.
static PLACEHOLDER: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);

/// The box used before the page reports one, in logical pixels, so the video has somewhere to
/// land on the first tick instead of appearing at the window's origin.
const FALLBACK_RECT: (f64, f64, f64, f64) = (40.0, 60.0, 420.0, 640.0);

/// The phone's own proportions (width / height), taken from the size scrcpy settled on. The
/// video is fitted inside the placeholder with this ratio so it never stretches.
static ASPECT: Mutex<Option<f64>> = Mutex::new(None);

/// Set when the user hid the video, so a window move does not map it back.
static VIDEO_WANTED: AtomicBool = AtomicBool::new(true);

/// One embedded mirror at a time: the scrcpy top-level window becomes a child of the app
/// window, so the pair is one frame, one shadow and one taskbar entry. Every entry point has to
/// tolerate `is_attached() == false`, because the fold can fail and the mirror then stands on
/// its own.
struct Attached {
    parent: isize,
    child: isize,
}

static HOSTED: AtomicBool = AtomicBool::new(false);
static ATTACHING: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<Attached>> = Mutex::new(None);

const POLL: Duration = Duration::from_millis(60);
const ATTEMPTS: u32 = 200;
/// scrcpy shows a placeholder window (256x256) for a few hundred milliseconds before it
/// applies the video's size. Attaching during that gap would freeze the frame at the
/// placeholder and letterbox the phone with empty columns, so the window must keep the same
/// size for this long before it is folded in.
const STABLE: Duration = Duration::from_millis(400);
const MIN_MIRROR: i32 = 200;

pub fn is_attached() -> bool {
    HOSTED.load(Ordering::Relaxed)
}

/// Whether a session still expects a mirror here. The tracker thread stays alive while it
/// holds, and `begin` gives up the moment it is cleared.
pub fn wanted() -> bool {
    WANTED.load(Ordering::Relaxed)
}

pub fn reset() {
    if let Ok(mut guard) = STATE.lock() {
        *guard = None;
    }
    HOSTED.store(false, Ordering::SeqCst);
    FORWARD_KEYS.store(false, Ordering::SeqCst);
    VIDEO_WANTED.store(true, Ordering::SeqCst);
    if let Ok(mut guard) = ASPECT.lock() {
        *guard = None;
    }
}

/// Put the mirror away without touching the session: the video is hidden and the target is
/// forgotten, so a scrcpy window that appears later is not adopted.
pub fn end() {
    WANTED.store(false, Ordering::SeqCst);
    if let Ok(mut guard) = SERIAL.lock() {
        *guard = None;
    }
    reset();
}

pub fn target_serial() -> Option<String> {
    SERIAL.lock().ok().and_then(|guard| guard.clone())
}

/// Which side gets the physical keyboard while the mirror is embedded: the phone (every key is
/// relayed to scrcpy) or the page (the caret stays in its field).
pub fn set_keyboard_owner(app: &AppHandle, phone: bool) {
    FORWARD_KEYS.store(phone, Ordering::SeqCst);
    if phone {
        focus(app);
    } else if let Some(window) = app.get_webview_window(PARENT_LABEL) {
        let webview: &tauri::Webview<_> = window.as_ref();
        let _ = webview.set_focus();
    }
}

/// Whether the frame is holding the keyboard for the phone right now.
fn keys_to_mirror() -> bool {
    FORWARD_KEYS.load(Ordering::Relaxed)
}

fn tracked() -> Option<(isize, isize)> {
    STATE
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|item| (item.parent, item.child)))
}

/// The app window hosts the embedded mirror: one window, one taskbar entry, one frame.
const PARENT_LABEL: &str = "main";

/// The video's box in physical pixels inside the parent's client area: the page's placeholder
/// scaled by the window's DPI factor, then aspect-fitted and centred inside it.
fn target(scale: f64) -> (i32, i32, i32, i32) {
    let box_ = placeholder().unwrap_or(FALLBACK_RECT);
    target_rect(box_, aspect(), scale)
}

/// The box the page reported, if any.
fn placeholder() -> Option<(f64, f64, f64, f64)> {
    PLACEHOLDER.lock().ok().and_then(|guard| *guard)
}

fn aspect() -> Option<f64> {
    ASPECT.lock().ok().and_then(|guard| *guard)
}

/// The phone's proportions (width / height) taken from the scrcpy window, once it is known.
/// The page uses this to make the mirror column exactly as wide as the video at its height,
/// so a portrait phone never leaves empty bands on either side of the picture.
pub fn current_aspect() -> Option<f64> {
    aspect()
}

/// Tell the page the phone's proportions. Emitted when the mirror is folded in and whenever
/// scrcpy reports a new size (a rotation), so the column can follow without a poll.
pub fn emit_aspect(app: &AppHandle) {
    if let Some(aspect) = aspect() {
        let _ = app.emit("mirror-aspect", aspect);
    }
}

/// Fit `placeholder` (logical pixels) at `scale`, keeping the phone's proportions when known.
fn target_rect(
    placeholder: (f64, f64, f64, f64),
    aspect: Option<f64>,
    scale: f64,
) -> (i32, i32, i32, i32) {
    let (x, y, width, height) = placeholder;
    let grow = |value: f64| (value * scale).round() as i32;
    let (box_x, box_y) = (grow(x), grow(y));
    let (box_w, box_h) = (grow(width).max(1), grow(height).max(1));
    let Some(ratio) = aspect.filter(|value| *value > 0.0) else {
        return (box_x, box_y, box_w, box_h);
    };
    // The video keeps its proportions and is centred, so a wide column of the page never turns
    // the phone into a stretched picture.
    let (video_w, video_h) = if box_w as f64 / box_h as f64 > ratio {
        (grow(box_h as f64 * ratio), box_h)
    } else {
        (box_w, grow(box_w as f64 / ratio))
    };
    let (video_w, video_h) = (video_w.max(1), video_h.max(1));
    (
        box_x + (box_w - video_w) / 2,
        box_y + (box_h - video_h) / 2,
        video_w,
        video_h,
    )
}

/// The page owns the layout while the mirror lives in the app window: it reports the box the
/// video may occupy, in logical pixels inside the window's client area.
pub fn set_placeholder(rect: (f64, f64, f64, f64)) {
    if let Ok(mut guard) = PLACEHOLDER.lock() {
        *guard = Some(rect);
    }
    VIDEO_WANTED.store(rect.2 >= 2.0 && rect.3 >= 2.0, Ordering::SeqCst);
}

/// Take the phone's proportions from the window scrcpy chose, so the first placement already
/// matches the picture instead of stretching it for one tick.
fn remember_aspect(width: i32, height: i32) {
    if width < 2 || height < 2 {
        return;
    }
    if let Ok(mut guard) = ASPECT.lock() {
        *guard = Some(width as f64 / height as f64);
    }
}

/// scrcpy resizes its own window on a rotation. The video is the only source of truth for the
/// phone's proportions, so a size that disagrees with the stored ratio by more than a rounding
/// error is a new orientation and not a layout glitch.
fn adopt_aspect_if_changed(child_width: i32, child_height: i32) -> bool {
    if child_width < 2 || child_height < 2 {
        return false;
    }
    let ratio = child_width as f64 / child_height as f64;
    let rotated = match aspect() {
        Some(current) => (ratio / current - 1.0).abs() > 0.05,
        None => true,
    };
    if rotated {
        ASPECT.lock().ok().map(|mut guard| *guard = Some(ratio));
    }
    rotated
}

fn scale_of(app: &AppHandle) -> f64 {
    app.get_webview_window(PARENT_LABEL)
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0)
}

/// Start looking for the scrcpy window and fold it into the app window. Polls because the
/// mirror is a separate process and needs a moment to create its window.
pub fn begin(app: &AppHandle, serial: &str, locked: bool) {
    #[cfg(windows)]
    {
        if let Ok(mut guard) = SERIAL.lock() {
            *guard = Some(serial.to_string());
        }
        LOCKED_AT_START.store(locked, Ordering::SeqCst);
        WANTED.store(true, Ordering::SeqCst);
        if is_attached() || ATTACHING.swap(true, Ordering::SeqCst) {
            return;
        }
        let serial = serial.to_string();
        let app = app.clone();
        thread::spawn(move || {
            let mut settled: Option<(i32, i32)> = None;
            let mut since = std::time::Instant::now();
            let mut hidden: Option<isize> = None;
            for _ in 0..ATTEMPTS {
                // The video is still hidden on purpose, so the attempt is alive only while a
                // session wants it and targets the same device.
                if target_serial().as_deref() != Some(serial.as_str()) || !wanted() {
                    break;
                }
                if let Some(child) = crate::mirror::mirror_hwnd(&serial) {
                    // scrcpy draws its window as soon as it is created; hide it until the
                    // size settles so the only thing the user ever sees is the video in its
                    // final box. SDL maps the window again when the first frame arrives, which
                    // lands inside the stability wait, so the hide is repeated every pass — one
                    // SW_HIDE per 60 ms for under a second is cheaper than a flash of the loose
                    // mirror at its original position.
                    if hidden != Some(child.0 as isize) {
                        square_corners(child.0 as isize);
                        hidden = Some(child.0 as isize);
                    }
                    hide_window(child);
                    match client_size(child) {
                        Some(size) if size.0 >= MIN_MIRROR && size.1 >= MIN_MIRROR => {
                            if settled == Some(size) {
                                if since.elapsed() >= STABLE
                                    && attach(&app, child.0 as isize).is_ok()
                                {
                                    break;
                                }
                            } else {
                                settled = Some(size);
                                since = std::time::Instant::now();
                            }
                        }
                        _ => settled = None,
                    }
                } else {
                    settled = None;
                    hidden = None;
                }
                thread::sleep(POLL);
            }
            if !is_attached() && wanted() {
                // The fold did not happen. Put the mirror back on the desktop as a normal
                // window: scrcpy was started without a title bar because the app window was
                // going to be its chrome, so the style has to come back with it or the user
                // gets a picture they can neither move nor close. The page keeps working —
                // unlock, wake, screenshot and clipboard are pure ADB — so only the video
                // stands apart.
                if let Some(child) = hidden {
                    unsafe { detach_standalone(hwnd(child)) };
                }
            }
            ATTACHING.store(false, Ordering::SeqCst);
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (app, serial, locked);
    }
}

/// Keep the embedded video where the page says it belongs. scrcpy can change the size of its
/// window on its own (a rotation, a resolution switch) and the page can change the box (a
/// drawer opening, a zoom, the window moving to another monitor); both show up here within a
/// tick. This is also where the keyboard relay re-asserts its focus.
#[cfg(windows)]
pub fn sync(app: &AppHandle) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetParent, IsIconic, IsWindow, IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE,
    };

    let Some((parent, child)) = tracked() else {
        return Ok(());
    };
    let (parent, child) = (hwnd(parent), hwnd(child));
    if !unsafe { IsWindow(child).as_bool() } {
        reset();
        return Ok(());
    }
    // Clicking the video hands the keyboard to it (see `focus_on_mirror_click`). The subclass
    // normally catches the press itself; this poll only covers a `WM_PARENTNOTIFY` the system
    // does not deliver to a cross-process parent, and fires once per press instead of on
    // every tick while the button is held. Any button counts: scrcpy's relative mouse mode
    // takes a right click without the keyboard ever moving, which used to strand the keys in
    // the webview until the user happened to left-click the video again.
    let pressed = (unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0) as u32
        | (((unsafe { GetAsyncKeyState(VK_RBUTTON.0 as i32) } < 0) as u32) << 1)
        | (((unsafe { GetAsyncKeyState(VK_MBUTTON.0 as i32) } < 0) as u32) << 2);
    let fresh = pressed & !BUTTONS_DOWN.swap(pressed, Ordering::Relaxed);
    if fresh != 0 && cursor_over_mirror() {
        focus(app);
    }
    let Some((video_w, video_h)) = client_size(child) else {
        return Ok(());
    };
    if video_w < 2 || video_h < 2 {
        return Ok(());
    }
    // A minimized frame sits in the shell's parking spot with bogus child coordinates;
    // re-framing it there would move its restore position and still leave it invisible.
    // Restoring belongs to the tray reveal, which runs when the user asks for it.
    if unsafe { IsIconic(parent).as_bool() } {
        return Ok(());
    }

    // WebView2 takes the system focus whenever the window is activated — a taskbar click, a
    // restore from Win+D, a tray reveal, the first show — and every click on the page focuses it
    // again. While the phone holds the keyboard this window has to keep it: its procedure relays
    // every key to the embedded SDL window, which never receives keys of its own. Checked here
    // because activation is not a message this thread receives; `focus_on_mirror_click` only
    // covers clicks on the video itself. The page is full of fields, so the frame is only
    // re-asserted when the phone was given the keyboard — otherwise the caret would lose a
    // focus fight every tick.
    if keys_to_mirror() && frame_is_foreground(parent) && focused_window() != Some(parent) {
        push_focus(app);
    }

    // The page owns the layout: the app window is never resized, only the child moves — back
    // into its box when scrcpy resized itself (a rotation) or the window was dragged onto
    // another monitor.
    let reparented = unsafe { GetParent(child) }
        .map(|current| current != parent)
        .unwrap_or(true);
    if reparented {
        // scrcpy took its window back (a style change, a crash-restart): the embed is over, and
        // writing child coordinates into a top-level window would only move it across the
        // desktop.
        reset();
        return Ok(());
    }
    if !VIDEO_WANTED.load(Ordering::Relaxed) {
        // Hidden by the page (its box was reported empty): moving the window must not map the
        // video back. The session keeps running; only the picture is put away.
        if unsafe { IsWindowVisible(child).as_bool() } {
            let _ = unsafe { ShowWindow(child, SW_HIDE) };
        }
        return Ok(());
    }
    if adopt_aspect_if_changed(video_w, video_h) {
        emit_aspect(app);
    }
    let (x, y, width, height) = target(scale_of(app));
    // A size scrcpy picked for itself (a rotation) is answered by re-fitting, so the child
    // is only left alone when it already matches the box the page asked for.
    let placed = current_rect(child, parent)
        .map(|(current_x, current_y, current_w, current_h)| {
            (current_x - x).abs() <= 1
                && (current_y - y).abs() <= 1
                && (current_w - width).abs() <= 1
                && (current_h - height).abs() <= 1
        })
        .unwrap_or(false);
    if placed {
        if !(unsafe { IsWindowVisible(child).as_bool() }) {
            let _ = unsafe { ShowWindow(child, SW_SHOWNOACTIVATE) };
        }
        // The webview fills the same client area and repaints on its own schedule; once a
        // second the video is lifted back to the top of the client without moving it, so a
        // z-order that got lost is never left lost.
        reassert_top(child);
        return Ok(());
    }
    unsafe { place_child(child, x, y, width, height) }
}

#[cfg(not(windows))]
pub fn sync(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

/// Lift the embedded video back to the front of its parent's client area, at most once a
/// second. Position and size are left exactly as they are.
#[cfg(windows)]
fn reassert_top(child: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let now = Instant::now();
    let due = LAST_TOP
        .lock()
        .map(|mut last| {
            let due = last
                .as_ref()
                .map_or(true, |seen| now.duration_since(*seen) >= Duration::from_secs(1));
            if due {
                *last = Some(now);
            }
            due
        })
        .unwrap_or(false);
    if due {
        unsafe {
            let _ = SetWindowPos(
                child,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

/// Hand keyboard focus to the frame, the window that owns the key relay. Windows does not
/// deliver real keyboard input to a `WS_CHILD` window owned by another process, so focusing
/// scrcpy's SDL window directly looked right (`GetFocus` reported it) and still delivered
/// nothing; the frame is in this process and relays what it receives (see
/// `focus_on_mirror_click`). Dispatched to the UI thread: that is the thread whose input
/// queue owns the frame.
pub fn focus(app: &AppHandle) {
    let app = app.clone();
    let _ = app.run_on_main_thread(move || focus_now());
}

#[cfg(windows)]
fn focus_now() {
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

    let Some((parent, _)) = tracked() else {
        return;
    };
    let parent = hwnd(parent);
    let _ = unsafe { SetForegroundWindow(parent) };
    if hand_focus_to(parent) {
        return;
    }
    // The transfer is occasionally refused while the frame is being re-activated. A single
    // retry after a short beat is cheap and turns a silent no-op into a working keyboard.
    thread::sleep(Duration::from_millis(8));
    let _ = hand_focus_to(parent);
}

/// Move keyboard focus to the frame and report whether Windows actually took it. The call
/// runs on the frame's own thread, so `SetFocus` is not refused the way a cross-process
/// transfer can be.
#[cfg(windows)]
fn hand_focus_to(window: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};

    unsafe {
        let _ = SetFocus(window);
        GetFocus() == window
    }
}

#[cfg(not(windows))]
fn focus_now() {}

/// Rate-limited hand-off for the keyboard guard in `sync`; the transfer itself belongs to the
/// UI thread (see `focus`).
#[cfg(windows)]
fn push_focus(app: &AppHandle) {
    let now = Instant::now();
    let due = LAST_FOCUS_PUSH
        .lock()
        .map(|mut last| {
            let due = last
                .as_ref()
                .map_or(true, |seen| now.duration_since(*seen) >= Duration::from_millis(300));
            if due {
                *last = Some(now);
            }
            due
        })
        .unwrap_or(false);
    if due {
        focus(app);
    }
}

#[cfg(windows)]
fn frame_is_foreground(parent: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    unsafe { GetForegroundWindow() == parent }
}

/// The window the desktop currently routes keys to. Windows reports the same value from any
/// thread's `GetGUIThreadInfo` while the input queues are attached, which is exactly how the
/// hand-off works — so this is the authoritative answer to "who gets the next keystroke".
#[cfg(windows)]
fn focused_window() -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return None;
        }
        let thread = GetWindowThreadProcessId(foreground, None);
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(thread, &mut info).ok()?;
        (!info.hwndFocus.0.is_null()).then_some(info.hwndFocus)
    }
}

/// Frame window procedure: key relay and click-to-mirror focus.
///
/// Windows does not deliver real keyboard input to a `WS_CHILD` window owned by another
/// process — scrcpy's SDL window can report the focus and still never see a keystroke —
/// so the frame keeps the keyboard itself and posts every key message to the video. Posted
/// messages take the same path as real ones inside SDL, shortcuts included.
///
/// Clicking the video has to give it the keyboard too: Windows does not move focus to a
/// child window on click, so after any panel interaction the WebView2 keeps the keys.
/// `WM_PARENTNOTIFY` (sent when a button goes down over a child) hands focus to the frame —
/// but only when the cursor is really over the video, because clicking the panel must keep
/// its own focus.
#[cfg(windows)]
unsafe extern "system" fn focus_on_mirror_click(
    window: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_MENU, VK_TAB};
    use windows::Win32::UI::Shell::DefSubclassProc;
    use windows::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WA_INACTIVE, WM_ACTIVATE, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
        WM_MBUTTONDOWN, WM_PARENTNOTIFY, WM_RBUTTONDOWN, WM_SETFOCUS, WM_SYSCHAR,
        WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    if msg == FOCUS_MESSAGE {
        // A click landed on the video: from now on the physical keyboard belongs to the phone,
        // until the page takes it back (a field focused in the panel).
        FORWARD_KEYS.store(true, Ordering::Relaxed);
        focus_now();
        return LRESULT(0);
    }
    if msg == REASSERT_FOCUS_MESSAGE {
        if FORWARD_KEYS.load(Ordering::Relaxed) {
            focus_now();
        }
        return LRESULT(0);
    }
    if msg == WM_ACTIVATE && (wparam.0 as u32) & 0xFFFF == WA_INACTIVE {
        // Alt+Tab leaves the modifiers held: the key-up belongs to whichever window is
        // foreground when the keys are released, and that is no longer this one. SDL would
        // keep seeing Alt down, turning every later keystroke into a scrcpy shortcut or an
        // Alt chord on the phone. Releasing them now is what makes typing work again after
        // the frame is activated a second time.
        if let Some((_, child)) = tracked() {
            release_held_keys(hwnd(child));
        }
    }
    if msg == WM_ACTIVATE
        && (wparam.0 as u32) & 0xFFFF != WA_INACTIVE
        && FORWARD_KEYS.load(Ordering::Relaxed)
    {
        // WebView2 restores its own focus whenever the frame is activated again (Alt+Tab,
        // taskbar, tray reveal). Posting the hand-off runs after that activation sequence,
        // so the mirror gets the keyboard back instead of silently losing every key.
        let _ = unsafe {
            PostMessageW(
                window,
                REASSERT_FOCUS_MESSAGE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            )
        };
    }
    let relay = tracked();
    if let Some((_, child)) = relay {
        if msg == WM_KEYDOWN || msg == WM_KEYUP || msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP {
            // Alt+Tab is the Windows switcher's own gesture; the OS opens it regardless of
            // what this window does with the message. Forwarding only teaches the phone an
            // Alt+Tab nobody typed there, and strands Alt as soon as the switcher eats the
            // release. The Tab press therefore stops here.
            if (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN)
                && wparam.0 as u32 == VK_TAB.0 as u32
                && (unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } as i16) < 0
            {
                return LRESULT(0);
            }
            let _ = unsafe { PostMessageW(hwnd(child), msg, wparam, lparam) };
            return LRESULT(0);
        }
        // Generated by this thread's message loop from the key message above; answering 0
        // forwards nothing and keeps the default alert away.
        if msg == WM_CHAR || msg == WM_SYSCHAR {
            return LRESULT(0);
        }
        // wry answers `WM_SETFOCUS` by moving focus into the WebView2 document. While the phone
        // holds the keyboard there is no field to type into — the page's own fields are not this
        // window's focus — and the keys have to stay here so the relay above can send them on.
        if msg == WM_SETFOCUS && FORWARD_KEYS.load(Ordering::Relaxed) {
            return LRESULT(0);
        }
    }
    let event = (wparam.0 as u32) & 0xFFFF;
    if msg == WM_PARENTNOTIFY
        && (event == WM_LBUTTONDOWN || event == WM_RBUTTONDOWN || event == WM_MBUTTONDOWN)
        && cursor_over_mirror()
    {
        let _ = unsafe {
            PostMessageW(
                window,
                FOCUS_MESSAGE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            )
        };
    }
    unsafe { DefSubclassProc(window, msg, wparam, lparam) }
}

#[cfg(windows)]
fn release_held_keys(child: windows::Win32::Foundation::HWND) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
        VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_TAB, MAPVK_VK_TO_VSC,
    };
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYUP};

    // Releasing a key that is already up is a no-op for SDL, so every candidate goes over
    // the list rather than guessing which one a switcher actually stranded. The right-hand
    // modifiers and the Win keys are extended: without bit 24 SDL reads them as other keys.
    const HELD: [(i32, bool); 12] = [
        (VK_SHIFT.0 as i32, false),
        (VK_LSHIFT.0 as i32, false),
        (VK_RSHIFT.0 as i32, true),
        (VK_CONTROL.0 as i32, false),
        (VK_LCONTROL.0 as i32, false),
        (VK_RCONTROL.0 as i32, true),
        (VK_MENU.0 as i32, false),
        (VK_LMENU.0 as i32, false),
        (VK_RMENU.0 as i32, true),
        (VK_TAB.0 as i32, false),
        (VK_LWIN.0 as i32, true),
        (VK_RWIN.0 as i32, true),
    ];

    for (vk, extended) in HELD {
        let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) };
        let keyup =
            1u32 | (scan << 16) | ((extended as u32) << 24) | (1 << 30) | (1 << 31);
        let _ = unsafe {
            PostMessageW(
                child,
                WM_KEYUP,
                WPARAM(vk as usize),
                LPARAM(keyup as isize),
            )
        };
    }
}

#[cfg(windows)]
fn cursor_over_mirror() -> bool {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, IsChild, WindowFromPoint};

    let Some((_, child)) = tracked() else {
        return false;
    };
    let child = hwnd(child);
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return false;
    }
    let hit = unsafe { WindowFromPoint(point) };
    !hit.0.is_null() && unsafe { IsChild(child, hit).as_bool() }
}

/// Install the click listener on the frame window. `attach` runs on the polling thread while
/// subclassing belongs to the thread that owns the window, so it is dispatched to the UI
/// thread; the frame outlives every session, so the subclass is never removed.
#[cfg(windows)]
fn listen_for_clicks(app: &AppHandle, parent: isize) {
    let app = app.clone();
    let _ = app.run_on_main_thread(move || {
        use windows::Win32::UI::Shell::{SetWindowSubclass, SUBCLASSPROC};

        let proc: SUBCLASSPROC = Some(focus_on_mirror_click);
        unsafe {
            let _ = SetWindowSubclass(hwnd(parent), proc, CLICK_FOCUS_ID, 0);
        }
    });
}

#[cfg(not(windows))]
fn listen_for_clicks(_app: &AppHandle, _parent: isize) {}

#[cfg(windows)]
fn attach(app: &AppHandle, child: isize) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetParent, GetWindowLongPtrW, IsWindow, SetParent, SetWindowLongPtrW, GWL_STYLE, WS_CHILD,
        WS_POPUP,
    };

    let child = hwnd(child);
    if !unsafe { IsWindow(child).as_bool() } {
        return Err("Jendela cermin belum siap.".to_string());
    }
    let parent = app
        .get_webview_window(PARENT_LABEL)
        .and_then(|window| window.hwnd().ok())
        .ok_or("Jendela utama tidak tersedia.")?;
    let parent = hwnd(parent.0 as isize);
    let rect = video_rect(child).ok_or("Gagal membaca jendela cermin.")?;
    if rect.2 - rect.0 < 2 || rect.3 - rect.1 < 2 {
        return Err("Jendela cermin belum memiliki ukuran.".to_string());
    }
    let scale = scale_of(app);

    HOSTED.store(true, Ordering::SeqCst);
    // scrcpy's window is a plain rectangle and the video must reach its corners. The app window
    // is chrome the user knows, so its corners are left to the OS.
    square_corners(child.0 as isize);
    let style = unsafe { GetWindowLongPtrW(child, GWL_STYLE) };
    let child_style = (style & !(WS_POPUP.0 as isize)) | (WS_CHILD.0 as isize);
    unsafe { SetWindowLongPtrW(child, GWL_STYLE, child_style) };
    let _ = unsafe { SetParent(child, parent) };
    let parented = unsafe { GetParent(child) }
        .map(|current| current == parent)
        .unwrap_or(false);
    if !parented {
        unsafe { SetWindowLongPtrW(child, GWL_STYLE, style) };
        HOSTED.store(false, Ordering::SeqCst);
        return Err("Jendela cermin menolak menjadi satu jendela.".to_string());
    }

    // The page owns the layout: the video takes the box it reported and the window around it
    // is never resized. scrcpy's own size is the reference for the phone's proportions.
    remember_aspect(rect.2 - rect.0, rect.3 - rect.1);
    emit_aspect(app);
    let (x, y, width, height) = target(scale);
    let placed = unsafe { place_child(child, x, y, width, height) };
    if let Err(error) = placed {
        unsafe {
            let _ = SetParent(child, windows::Win32::Foundation::HWND::default());
            SetWindowLongPtrW(child, GWL_STYLE, style);
        }
        HOSTED.store(false, Ordering::SeqCst);
        return Err(error);
    }

    if let Ok(mut guard) = STATE.lock() {
        *guard = Some(Attached {
            parent: parent.0 as isize,
            child: child.0 as isize,
        });
    }
    listen_for_clicks(app, parent.0 as isize);
    // There is no frame to reveal: the app window is already on screen and the video came with
    // it. Only a phone that is already open hands the keyboard to the mirror; a locked one keeps
    // the caret in the page, where the credential field lives.
    let unlocked = !LOCKED_AT_START.load(Ordering::Relaxed);
    FORWARD_KEYS.store(unlocked, Ordering::SeqCst);
    if unlocked {
        focus(app);
    }
    Ok(())
}

/// Windows 11 rounds the corners of top-level windows by default. The panel is a plain
/// rectangle and the embedded video must reach its corners, so both opt out. On Windows 10
/// the attribute is rejected (and corners were never rounded), so the error is ignored.
#[cfg(windows)]
pub fn square_corners(handle: isize) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };

    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd(handle),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_DONOTROUND as *const _ as *const core::ffi::c_void,
            std::mem::size_of_val(&DWMWCP_DONOTROUND) as u32,
        );
    }
}

#[cfg(not(windows))]
pub fn square_corners(_hwnd: isize) {}

#[cfg(windows)]
fn hide_window(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

#[cfg(windows)]
fn show_window(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNA};

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNA);
    }
}

#[cfg(windows)]
fn hwnd(value: isize) -> windows::Win32::Foundation::HWND {
    windows::Win32::Foundation::HWND(value as *mut std::ffi::c_void)
}

#[cfg(windows)]
fn rect_of(hwnd: windows::Win32::Foundation::HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some((rect.left, rect.top, rect.right, rect.bottom))
}

#[cfg(windows)]
fn client_size(hwnd: windows::Win32::Foundation::HWND) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }.ok()?;
    Some((rect.right - rect.left, rect.bottom - rect.top))
}

/// Screen position of a window's client area. Compared against the frame's target so `sync`
/// notices a window that was moved away (or pushed off screen by a display change) without
/// its size changing.
#[cfg(windows)]
fn client_origin(hwnd: windows::Win32::Foundation::HWND) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;

    let mut origin = POINT::default();
    unsafe { ClientToScreen(hwnd, &mut origin) }
        .as_bool()
        .then_some((origin.x, origin.y))
}

/// The video's box expressed in its parent's client coordinates — the same space `place_child`
/// writes, so `sync` can compare what it asked for with what the child ended up with.
#[cfg(windows)]
fn current_rect(
    child: windows::Win32::Foundation::HWND,
    parent: windows::Win32::Foundation::HWND,
) -> Option<(i32, i32, i32, i32)> {
    let ((x, y), (width, height)) = client_origin(child).zip(client_size(child))?;
    let (origin_x, origin_y) = client_origin(parent)?;
    Some((x - origin_x, y - origin_y, width, height))
}

/// The rectangle the video actually draws into, in screen coordinates: the window's client
/// area. Used as the geometry source everywhere, so a non-client frame around the embedded
/// window can never be mistaken for part of the picture.#[cfg(windows)]
fn video_rect(hwnd: windows::Win32::Foundation::HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;

    let (width, height) = client_size(hwnd)?;
    let mut origin = POINT::default();
    if !unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
        return None;
    }
    Some((origin.x, origin.y, origin.x + width, origin.y + height))
}

/// Non-client offsets of the control window. tao keeps a resize frame around a window whose
/// decorations are off, so the outer rectangle from GetWindowRect is a few pixels larger than
/// the client area the webview and the video actually share.
#[derive(Clone, Copy, Default)]
struct Margins {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
fn margins(hwnd: windows::Win32::Foundation::HWND) -> Option<Margins> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;

    let outer = rect_of(hwnd)?;
    let (client_w, client_h) = client_size(hwnd)?;
    let mut origin = POINT::default();
    if !unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
        return None;
    }
    Some(Margins {
        left: origin.x - outer.0,
        top: origin.y - outer.1,
        right: outer.2 - (origin.x + client_w),
        bottom: outer.3 - (origin.y + client_h),
    })
}

#[cfg(windows)]
unsafe fn place_child(
    child: windows::Win32::Foundation::HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOP, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    };

    // `width` x `height` is the video area; the child's own non-client frame, if it kept
    // one, is added around it so the renderer gets exactly those pixels.
    let frame = margins(child).unwrap_or_default();
    // HWND_TOP keeps the video above the webview that fills the same client area.
    SetWindowPos(
        child,
        HWND_TOP,
        x - frame.left,
        y - frame.top,
        width + frame.left + frame.right,
        height + frame.top + frame.bottom,
        SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_SHOWWINDOW,
    )
    .map_err(|error| format!("Gagal menempatkan cermin: {error}"))
}

/// Hand a mirror that refused to be embedded back to the desktop as a normal window. scrcpy was
/// started without a title bar because this window was going to be its chrome, so the style has
/// to come back with it — otherwise the user is left with a picture they can neither move nor
/// close. The page keeps working: unlock, wake, screenshot and clipboard are pure ADB.
#[cfg(windows)]
unsafe fn detach_standalone(child: windows::Win32::Foundation::HWND) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CHILD, WS_OVERLAPPEDWINDOW,
    };

    let style = GetWindowLongPtrW(child, GWL_STYLE);
    SetWindowLongPtrW(
        child,
        GWL_STYLE,
        (style & !(WS_CHILD.0 as isize)) | WS_OVERLAPPEDWINDOW.0 as isize,
    );
    // The style change only takes effect on the next frame calculation.
    let _ = SetWindowPos(
        child,
        HWND::default(),
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
    );
    show_window(child);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_box_is_scaled_as_reported() {
        // No known proportions yet: the video occupies the whole box the page asked for.
        assert_eq!(
            target_rect((40.0, 60.0, 420.0, 640.0), None, 1.0),
            (40, 60, 420, 640)
        );
        assert_eq!(
            target_rect((40.0, 60.0, 420.0, 640.0), None, 1.5),
            (60, 90, 630, 960)
        );
    }

    #[test]
    fn a_tall_box_narrows_the_video_instead_of_stretching_it() {
        // A phone that is 9 wide for every 19 tall, given a square-ish column.
        let (x, y, width, height) = target_rect((100.0, 0.0, 600.0, 800.0), Some(9.0 / 19.0), 1.0);
        assert_eq!((width, height), (379, 800));
        assert_eq!(x, 100 + (600 - 379) / 2);
        assert_eq!(y, 0);
    }

    #[test]
    fn a_wide_box_shortens_the_video_and_centres_it() {
        let (x, y, width, height) = target_rect((0.0, 20.0, 900.0, 400.0), Some(2.0), 1.0);
        assert_eq!((width, height), (800, 400));
        assert_eq!((x, y), (50, 20));
    }
}
