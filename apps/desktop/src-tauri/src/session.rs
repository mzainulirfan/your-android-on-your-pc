use crate::adb;
use serde::Serialize;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

// Frame budget for the latency-first preset, measured while scrolling the test phone:
// 1920 px · 60 FPS moved 12.6 MB in 22 s at 12 ms average ping, 1080 px · 30 FPS · 2 Mbps
// moved 2.9 MB at 9 ms. The cheaper setting bought no measurable delay improvement, but a
// 120 Hz panel rendered by SystemUI at ~34 fps loses nothing at 60 and gains nothing at 30 —
// it only looks choppy. So the preset cuts the pixels the phone has to encode (the part that
// competes with the animation) and leaves the frame rate at 60.
const LOW_LATENCY_MAX_SIZE: u32 = 1080;
const LOW_LATENCY_MAX_FPS: u32 = 60;
const LOW_LATENCY_BITRATE_KBPS: u32 = 4_000;

// Measured on the live Wi-Fi link (Redmi on 2.4 GHz, 72 Mbps association), scrolling the
// phone while capturing: no cap moved 16 MB in 22 s and pushed the ping to the phone to
// 26 ms average with 156 ms spikes; 1920 px moved 12.6 MB at 12 ms/106 ms and 1080 px moved
// 2.9 MB at 9 ms/103 ms. The floor therefore only removes the pathological "mirror the
// whole panel" case and leaves the aggressive setting to the latency preset.
const WIFI_AUTO_MAX_SIZE: u32 = 1920;

/// The preset only ever lowers the quality the user configured — 0 means "device default",
/// which is the larger of the two here.
fn cap_quality(value: u32, limit: u32) -> u32 {
    if value == 0 || value > limit {
        limit
    } else {
        value
    }
}

/// 0 means "mirror whatever the panel produces". That is fine over USB and ruinous over a
/// shared 2.4 GHz channel, so a wireless session always gets a cap.
fn effective_max_size(max_size: u32, connection_type: &str) -> u32 {
    if max_size == 0 && connection_type != "usb" {
        WIFI_AUTO_MAX_SIZE
    } else {
        max_size
    }
}

/// Turning the screen off parks the phone's Wi-Fi radio in power save: the same link that
/// answered in 87 ms with the screen on answered in 146 ms on average, with 318 ms spikes,
/// once the screen was off. A dark phone is worth less than a stuttering mirror, so the
/// preference is honoured over USB only.
fn should_turn_screen_off(
    mirror_screen_off: bool,
    keyguard_up: bool,
    connection_type: &str,
) -> bool {
    mirror_screen_off && !keyguard_up && connection_type == "usb"
}

/// scrcpy captures the phone's playback through the Android 13 AudioPlaybackCapture API.
/// Below API 33 the only other source is the microphone — which is not "the sound coming
/// from the phone" — so older devices keep mirroring video only. A device whose API level
/// cannot be read is treated the same way: silence rather than a failed session.
fn wants_audio(enabled: bool, sdk: Option<u32>) -> bool {
    enabled && sdk.is_some_and(|version| version >= 33)
}

pub struct SessionManager {
    current: Mutex<Option<Session>>,
}

struct Session {
    id: u64,
    serial: String,
    child: Child,
    started_at: u64,
    locked: bool,
    screen_on: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub serial: String,
    pub started_at: u64,
    pub pid: u32,
    pub locked: bool,
    /// The display state the session starts with, read by the same prep that woke the
    /// phone. The UI can label its screen toggle immediately instead of asking the phone
    /// for a dump while scrcpy is still starting.
    pub screen_on: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExit {
    pub serial: String,
    pub code: Option<i32>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionError {
    pub serial: String,
    pub code: Option<i32>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrcpyStatus {
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub error: Option<String>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }
}

impl SessionManager {
    pub fn info(&self) -> Option<SessionInfo> {
        let guard = self.current.lock().ok()?;
        guard.as_ref().map(|session| SessionInfo {
            serial: session.serial.clone(),
            started_at: session.started_at,
            pid: session.child.id(),
            locked: session.locked,
            screen_on: session.screen_on,
        })
    }

    pub fn start(&self, app: &AppHandle, serial: &str) -> Result<SessionInfo, String> {
        if !adb::is_valid_serial(serial) {
            return Err("Serial perangkat tidak valid.".to_string());
        }

        // Refuse a second session before any phone work: the prep below costs a key event
        // and a lock-screen wait whenever the device is not already sitting unlocked.
        {
            let mut guard = self
                .current
                .lock()
                .map_err(|_| "State sesi tidak dapat diakses.".to_string())?;
            if let Some(existing) = guard.as_mut() {
                let running = existing
                    .child
                    .try_wait()
                    .map(|status| status.is_none())
                    .unwrap_or(false);
                if running {
                    return Err(format!(
                        "Sesi untuk {} masih berjalan. Hentikan sesi tersebut terlebih dahulu.",
                        existing.serial
                    ));
                }
                *guard = None;
            }
        }

        let devices = adb::list_devices()?;
        let device = devices
            .iter()
            .find(|device| device.serial == serial)
            .ok_or_else(|| {
                "Perangkat tidak ditemukan pada ADB. Muat ulang daftar perangkat.".to_string()
            })?;
        if device.state != "device" {
            return Err(format!(
                "Perangkat belum siap ({}). Selesaikan otorisasi di layar HP.",
                device.state
            ));
        }

        let scrcpy = scrcpy_path()?;
        let adb = adb::adb_path()?;
        let settings = crate::settings::load(app);

        // Bring the display up so the session has something to mirror; a locked device
        // would otherwise show a black window until the user wakes it by hand. The prep
        // reads first and writes only when needed: awake + unlocked is one probe, while a
        // sleeping or locked phone gets wake, dismiss and a settled verdict in one shell
        // session (a stale dump right after `dismiss-keyguard` used to keep the dock open
        // over an unlocked phone and skip the screen-off the user asked for).
        let (mut screen_on, keyguard_up) = match adb::prepare_session(serial) {
            Ok(state) => (state.screen_on, state.keyguard_up),
            // The probes failed but scrcpy may still reach the phone; it will report the
            // real error instead of the session silently not starting.
            Err(_) => (true, false),
        };

        let mut guard = self
            .current
            .lock()
            .map_err(|_| "State sesi tidak dapat diakses.".to_string())?;

        let mut stale = false;
        if let Some(existing) = guard.as_mut() {
            match existing.child.try_wait() {
                Ok(Some(_)) | Err(_) => stale = true,
                Ok(None) => {
                    return Err(format!(
                        "Sesi untuk {} masih berjalan. Hentikan sesi tersebut terlebih dahulu.",
                        existing.serial
                    ))
                }
            }
        }
        if stale {
            *guard = None;
        }

        let mut command = Command::new(&scrcpy);
        command
            .arg("-s")
            .arg(serial)
            .arg("--push-target=/sdcard/Download/")
            .arg(format!("--window-title=YAOP - {serial}"))
            .env("ADB", &adb);
        // One small shell read on the path to spawning scrcpy, in exchange for not opening
        // an audio stream the phone cannot provide (a failed capture takes the session with
        // it) and not silently ignoring the user's audio preference on Android 13+.
        if wants_audio(settings.audio_enabled, adb::sdk_version(serial)) {
            command.arg("--audio-source=output");
        } else {
            command.arg("--no-audio");
        }
        // The dock is drawn flush against the top edge of the video and is as wide as it
        // is, which only looks like one window if the video has no title bar of its own.
        // The panel carries what the decoration used to do: drag, zoom, close.
        #[cfg(windows)]
        command.arg("--window-borderless");
        // "uhid" simulates a physical HID keyboard, so keystrokes are delivered by the
        // device's own input stack. Android 14+ rejects synthetic injection (`sdk`) into
        // secure fields such as the keyguard PIN box, and a real HID accessory is not
        // synthetic — typing into the lock screen only works through uhid. Only the "aoa"
        // mode is limited to USB; uhid was verified over a TCP connection, where the device
        // registers an input device named "scrcpy" and removes it when the session ends.
        command.arg("--keyboard=uhid");
        // The panel unlock used to be undone a few seconds later: the phone's screen-off
        // timer fired (the test device reported 6 s at power-off, right after
        // `KeyguardGoingAway`) and, with lock-after-timeout disabled, the secure keyguard
        // re-armed. ADB-injected key events do not hold that timer down, so the session
        // itself keeps the display alive: scrcpy pokes `PowerManager.userActivity` every
        // 4 s. The poke does not power a display back on, so "Matikan layar" still sleeps
        // it, and unlike --stay-awake this also works over TCP (it is a control message,
        // not the global `stay_on_while_plugged_in` setting).
        command.arg("--keep-active");
        // Mouse always uses scrcpy's default touch-injection mode (both USB and Wi-Fi): the
        // PC pointer stays a normal, visible cursor and moving/clicking over the video injects
        // absolute taps. The alternative, `--mouse=uhid`, grabs the pointer in relative mode and
        // hides it — which only works when scrcpy owns a top-level frame. Embedded as a
        // cross-process WS_CHILD, that confinement breaks: the cursor freezes as a black blob
        // and no click ever lands. Keyboard still uses uhid (see above) because typing into
        // the keyguard PIN box needs a real HID device, not synthetic injection.
        let (max_size, bitrate, max_fps) = if settings.low_latency {
            (
                cap_quality(settings.max_size, LOW_LATENCY_MAX_SIZE),
                cap_quality(settings.video_bitrate_kbps, LOW_LATENCY_BITRATE_KBPS),
                cap_quality(settings.max_fps, LOW_LATENCY_MAX_FPS),
            )
        } else {
            (
                settings.max_size,
                settings.video_bitrate_kbps,
                settings.max_fps,
            )
        };
        let max_size = effective_max_size(max_size, &device.connection_type);

        if max_size > 0 {
            command.arg(format!("--max-size={max_size}"));
        }
        if bitrate > 0 {
            command.arg(format!("--video-bit-rate={}", bitrate.saturating_mul(1000)));
        }
        if max_fps > 0 {
            // A 90–120 Hz panel fed an uncapped capture spreads the bitrate over twice as
            // many frames, so every frame gets fewer bits and the picture reads as stutter.
            // Unsupported below Android 10, which is why 0 leaves it to scrcpy.
            command.arg(format!("--max-fps={max_fps}"));
        }
        if should_turn_screen_off(
            settings.mirror_screen_off,
            keyguard_up,
            &device.connection_type,
        ) {
            // Inputs must not wake the device screen back up.
            command.arg("--turn-screen-off").arg("--no-power-on");
            screen_on = false;
        }
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        let child = command
            .spawn()
            .map_err(|error| format!("Gagal menjalankan scrcpy ({}): {error}", scrcpy.display()))?;

        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let started_at = now_ms();
        let info = SessionInfo {
            serial: serial.to_string(),
            started_at,
            pid: child.id(),
            locked: keyguard_up,
            screen_on,
        };

        *guard = Some(Session {
            id,
            serial: serial.to_string(),
            child,
            started_at,
            locked: keyguard_up,
            screen_on,
        });
        drop(guard);

        let _ = app.emit("session-started", &info);
        spawn_monitor(app.clone(), id);

        // The page keeps the mirror box empty until the user is on the mirroring view;
        // `host::begin` waits for scrcpy's window and folds it into the app window once the
        // size settles. `keyguard_up` only decides who gets the keyboard — the credential
        // field in the page for a locked phone, the video otherwise.
        crate::host::begin(app, serial, keyguard_up);
        crate::mirror::start(app);

        Ok(info)
    }

    pub fn stop(&self, app: &AppHandle) -> Result<(), String> {
        let mut guard = self
            .current
            .lock()
            .map_err(|_| "State sesi tidak dapat diakses.".to_string())?;

        let Some(mut session) = guard.take() else {
            return Err("Tidak ada sesi aktif.".to_string());
        };
        drop(guard);

        let _ = session.child.kill();
        let code = session.child.wait().ok().and_then(|status| status.code());

        crate::mirror::stop(app);
        crate::host::end();

        let _ = app.emit(
            "session-exited",
            SessionExit {
                serial: session.serial,
                code,
                reason: "stopped".to_string(),
            },
        );

        Ok(())
    }

    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.current.lock() {
            if let Some(mut session) = guard.take() {
                let _ = session.child.kill();
                let _ = session.child.wait();
            }
        }
    }
}

fn spawn_monitor(app: AppHandle, id: u64) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));

        let manager = app.state::<SessionManager>();
        let exited = {
            let Ok(mut guard) = manager.current.lock() else {
                return;
            };

            let outcome = {
                let Some(session) = guard.as_mut() else {
                    return;
                };
                if session.id != id {
                    return;
                }
                match session.child.try_wait() {
                    Ok(Some(status)) => Some((session.serial.clone(), status.code())),
                    Ok(None) => None,
                    Err(_) => Some((session.serial.clone(), None)),
                }
            };

            if outcome.is_some() {
                guard.take();
            }
            outcome
        };

        if let Some((serial, code)) = exited {
            crate::mirror::stop(&app);
            crate::host::end();
            if code != Some(0) {
                let _ = app.emit(
                    "session-error",
                    SessionError {
                        serial: serial.clone(),
                        code,
                        message: match code {
                            Some(value) => format!("scrcpy berhenti dengan kode {value}."),
                            None => "scrcpy berhenti tanpa kode keluar.".to_string(),
                        },
                    },
                );
            }

            let _ = app.emit(
                "session-exited",
                SessionExit {
                    serial,
                    code,
                    reason: "exited".to_string(),
                },
            );
            return;
        }
    });
}

/// Whether scrcpy can be started, without running it. `--version` loads SDL and the video
/// stack, which is far more than a status badge is worth at window startup.
pub fn locate() -> ScrcpyStatus {
    match scrcpy_path() {
        Ok(path) => ScrcpyStatus {
            found: true,
            path: Some(path.display().to_string()),
            version: None,
            error: None,
        },
        Err(error) => ScrcpyStatus {
            found: false,
            path: None,
            version: None,
            error: Some(error),
        },
    }
}

pub fn status() -> ScrcpyStatus {
    let path = match scrcpy_path() {
        Ok(path) => path,
        Err(error) => {
            return ScrcpyStatus {
                found: false,
                path: None,
                version: None,
                error: Some(error),
            }
        }
    };

    let display_path = path.display().to_string();
    match version_of(&path) {
        Ok(version) => ScrcpyStatus {
            found: true,
            path: Some(display_path),
            version: Some(version),
            error: None,
        },
        Err(error) => ScrcpyStatus {
            found: true,
            path: Some(display_path),
            version: None,
            error: Some(error),
        },
    }
}

fn version_of(scrcpy: &PathBuf) -> Result<String, String> {
    let mut command = Command::new(scrcpy);
    command.arg("--version");
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = command
        .output()
        .map_err(|error| format!("Gagal menjalankan scrcpy: {error}"))?;

    if !output.status.success() {
        return Err(format!("scrcpy keluar dengan status {}", output.status));
    }

    output
        .stdout
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).trim().to_string())
        .filter(|line| !line.is_empty())
        .ok_or_else(|| "Versi scrcpy tidak terbaca.".to_string())
}

pub fn scrcpy_path() -> Result<PathBuf, String> {
    let binary = if cfg!(windows) {
        "scrcpy.exe"
    } else {
        "scrcpy"
    };

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(explicit) = std::env::var_os("YAOP_SCRCPY_PATH") {
        candidates.push(PathBuf::from(explicit));
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("binaries").join(binary));
            candidates.push(directory.join(binary));
        }
    }

    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("binaries")
            .join("scrcpy")
            .join(binary),
    );

    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    if adb::on_path(binary) {
        return Ok(PathBuf::from(binary));
    }

    Err(format!(
        "scrcpy tidak ditemukan. Letakkan {binary} beserta DLL-nya di binaries/scrcpy/, \
         set YAOP_SCRCPY_PATH, atau tambahkan scrcpy ke PATH."
    ))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_audio_needs_the_android_13_api() {
        assert!(wants_audio(true, Some(33)));
        assert!(wants_audio(true, Some(36)));
        // Below API 33 there is no playback capture, only the microphone.
        assert!(!wants_audio(true, Some(32)));
        // An unreadable API level must not open a stream that can fail the session.
        assert!(!wants_audio(true, None));
        // The user's preference always wins over a capable device.
        assert!(!wants_audio(false, Some(36)));
    }

    #[test]
    fn the_latency_preset_only_ever_lowers_the_frame_budget() {
        // Device default (0) and a larger user value both fall back to the cap.
        assert_eq!(cap_quality(0, LOW_LATENCY_MAX_FPS), LOW_LATENCY_MAX_FPS);
        assert_eq!(cap_quality(120, LOW_LATENCY_MAX_FPS), LOW_LATENCY_MAX_FPS);
        // A stricter user value is respected.
        assert_eq!(cap_quality(24, LOW_LATENCY_MAX_FPS), 24);
    }

    #[test]
    fn a_wireless_session_is_never_left_uncapped() {
        // 0 means the whole panel, which measured 16 MB per 22 s and 156 ms ping spikes.
        assert_eq!(effective_max_size(0, "wifi"), WIFI_AUTO_MAX_SIZE);
        assert_eq!(effective_max_size(0, "usb"), 0);
        // Anything the user picked deliberately is left alone, stricter values included.
        assert_eq!(effective_max_size(720, "wifi"), 720);
        assert_eq!(effective_max_size(4096, "wifi"), 4096);
    }

    #[test]
    fn the_screen_stays_on_when_mirroring_over_the_network() {
        // A phone with its screen off parks the Wi-Fi radio in power save: 146 ms average
        // and 318 ms spikes against 87 ms with the screen on, on the same link.
        assert!(!should_turn_screen_off(true, false, "wifi"));
        assert!(should_turn_screen_off(true, false, "usb"));
        // Over any transport, the lock screen is never switched off — it has to stay visible
        // so it can be unlocked from the PC.
        assert!(!should_turn_screen_off(true, true, "usb"));
        assert!(!should_turn_screen_off(false, false, "usb"));
    }
}
