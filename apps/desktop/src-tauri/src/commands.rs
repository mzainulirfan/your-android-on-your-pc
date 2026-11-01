use crate::adb::{self, AdbStatus, WirelessInfo};
use crate::chat_target::{self, TargetView};
use crate::clipboard_image::{self, ImageSendResult};
use crate::credentials::{self, CredentialView};
use crate::files::{self, AndroidEntry};
use crate::locate::{self, CaptureMode, Lens, LocateReport};
use crate::screenshot::{self, ScreenshotResult};
use crate::session::{self, ScrcpyStatus, SessionInfo, SessionManager};
use crate::settings::{self, Settings};
use crate::transfer::{self, PullResult, TransferResult};
use crate::unlock::{self, UnlockReport};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{
    AppHandle, LogicalUnit, Manager, PhysicalPosition, PixelUnit, Position, Size, State, WebviewWindow,
    WindowSizeConstraints,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub adb: AdbStatus,
    pub scrcpy: ScrcpyStatus,
    pub config_path: Option<String>,
    pub download_dir: String,
    pub platform: String,
    pub app_version: String,
}

/// ADB work is a blocking subprocess call. Running it on the IPC thread would freeze the
/// window for as long as the device takes to answer, so every command below that touches
/// ADB or scrcpy is dispatched to the blocking pool.
#[tauri::command]
pub async fn get_adb_status() -> AdbStatus {
    tauri::async_runtime::spawn_blocking(adb::status)
        .await
        .unwrap_or_else(|error| AdbStatus {
            found: false,
            path: None,
            version: None,
            error: Some(format!("Pemeriksaan ADB gagal: {error}")),
        })
}

/// The poller's answer, or a scan when it has not run yet. The page used to run its own
/// `adb devices` on mount beside the poller's, which spawned two clients for the same fact
/// and could miss the change event by a few milliseconds; a cached snapshot fixes both.
/// An explicit refresh passes `fresh`, so the user is never shown a stale answer.
#[tauri::command]
pub async fn get_devices_snapshot(
    app: AppHandle,
    fresh: bool,
) -> Result<crate::DevicesSnapshot, String> {
    if !fresh {
        let cached = app
            .state::<crate::DevicesCache>()
            .0
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(snapshot) = cached {
            return Ok(snapshot);
        }
    }

    let previous = app
        .state::<crate::DevicesCache>()
        .0
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        crate::snapshot_or_previous(previous.as_ref(), adb::list_devices())
    })
    .await
    .map_err(|error| format!("Gagal membaca daftar perangkat: {error}"))?;

    if let Ok(mut guard) = app.state::<crate::DevicesCache>().0.lock() {
        *guard = Some(snapshot.clone());
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn enable_wireless(app: AppHandle, serial: String) -> Result<WirelessInfo, String> {
    let result = tauri::async_runtime::spawn_blocking(move || adb::enable_wireless(&serial))
        .await
        .map_err(|error| format!("Handoff Wi-Fi gagal dijalankan: {error}"))??;

    let _ = settings::remember_wireless(&app, &result.wifi_serial);
    Ok(result)
}

/// Wireless addresses the app remembers, newest first.
#[tauri::command]
pub fn get_known_wireless(app: AppHandle) -> Vec<String> {
    settings::known_wireless(&app)
}

/// Connect to a phone that is already listening on the network by address, without a cable.
#[tauri::command]
pub async fn connect_wireless(app: AppHandle, address: String) -> Result<String, String> {
    let address = tauri::async_runtime::spawn_blocking(move || adb::connect_address(&address))
        .await
        .map_err(|error| format!("Koneksi Wi-Fi gagal dijalankan: {error}"))??;

    settings::remember_wireless(&app, &address)?;
    Ok(address)
}

/// Pair with a phone's Wireless debugging dialog (Android 11+). The pairing port is not a
/// reconnectable address — it lives only while the dialog is open — so nothing is stored.
#[tauri::command]
pub async fn pair_wireless(address: String, code: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || adb::pair_address(&address, &code))
        .await
        .map_err(|error| format!("Pairing gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn forget_wireless(app: AppHandle, address: String) -> Result<(), String> {
    let worker = address.clone();
    tauri::async_runtime::spawn_blocking(move || adb::disconnect_address(&worker).ok())
        .await
        .map_err(|error| format!("Pemutusan gagal dijalankan: {error}"))?;

    settings::forget_wireless(&app, &adb::normalize_wireless_address(&address)?)
}

#[tauri::command]
pub async fn set_screen_power(serial: String, sleep: bool) -> Result<adb::WakeState, String> {
    tauri::async_runtime::spawn_blocking(move || adb::set_screen_power(&serial, sleep))
        .await
        .map_err(|error| format!("Gagal mengubah status layar: {error}"))?
}

/// Put the phone's display away (or back) through the live scrcpy session itself instead of
/// a key event: the phone stays awake, so the keyguard is never armed and the mirror keeps
/// showing a usable screen. Fails when no mirror is embedded; the caller then falls back to
/// the plain ADB sleep. `on = false` hides the display, `on = true` lights it.
#[tauri::command]
pub fn scrcpy_screen_power(on: bool) -> Result<(), String> {
    crate::host::screen_power_via_scrcpy(on)
}

#[tauri::command]
pub async fn open_find_device(serial: String) -> Result<LocateReport, String> {
    tauri::async_runtime::spawn_blocking(move || locate::find_device(&serial))
        .await
        .map_err(|error| format!("Pelacakan gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn unlock_device(serial: String, secret: String) -> Result<UnlockReport, String> {
    tauri::async_runtime::spawn_blocking(move || unlock::unlock(&serial, &secret))
        .await
        .map_err(|error| format!("Penguncian gagal dijalankan: {error}"))?
}

/// The page owns the layout once the mirror lives inside it: it reports the box the video may
/// occupy, in logical pixels inside the window's client area. Rust only ever follows that box —
/// the window itself is never resized — and an empty box means the video is put away while the
/// session keeps running.
#[tauri::command]
pub fn set_mirror_rect(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let inside = |value: f64| value.is_finite() && (0.0..=16_384.0).contains(&value);
    if !(inside(x) && inside(y) && inside(width) && inside(height)) {
        return Err("Ukuran area cermin tidak valid.".to_string());
    }
    crate::host::set_placeholder((x, y, width, height));
    crate::host::sync(&app)
}

/// Which side gets the physical keyboard while the mirror is embedded. Typing into a field in
/// the panel answers "pc"; a click on the video — or the panel handing control back — answers
/// "phone", and every keystroke is relayed to scrcpy from then on.
#[tauri::command]
pub fn set_keyboard_owner(app: AppHandle, owner: String) -> Result<(), String> {
    match owner.as_str() {
        "phone" => crate::host::set_keyboard_owner(&app, true),
        "pc" => crate::host::set_keyboard_owner(&app, false),
        other => return Err(format!("Pemilik keyboard tidak dikenal: {other}")),
    }
    Ok(())
}

/// The phone's proportions taken from the embedded mirror, once the scrcpy window settled.
/// A reloaded page asks for this so the mirror column is sized correctly before the next
/// `mirror-aspect` event arrives.
#[tauri::command]
pub fn get_mirror_aspect() -> Option<f64> {
    crate::host::current_aspect()
}

fn logical_pixels(value: f64) -> PixelUnit {
    PixelUnit::Logical(LogicalUnit::new(value))
}

fn window_constraints(min_width: f64, max_width: Option<f64>) -> WindowSizeConstraints {
    WindowSizeConstraints {
        min_width: Some(logical_pixels(min_width)),
        min_height: Some(logical_pixels(560.0)),
        max_width: max_width.map(logical_pixels),
        max_height: None,
    }
}

/// Everything needed to animate a width change, captured before the first frame.
struct WindowFrame {
    inner_height: f64,
    start_width: f64,
    start_x: i32,
    target_width: f64,
    target_x: i32,
    y: i32,
}

/// Keep the left edge as the resize origin, but slide the frame left when growing would push
/// its right side past the work area. If it is wider than the work area it sticks to the left.
fn fit_in_work_area(x: i32, outer_width: i32, work_left: i32, work_right: i32) -> i32 {
    let mut x = x;
    if x + outer_width > work_right {
        x = work_right - outer_width;
    }
    x.max(work_left)
}

/// Plan a width change. `dock_right` moves the resize origin from the left edge to the
/// monitor's right edge: the frame grows leftward and shrinks rightward, so folding the
/// admin panel parks the phone against the side of the screen instead of leaving it where
/// the wide window started.
fn plan_window_frame(window: &WebviewWindow, target_width: f64, dock_right: bool) -> Result<WindowFrame, String> {
    let scale = window
        .scale_factor()
        .map_err(|error| format!("Gagal membaca skala jendela: {error}"))?;
    let inner = window
        .inner_size()
        .map(|size| size.to_logical::<f64>(scale))
        .map_err(|error| format!("Gagal membaca ukuran jendela: {error}"))?;
    let outer = window
        .outer_size()
        .map_err(|error| format!("Gagal membaca ukuran luar jendela: {error}"))?;
    let position = window
        .outer_position()
        .map_err(|error| format!("Gagal membaca posisi jendela: {error}"))?;

    let target_outer_width =
        outer.width as i32 + ((target_width - inner.width) * scale).round() as i32;
    let mut target_x = position.x;
    if let Ok(Some(monitor)) = window.current_monitor() {
        let work = *monitor.work_area();
        let left = work.position.x;
        let right = work.position.x + work.size.width as i32;
        // Docked: the frame's right edge becomes the anchor, so it shrinks toward the screen's
        // right and grows leftward. `fit_in_work_area` below still clamps a too-wide frame.
        if dock_right {
            target_x = right - target_outer_width;
        }
        target_x = fit_in_work_area(target_x, target_outer_width, left, right);
    }

    Ok(WindowFrame {
        inner_height: inner.height,
        start_width: inner.width,
        start_x: position.x,
        target_width,
        target_x,
        y: position.y,
    })
}

fn apply_window_frame(
    window: &WebviewWindow,
    frame: &WindowFrame,
    min_width: f64,
    lock_width: bool,
) -> Result<(), String> {
    window
        .set_size(Size::Logical(tauri::LogicalSize::new(
            frame.target_width,
            frame.inner_height,
        )))
        .map_err(|error| format!("Gagal mengubah lebar jendela: {error}"))?;
    window
        .set_position(Position::Physical(PhysicalPosition::new(
            frame.target_x,
            frame.y,
        )))
        .map_err(|error| format!("Gagal memindahkan jendela: {error}"))?;

    let constraints = if lock_width {
        window_constraints(frame.target_width, Some(frame.target_width))
    } else {
        window_constraints(min_width, None)
    };
    window
        .set_size_constraints(constraints)
        .map_err(|error| format!("Gagal mengatur batas lebar jendela: {error}"))
}

/// Resize the main window to an exact client width, keeping its height and (when it fits)
/// its left edge. The mirror column uses this when the admin panel is folded away so the
/// frame hugs the phone instead of leaving empty bands beside it. `min_width` is lowered with
/// the size so Windows accepts a portrait-shaped window; showing the panel restores the 720 px
/// floor.
///
/// While the panel is folded, `lock_width` pins the client width between min and max. The
/// mirror's box is derived from the window height, so a manual horizontal drag would either
/// letterbox the phone or bring the empty band back; vertical resizing stays allowed and the
/// frontend asks for a new locked width as the height changes.
#[tauri::command]
pub fn set_window_width(
    app: AppHandle,
    width: f64,
    min_width: f64,
    lock_width: bool,
    dock_right: bool,
) -> Result<(), String> {
    let inside = |value: f64| value.is_finite() && (160.0..=8192.0).contains(&value);
    if !inside(width) || !inside(min_width) {
        return Err("Lebar jendela tidak valid.".to_string());
    }

    // A new direct resize wins over any animation still in flight.
    WINDOW_ANIMATION_GENERATION.fetch_add(1, Ordering::SeqCst);

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Jendela utama tidak tersedia.".to_string())?;
    let frame = plan_window_frame(&window, width, dock_right)?;

    // Release any previous upper bound before moving: a max width left from the last locked
    // frame would clamp a window that has to grow because the phone got taller.
    let floor = if lock_width { width } else { min_width };
    window
        .set_size_constraints(window_constraints(floor, None))
        .map_err(|error| format!("Gagal mengatur batas lebar jendela: {error}"))?;

    apply_window_frame(&window, &frame, min_width, lock_width)
}

/// Grow the main window to the current monitor's work area, top to bottom. The height is
/// measured including the title bar/borders so the outer frame fits the usable screen exactly;
/// the width is left alone because the mirror's aspect effect chooses it right after.
#[tauri::command]
pub fn fit_window_height(app: AppHandle) -> Result<(), String> {
    // A height fit changes the same frame the width animation writes; cancel it first so it
    // cannot restore the old (shorter) height on its next tick.
    WINDOW_ANIMATION_GENERATION.fetch_add(1, Ordering::SeqCst);

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Jendela utama tidak tersedia.".to_string())?;
    let scale = window
        .scale_factor()
        .map_err(|error| format!("Gagal membaca skala jendela: {error}"))?;
    let inner = window
        .inner_size()
        .map_err(|error| format!("Gagal membaca ukuran jendela: {error}"))?;
    let outer = window
        .outer_size()
        .map_err(|error| format!("Gagal membaca ukuran luar jendela: {error}"))?;
    let monitor = window
        .current_monitor()
        .map_err(|error| format!("Gagal membaca monitor: {error}"))?
        .ok_or_else(|| "Monitor tidak terdeteksi.".to_string())?;
    let work = *monitor.work_area();

    let non_client_height = outer.height.saturating_sub(inner.height) as i32;
    let target_inner_height =
        (work.size.height as i32 - non_client_height).max(1) as f64 / scale;
    // Dock the frame to the screen's right edge for the session: every later panel
    // fold/grow keeps this right edge, so the phone mirror behaves like a right sidebar.
    let work_right = work.position.x + work.size.width as i32;
    let x = fit_in_work_area(
        work_right - outer.width as i32,
        outer.width as i32,
        work.position.x,
        work_right,
    );

    window
        .set_size(Size::Logical(tauri::LogicalSize::new(
            inner.width as f64 / scale,
            target_inner_height,
        )))
        .map_err(|error| format!("Gagal menyesuaikan tinggi jendela: {error}"))?;
    window
        .set_position(Position::Physical(PhysicalPosition::new(
            x,
            work.position.y,
        )))
        .map_err(|error| format!("Gagal memindahkan jendela: {error}"))?;

    Ok(())
}

const WINDOW_ANIMATION_MS: u64 = 220;
const WINDOW_ANIMATION_FRAME_MS: u64 = 16;
static WINDOW_ANIMATION_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The same width/position change as `set_window_width`, but eased over ~220 ms so folding
/// the admin panel does not snap. A newer request (or a direct resize) cancels the running
/// one through the generation counter.
#[tauri::command]
pub async fn animate_window_width(
    app: AppHandle,
    width: f64,
    min_width: f64,
    lock_width: bool,
    dock_right: bool,
) -> Result<(), String> {
    let inside = |value: f64| value.is_finite() && (160.0..=8192.0).contains(&value);
    if !inside(width) || !inside(min_width) {
        return Err("Lebar jendela tidak valid.".to_string());
    }

    // Claim the generation here, not inside the blocking task: whichever command was issued
    // last must win even if the pool happens to run the closures out of order.
    let generation = WINDOW_ANIMATION_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn_blocking(move || {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "Jendela utama tidak tersedia.".to_string())?;
        animate_window_frame(window, width, min_width, lock_width, dock_right, generation)
    })
    .await
    .map_err(|error| format!("Gagal menganimasikan jendela: {error}"))?
}

fn animate_window_frame(
    window: WebviewWindow,
    width: f64,
    min_width: f64,
    lock_width: bool,
    dock_right: bool,
    generation: u64,
) -> Result<(), String> {
    let frame = plan_window_frame(&window, width, dock_right)?;

    // Intermediate widths must be allowed even when the final frame is locked again.
    window
        .set_size_constraints(window_constraints(160.0, None))
        .map_err(|error| format!("Gagal melepas batas lebar jendela: {error}"))?;

    let delta_width = frame.target_width - frame.start_width;
    let delta_x = frame.target_x - frame.start_x;
    if delta_width.abs() < 0.5 && delta_x == 0 {
        return apply_window_frame(&window, &frame, min_width, lock_width);
    }

    let frames = (WINDOW_ANIMATION_MS / WINDOW_ANIMATION_FRAME_MS).max(1);
    for step in 1..=frames {
        if WINDOW_ANIMATION_GENERATION.load(Ordering::SeqCst) != generation {
            return Ok(());
        }
        let progress = step as f64 / frames as f64;
        let eased = 1.0 - (1.0 - progress).powi(3);
        let frame_width = frame.start_width + delta_width * eased;
        let frame_x = frame.start_x as f64 + delta_x as f64 * eased;
        window
            .set_size(Size::Logical(tauri::LogicalSize::new(
                frame_width,
                frame.inner_height,
            )))
            .map_err(|error| format!("Gagal menganimasikan lebar jendela: {error}"))?;
        window
            .set_position(Position::Physical(PhysicalPosition::new(
                frame_x.round() as i32,
                frame.y,
            )))
            .map_err(|error| format!("Gagal menganimasikan posisi jendela: {error}"))?;
        std::thread::sleep(Duration::from_millis(WINDOW_ANIMATION_FRAME_MS));
    }

    if WINDOW_ANIMATION_GENERATION.load(Ordering::SeqCst) != generation {
        return Ok(());
    }
    apply_window_frame(&window, &frame, min_width, lock_width)
}

#[tauri::command]
pub async fn screen_state(serial: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || adb::screen_probe(&serial))
        .await
        .map_err(|error| format!("Gagal membaca status layar: {error}"))?
}

/// Both live states in one device read, for the status poll that runs while mirroring.
#[tauri::command]
pub async fn wake_state(serial: String) -> Result<adb::WakeState, String> {
    tauri::async_runtime::spawn_blocking(move || adb::wake_state(&serial))
        .await
        .map_err(|error| format!("Gagal membaca status layar dan kunci: {error}"))?
}

/// Only the keyguard flag, for the panel while it waits to be dismissed. The panel polls this
/// because the phone can also be unlocked outside it — a PIN typed into the mirror over uhid,
/// or a fingerprint on the device — and the form has to fold away when that happens.
#[tauri::command]
pub async fn keyguard_locked(serial: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || adb::keyguard_probe(&serial))
        .await
        .map_err(|error| format!("Gagal membaca status kunci: {error}"))?
}

/// Saved keys for one device, as chips the panel can pick. Never carries a key.
#[tauri::command]
pub fn list_credentials(app: AppHandle, serial: String) -> Result<Vec<CredentialView>, String> {
    if !adb::is_valid_serial(&serial) {
        return Err("Serial perangkat tidak valid.".to_string());
    }
    Ok(credentials::list(&app, &serial))
}

/// Every saved key across devices, for the settings page that lets the user audit them.
#[tauri::command]
pub fn get_all_credentials(app: AppHandle) -> Vec<CredentialView> {
    credentials::all(&app)
}

#[tauri::command]
pub fn save_credential(
    app: AppHandle,
    serial: String,
    label: String,
    secret: String,
) -> Result<CredentialView, String> {
    credentials::save(&app, &serial, &label, &secret)
}

#[tauri::command]
pub fn delete_credential(app: AppHandle, id: String) -> Result<(), String> {
    credentials::delete(&app, &id)
}

#[tauri::command]
pub fn clear_credentials(app: AppHandle) -> Result<usize, String> {
    credentials::clear(&app)
}

/// Send a stored key without ever showing it to the interface: the id is looked up,
/// decrypted on the blocking pool and typed straight into the device.
#[tauri::command]
pub async fn unlock_with_credential(
    app: AppHandle,
    serial: String,
    id: String,
) -> Result<UnlockReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let secret = credentials::reveal(&app, &id, &serial)?;
        unlock::unlock(&serial, &secret)
    })
    .await
    .map_err(|error| format!("Buka kunci dengan simpanan gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn open_device_camera(
    serial: String,
    lens: Lens,
    mode: CaptureMode,
) -> Result<LocateReport, String> {
    tauri::async_runtime::spawn_blocking(move || locate::open_camera(&serial, lens, mode))
        .await
        .map_err(|error| format!("Kamera gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn close_device_camera(serial: String) -> Result<LocateReport, String> {
    tauri::async_runtime::spawn_blocking(move || locate::close_camera(&serial))
        .await
        .map_err(|error| format!("Penutupan kamera gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn push_files(
    app: AppHandle,
    serial: String,
    paths: Vec<String>,
    target_dir: Option<String>,
) -> Result<TransferResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        transfer::push_files(&app, &serial, &paths, target_dir.as_deref())
    })
    .await
    .map_err(|error| format!("Transfer gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn pull_android_file(
    app: AppHandle,
    serial: String,
    remote_path: String,
) -> Result<PullResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let directory = settings::load(&app).download_dir;
        transfer::pull_file(
            &app,
            &serial,
            &remote_path,
            std::path::Path::new(&directory),
        )
    })
    .await
    .map_err(|error| format!("Unduhan gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn list_android_files(serial: String, path: String) -> Result<Vec<AndroidEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || files::list(&serial, &path))
        .await
        .map_err(|error| format!("Gagal membaca daftar file: {error}"))?
}

#[tauri::command]
pub async fn capture_screenshot(
    app: AppHandle,
    serial: String,
) -> Result<ScreenshotResult, String> {
    tauri::async_runtime::spawn_blocking(move || screenshot::capture(&app, &serial))
        .await
        .map_err(|error| format!("Screenshot gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn send_clipboard_image(
    app: AppHandle,
    serial: String,
) -> Result<ImageSendResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        clipboard_image::send_clipboard_image(&app, &serial)
    })
    .await
    .map_err(|error| format!("Kirim gambar gagal dijalankan: {error}"))?
}

/// The user clicked a chat the panel offered. Read the phone again and finish its destination
/// list, then keep that chat as where this device sends to. Refusing to tap an unread or changed
/// list is the point: the answer comes from a fresh dump, not from the list drawn seconds ago.
#[tauri::command]
pub async fn pick_chat_target(
    app: AppHandle,
    serial: String,
    package: String,
    label: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        chat_target::resolve_by_choice(&app, &serial, &package, &label)
    })
    .await
    .map_err(|error| format!("Pilihan chat gagal dijalankan: {error}"))?
}

#[tauri::command]
pub fn get_all_chat_targets(app: AppHandle) -> Vec<TargetView> {
    chat_target::all(&app)
}

#[tauri::command]
pub fn remove_chat_target(app: AppHandle, id: String) -> Result<(), String> {
    chat_target::remove(&app, &id)
}

/// Mount-time badge: path only. `scrcpy --version` starts SDL and its DLLs, which is too
/// much work just to draw a status dot; the version stays available in Diagnostics.
#[tauri::command]
pub async fn get_scrcpy_status() -> ScrcpyStatus {
    tauri::async_runtime::spawn_blocking(session::locate)
        .await
        .unwrap_or_else(|error| ScrcpyStatus {
            found: false,
            path: None,
            version: None,
            error: Some(format!("Pemeriksaan scrcpy gagal: {error}")),
        })
}

#[tauri::command]
pub fn get_session_status(state: State<'_, SessionManager>) -> Option<SessionInfo> {
    state.info()
}

#[tauri::command]
pub async fn start_session(app: AppHandle, serial: String) -> Result<SessionInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let manager = app.state::<SessionManager>();
        manager.start(&app, &serial)
    })
    .await
    .map_err(|error| format!("Sesi gagal dijalankan: {error}"))?
}

#[tauri::command]
pub async fn stop_session(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let manager = app.state::<SessionManager>();
        manager.stop(&app)
    })
    .await
    .map_err(|error| format!("Sesi gagal dihentikan: {error}"))?
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    settings::load(&app)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, mut settings: Settings) -> Result<Settings, String> {
    // The Settings page form never carries the theme (its toggle lives in the sidebar and
    // saves itself), so a settings write must not be allowed to reset it to the serde default.
    settings.theme = settings::load(&app).theme;
    crate::settings::save(&app, settings)
}

#[derive(Clone, Copy, Debug)]
enum Theme {
    Dark,
    Light,
}

impl Theme {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

#[tauri::command]
pub fn get_theme(app: AppHandle) -> String {
    settings::load(&app).theme
}

#[tauri::command]
pub fn set_theme(app: AppHandle, theme: String) -> Result<String, String> {
    let Some(parsed) = Theme::parse(&theme) else {
        return Err("Tema hanya mendukung 'dark' atau 'light'.".to_string());
    };

    let mut settings = settings::load(&app);
    settings.theme = parsed.as_str().to_string();
    crate::settings::save(&app, settings)?;

    // The native frame (title bar on standalone fallbacks, tray menus) should follow the
    // page; a failure here is cosmetic, the persisted theme is what the frontend reads.
    if let Some(window) = app.get_webview_window("main") {
        let native = match parsed {
            Theme::Dark => tauri::Theme::Dark,
            Theme::Light => tauri::Theme::Light,
        };
        let _ = window.set_theme(Some(native));
    }

    Ok(parsed.as_str().to_string())
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Hanya URL http/https yang boleh dibuka.".to_string());
    }
    if !url.chars().all(|c| !c.is_control() && c != '"') {
        return Err("URL mengandung karakter tidak valid.".to_string());
    }

    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";

    std::process::Command::new(program)
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Gagal membuka browser: {error}"))
}

#[tauri::command]
pub async fn get_diagnostics(app: AppHandle, deep: bool) -> Diagnostics {
    let worker = app.clone();
    let (adb_status, scrcpy_status, current) = tauri::async_runtime::spawn_blocking(move || {
        // Opening the page only needs to know where the tools are. Asking each of them for
        // its version runs a process — `scrcpy --version` loads SDL and the whole video
        // stack — so the versions are read only when the user asks (`deep`).
        let adb_status = if deep { adb::status() } else { adb::locate() };
        let scrcpy_status = if deep {
            session::status()
        } else {
            session::locate()
        };
        (adb_status, scrcpy_status, settings::load(&worker))
    })
    .await
    .unwrap_or_else(|_| {
        (
            adb::locate(),
            session::locate(),
            settings::load(&app),
        )
    });

    Diagnostics {
        adb: adb_status,
        scrcpy: scrcpy_status,
        config_path: settings::config_path(&app).map(|path| path.display().to_string()),
        download_dir: current.download_dir,
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
pub fn open_download_dir(app: AppHandle) -> Result<(), String> {
    let directory = settings::load(&app).download_dir;
    if !std::path::Path::new(&directory).is_dir() {
        return Err(format!("Folder unduhan tidak ditemukan: {directory}"));
    }

    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";

    std::process::Command::new(program)
        .arg(&directory)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Gagal membuka folder unduhan: {error}"))
}

#[cfg(test)]
mod window_tests {
    use super::fit_in_work_area;

    #[test]
    fn a_left_anchored_window_grows_to_the_right() {
        assert_eq!(fit_in_work_area(0, 1200, 0, 1920), 0);
    }

    #[test]
    fn a_window_near_the_right_edge_slides_left() {
        assert_eq!(fit_in_work_area(1000, 1200, 0, 1920), 720);
    }

    #[test]
    fn a_window_wider_than_the_work_area_sticks_to_the_left() {
        assert_eq!(fit_in_work_area(500, 2400, 0, 1920), 0);
    }

    #[test]
    fn the_shift_never_crosses_the_left_edge() {
        assert_eq!(fit_in_work_area(-50, 1200, 0, 1920), 0);
    }
}
