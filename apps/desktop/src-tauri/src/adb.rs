use serde::Serialize;
use std::io::Read;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidDevice {
    pub serial: String,
    pub state: String,
    pub connection_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_codename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WirelessInfo {
    pub usb_serial: String,
    pub wifi_serial: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbStatus {
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const WIRELESS_PORT: u16 = 5555;

fn adb_binary() -> &'static str {
    if cfg!(windows) {
        "adb.exe"
    } else {
        "adb"
    }
}

/// The resolved binary does not change while the app runs, so it is remembered instead of
/// stat'ing ~10 candidate paths on every ADB call. An explicit `YAOP_ADB_PATH` still wins on
/// every call, so a development or test override is never shadowed by the cache.
static ADB_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

pub fn adb_path() -> Result<PathBuf, String> {
    if let Some(explicit) = std::env::var_os("YAOP_ADB_PATH") {
        let candidate = PathBuf::from(explicit);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    if let Ok(guard) = ADB_PATH.lock() {
        if let Some(cached) = guard.as_ref().filter(|path| path.is_file()) {
            return Ok(cached.clone());
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = std::env::var_os(variable) {
            candidates.push(
                PathBuf::from(root)
                    .join("platform-tools")
                    .join(adb_binary()),
            );
        }
    }

    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join(adb_binary()),
        );
    }

    candidates.extend(side_by_side_candidates(adb_binary()));

    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("binaries")
            .join("adb")
            .join(adb_binary()),
    );

    if let Some(found) = candidates.into_iter().find(|candidate| candidate.is_file()) {
        if let Ok(mut guard) = ADB_PATH.lock() {
            *guard = Some(found.clone());
        }
        return Ok(found);
    }

    if on_path(adb_binary()) {
        let found = PathBuf::from(adb_binary());
        if let Ok(mut guard) = ADB_PATH.lock() {
            *guard = Some(found.clone());
        }
        return Ok(found);
    }

    Err(format!(
        "ADB tidak ditemukan. Pasang Android Platform Tools lalu set ANDROID_HOME, \
         letakkan {} di binaries/adb/, atau tambahkan adb ke PATH.",
        adb_binary()
    ))
}

pub(crate) fn on_path(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| directory.join(binary).is_file())
}

/// Binaries shipped next to the running executable. `CARGO_MANIFEST_DIR` is baked in at
/// compile time, so an installed build would otherwise only find ADB while the developer's
/// source tree still happens to sit at that exact path on the user's machine.
pub(crate) fn side_by_side_candidates(binary: &str) -> Vec<PathBuf> {
    let Some(directory) = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|parent| parent.to_path_buf()))
    else {
        return Vec::new();
    };

    vec![
        directory.join("binaries").join(binary),
        directory.join(binary),
    ]
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
// A sleeping phone answers TCP slowly, but a host that is gone takes the full Windows
// connect timeout (~21s), so an explicit attempt gets longer than a background retry.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(4);

fn run(adb: &Path, args: &[&str]) -> Result<String, String> {
    run_with_timeout(adb, args, DEFAULT_TIMEOUT)
}

/// ADB blocks on its own schedule: a half-dead device or a server that is being restarted
/// can leave a call hanging indefinitely, and the caller would never report anything.
/// Reading the pipes on a helper thread lets the caller give up and surface an error.
fn run_with_timeout(adb: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(adb);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Gagal menjalankan ADB ({}): {error}", adb.display()))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (sender, receiver) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(mut stream) = stdout {
            let _ = stream.read_to_end(&mut out);
        }
        if let Some(mut stream) = stderr {
            let _ = stream.read_to_end(&mut err);
        }
        let _ = sender.send((out, err));
    });

    let (stdout, stderr) = match receiver.recv_timeout(timeout) {
        Ok(pipes) => pipes,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "ADB tidak merespons dalam {} detik. Proses adb.exe mungkin macet; \
                 coba `adb kill-server` lalu muat ulang daftar perangkat.",
                timeout.as_secs()
            ));
        }
    };

    let status = child
        .wait()
        .map_err(|error| format!("Gagal menunggu ADB: {error}"))?;

    if !status.success() {
        let message = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(if message.is_empty() {
            format!("ADB keluar dengan status {status}")
        } else {
            message
        });
    }

    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

pub fn list_devices() -> Result<Vec<AndroidDevice>, String> {
    let adb = adb_path()?;
    let output = run(&adb, &["devices", "-l"])?;
    Ok(parse_devices(&output))
}

/// Put the device screen to sleep or wake it without touching scrcpy's session, and report
/// the state the display ended up in.
///
/// scrcpy keeps a wake lock while mirroring, so the display stays powered and the
/// mirrored window remains usable even when the physical screen is off. Waking runs one
/// shell session: the key event, the waits, the dismiss and both verdicts happen on the
/// phone, so the toggle costs one ADB spawn and the UI can label the result without a
/// second read. A sleep needs no read: an off screen always has its keyguard up.
pub fn set_screen_power(serial: &str, sleep: bool) -> Result<WakeState, String> {
    if sleep {
        shell(serial, &["input", "keyevent", "KEYCODE_SLEEP"])?;
        return Ok(WakeState {
            screen_on: false,
            keyguard_up: true,
        });
    }
    let text = shell(serial, &[WAKE_UP_PROBE])?;
    wake_state_from(serial, &text)
}

pub fn shell(serial: &str, args: &[&str]) -> Result<String, String> {
    if !is_valid_serial(serial) {
        return Err("Serial perangkat tidak valid.".to_string());
    }

    let adb = adb_path()?;
    let mut command_args = vec!["-s", serial, "shell"];
    command_args.extend_from_slice(args);
    run(&adb, &command_args)
}

/// The device's Android API level, or `None` when it cannot be read. scrcpy captures the
/// phone's playback audio through the AudioPlaybackCapture API, which only exists from
/// Android 13 (API 33); the caller decides what that number means before asking scrcpy
/// to open an audio stream.
pub fn sdk_version(serial: &str) -> Option<u32> {
    let text = shell(serial, &["getprop", "ro.build.version.sdk"]).ok()?;
    text.trim().parse().ok()
}

/// Probes run on the phone, so the pipe filters there and only the matched lines cross the
/// adb link. `dumpsys power` alone is a ~600 KB dump that took 0.48 s on the test device, and
/// scrcpy video shares the same adbd connection: polling that during a mirror session is what
/// made the picture hiccup. Each script is a compile-time constant with no caller data in it,
/// and the serial still travels as its own validated argument.
const POWER_STATE_PROBE: &str = "dumpsys power | grep -m1 -E 'mWakefulness=|mScreenOn='";
pub const KEYGUARD_BLOCK_PROBE: &str = "dumpsys window | grep -m1 -A40 KeyguardServiceDelegate";
/// Wake and open the lock-screen bouncer, waiting on the phone between the steps. Shared by
/// the wake toggle and the unlock script so a credential is typed immediately after the wake
/// instead of after several host round trips — the screen-off timer keeps running during
/// those, and the lock screen used to time out before the first digit landed.
pub const WAKE_UP_COMMAND: &str =
    "input keyevent KEYCODE_WAKEUP; sleep 0.6; wm dismiss-keyguard; sleep 0.4";
/// Both verdicts in one shell session. `dumpsys` is served by system_server — the same
/// process that schedules the animations the mirror is showing — and on the test phone one
/// read cost 77–189 ms and the other 99–484 ms. Asking for them concurrently therefore stole
/// CPU from the very frames the user was watching, so the status poll asks for both in a
/// single sequential session instead.
const WAKE_STATE_PROBE: &str = concat!(
    "dumpsys power | grep -m1 -E 'mWakefulness=|mScreenOn='; ",
    "dumpsys window | grep -m1 -A40 KeyguardServiceDelegate"
);
/// Wake, dismiss and settle in one shell session, then read both verdicts from that same
/// answer. The sleeps run on the phone between the steps, so waking costs one ADB spawn
/// instead of a key event, a host-side wait, a dismiss and two probes.
const WAKE_UP_PROBE: &str = concat!(
    "input keyevent KEYCODE_WAKEUP; ",
    "sleep 0.6; ",
    "wm dismiss-keyguard; ",
    "sleep 0.4; ",
    "dumpsys power | grep -m1 -E 'mWakefulness=|mScreenOn='; ",
    "dumpsys window | grep -m1 -A40 KeyguardServiceDelegate"
);

/// Screen and lock-screen verdicts read from one device response.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeState {
    pub screen_on: bool,
    pub keyguard_up: bool,
}

/// `None` when the answer carries no keyguard flag, which the caller must not read as
/// "unlocked".
fn wake_state_reported(text: &str) -> Option<(bool, bool)> {
    // Each verdict is read only from its own service's words: the keyguard block also prints
    // screen flags, and scanning the whole answer would let one service answer for the other.
    const MARKER: &str = "KeyguardServiceDelegate";
    let (power, window) = match text.find(MARKER) {
        Some(at) => (&text[..at], &text[at..]),
        None => (text, ""),
    };
    let showing = read_flag(window, "showing").or_else(|| read_flag(window, "mIsShowing"))?;
    Some((screen_on_reported(power), showing))
}

/// Both verdicts from one answer; a missing keyguard flag falls back to a dedicated read.
fn wake_state_from(serial: &str, text: &str) -> Result<WakeState, String> {
    if let Some((screen_on, keyguard_up)) = wake_state_reported(text) {
        return Ok(WakeState {
            screen_on,
            keyguard_up,
        });
    }
    // No keyguard flag. An asleep or dozing panel has its keyguard up whatever the dump says,
    // so only a screen that is genuinely on needs the dedicated keyguard read.
    let screen_on = screen_on_reported(text);
    let keyguard_up = !screen_on || keyguard_probe(serial)?;
    Ok(WakeState {
        screen_on,
        keyguard_up,
    })
}

pub fn wake_state(serial: &str) -> Result<WakeState, String> {
    let text = shell(serial, &[WAKE_STATE_PROBE])?;
    wake_state_from(serial, &text)
}

/// Put the phone in a state worth mirroring and report what that state is, before scrcpy
/// starts. A phone that is already awake and unlocked is answered by a single read: the
/// key event, the 600 ms wait and the dismiss would all be no-ops that delay the session by
/// more than a second. Only a sleeping or locked phone pays for the write path, where the
/// waits run on the device and the settled verdict arrives in the same answer.
pub fn prepare_session(serial: &str) -> Result<WakeState, String> {
    let quick = wake_state(serial)?;
    if quick.screen_on && !quick.keyguard_up {
        return Ok(quick);
    }

    let text = shell(serial, &[WAKE_UP_PROBE])?;
    wake_state_from(serial, &text)
}

/// Keyguard verdict read from a `dumpsys window` answer, or `None` when it carries no flag.
pub fn keyguard_flag(text: &str) -> Option<bool> {
    read_flag(text, "showing").or_else(|| read_flag(text, "mIsShowing"))
}

/// The keyguard verdict, or the ADB failure behind it. A verdict of `false` is only honest
/// when the phone actually answered: reporting "unlocked" for a device we could not read
/// would let the UI claim a phone it cannot reach is sitting open.
pub fn keyguard_probe(serial: &str) -> Result<bool, String> {
    let block = shell(serial, &[KEYGUARD_BLOCK_PROBE]);
    if let Ok(text) = block.as_deref() {
        if let Some(showing) = keyguard_flag(text) {
            return Ok(showing);
        }
    }

    // No `KeyguardServiceDelegate` block, or the probe stopped before its flags: read the
    // whole dump and use the documented fallback order.
    match shell(serial, &["dumpsys", "window"]) {
        Ok(dump) => Ok(keyguard_state(&dump).unwrap_or(false)),
        Err(fallback_error) => Err(block.err().unwrap_or(fallback_error)),
    }
}

/// True when the keyguard (lock screen) is on top. Used so a mirroring session does
/// not force the physical display off while the user still needs to see and enter the
/// PIN/pattern.
pub fn is_keyguard_showing(serial: &str) -> bool {
    keyguard_probe(serial).unwrap_or(false)
}

/// The live keyguard state comes from the `KeyguardServiceDelegate` block that
/// PhoneWindowManager prints, which holds one copy of the authoritative flag.
/// Where it is absent, the legacy `DisplayPolicy` field names are used.
fn keyguard_state(dump: &str) -> Option<bool> {
    if let Some(block) = keyguard_delegate_block(dump) {
        if let Some(showing) =
            read_flag(block, "showing").or_else(|| read_flag(block, "mIsShowing"))
        {
            return Some(showing);
        }
    }
    read_flag(dump, "isKeyguardShowing").or_else(|| read_flag(dump, "mShowingLockscreen"))
}

#[cfg(test)]
fn keyguard_reported(dump: &str) -> bool {
    keyguard_state(dump).unwrap_or(false)
}

/// The `DisplayPolicy` blocks cannot be scanned as text: a device reports one per
/// display, and the observed HyperOS dump holds an awake copy beside a display that
/// still says `mAwake=false`. It also keeps `mDreamingLockscreen=true` while
/// `mShowingDream=false` and the panel is awake, so that field says nothing about a
/// visible lock screen — both are only consulted as an ordered, first-match read.
fn keyguard_delegate_block(dump: &str) -> Option<&str> {
    const MARKER: &str = "KeyguardServiceDelegate";
    let rest = dump.get(dump.find(MARKER)? + MARKER.len()..)?;
    Some(match rest.find("Looper state:") {
        Some(end) => rest.get(..end)?,
        None => rest,
    })
}

/// Value of the first whole-word `key=true` / `key=false` assignment, or None when the
/// key never carries a boolean.
fn read_flag(source: &str, key: &str) -> Option<bool> {
    let mut offset = 0;
    while let Some(found) = source.get(offset..)?.find(key) {
        let start = offset + found;
        let end = start + key.len();
        let is_own_word = start == 0
            || !source[..start]
                .as_bytes()
                .last()
                .is_some_and(|before| before.is_ascii_alphanumeric());
        if let Some(value) = source.get(end..).filter(|_| is_own_word) {
            if let Some(value) = value.strip_prefix('=').map(|rest| rest.trim_start()) {
                if value.starts_with("true") {
                    return Some(true);
                }
                if value.starts_with("false") {
                    return Some(false);
                }
            }
        }
        offset = end;
    }
    None
}

/// True when the display is powered on. OEMs label the power state differently, so a
/// few known spellings are checked; used by the single wake/sleep toggle to show the
/// current state.
pub fn screen_probe(serial: &str) -> Result<bool, String> {
    if let Ok(filtered) = shell(serial, &[POWER_STATE_PROBE]) {
        if !filtered.trim().is_empty() {
            return Ok(screen_on_reported(&filtered));
        }
    }
    // Either this OEM prints neither field or the filter was unavailable: read the whole
    // dump rather than guessing from a missing line.
    shell(serial, &["dumpsys", "power"]).map(|dump| screen_on_reported(&dump))
}

fn screen_on_reported(dump: &str) -> bool {
    if [
        "mScreenOn=false",
        "state=OFF",
        "mWakefulness=Asleep",
        "mWakefulness=Dozing",
    ]
    .iter()
    .any(|needle| dump.contains(needle))
    {
        return false;
    }
    [
        "mScreenOn=true",
        "Display Power: state=ON",
        "mWakefulness=Awake",
    ]
    .iter()
    .any(|needle| dump.contains(needle))
}

/// Cheap guard for call sites that do not pre-flight `adb devices -l`: the serial must be
/// safe to hand to adb as its own argument. Nothing is spawned — the operation itself
/// reports when the phone is gone, and [`explain_device_error`] turns that into a sentence
/// the user can act on.
pub fn validate_serial(serial: &str) -> Result<(), String> {
    if is_valid_serial(serial) {
        Ok(())
    } else {
        Err("Serial perangkat tidak valid.".to_string())
    }
}

/// Rewrite the failures that mean "the phone is not on the other end" into the actionable
/// sentence the old `adb devices` pre-flight used to produce from a second round trip.
/// Anything else is passed through unchanged.
pub fn explain_device_error(serial: &str, message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    // `adb` prints `adb: device '<serial>' not found`, so the serial sits between the two
    // words that identify this failure.
    if lower.contains("no devices/emulators found")
        || lower.contains("device offline")
        || lower.contains("device still connecting")
        || (lower.contains("device") && lower.contains("not found"))
    {
        return format!(
            "Perangkat {serial} tidak tersambung ke ADB. Periksa kabel atau koneksi Wi-Fi, \
             lalu muat ulang daftar perangkat."
        );
    }
    if lower.contains("unauthorized") {
        return format!(
            "ADB ditolak oleh {serial}. Terima dialog \"Izinkan USB debugging\" di HP, \
             lalu coba lagi."
        );
    }
    message.to_string()
}

pub fn enable_wireless(serial: &str) -> Result<WirelessInfo, String> {
    if !is_valid_serial(serial) {
        return Err("Serial perangkat tidak valid.".to_string());
    }

    let adb = adb_path()?;
    let devices = list_devices()?;
    let device = devices
        .iter()
        .find(|device| device.serial == serial)
        .ok_or_else(|| {
            "Perangkat tidak ditemukan pada ADB. Sambungkan kabel dan muat ulang daftar."
                .to_string()
        })?;

    if device.connection_type != "usb" {
        return Err("Perangkat ini sudah terhubung melalui Wi-Fi.".to_string());
    }
    if device.state != "device" {
        return Err(format!(
            "Perangkat belum siap ({}). Selesaikan otorisasi USB terlebih dahulu.",
            device.state
        ));
    }

    let ip = wlan_ip(&adb, serial)?;
    let address = format!("{ip}:{WIRELESS_PORT}");

    if devices
        .iter()
        .any(|device| device.serial == address && device.state == "device")
    {
        return Ok(WirelessInfo {
            usb_serial: serial.to_string(),
            wifi_serial: address,
        });
    }

    let port = WIRELESS_PORT.to_string();
    run(&adb, &["-s", serial, "tcpip", &port])
        .map_err(|error| format!("Gagal mengaktifkan mode TCP/IP: {error}"))?;

    connect_wireless(&adb, &address)?;

    Ok(WirelessInfo {
        usb_serial: serial.to_string(),
        wifi_serial: address,
    })
}

/// Connect to a device that is already listening on the network, given its address.
///
/// The handoff in [`enable_wireless`] needs a USB connection, but `adb tcpip` mode and the
/// ADB server's TCP connections do not survive a phone reboot or an `adb kill-server`. A
/// phone that was reachable yesterday therefore disappears from `adb devices` today with
/// nothing in the list to point at, so the address is reconnectable on its own.
pub fn connect_address(input: &str) -> Result<String, String> {
    let address = normalize_wireless_address(input)?;
    let adb = adb_path()?;

    let output =
        run_with_timeout(&adb, &["connect", &address], CONNECT_TIMEOUT).map_err(|error| {
            format!(
                "HP di {address} tidak menjawab dalam {} detik. Matikan layar HP \
                 mungkin sedang tidur — tekan tombol power HP sekali lalu coba lagi, atau pastikan \
                 HP dan PC memakai jaringan Wi-Fi yang sama. ({error})",
                CONNECT_TIMEOUT.as_secs()
            )
        })?;
    let text = output.trim().to_string();

    if text.contains("cannot") || text.contains("unable") || text.contains("failed") {
        return Err(format!(
            "ADB menolak {address} — {text}. Pastikan HP dan PC di jaringan Wi-Fi yang sama, \
             USB debugging aktif, dan HP masih dalam mode tcpip (mode ini hilang saat HP \
             dinyalakan ulang)."
        ));
    }

    if wait_for_wireless(&address) {
        return Ok(address);
    }

    Err(format!(
        "{address} tersambung tetapi perangkat tidak siap. Buka layar HP dan setujui \
         dialog debugging bila muncul, lalu coba lagi."
    ))
}

pub fn disconnect_address(address: &str) -> Result<(), String> {
    let address = normalize_wireless_address(address)?;
    let adb = adb_path()?;
    run_with_timeout(&adb, &["disconnect", &address], RECONNECT_TIMEOUT).map(|_| ())
}

/// The pairing code Android shows is exactly six digits; anything else cannot reach the
/// phone's dialog, so it is rejected before ADB is spawned.
fn validate_pair_code(input: &str) -> Result<String, String> {
    let code = input.trim();
    if code.len() != 6 || !code.chars().all(|digit| digit.is_ascii_digit()) {
        return Err("Kode pairing harus 6 angka, seperti yang ditampilkan HP.".to_string());
    }
    Ok(code.to_string())
}

/// Wireless debugging pairing (Android 11+): the phone shows a six-digit code and its own
/// pairing port under "Pair device with pairing code", and the TLS exchange done by
/// `adb pair` is what lets a later `adb connect` reach the phone without the older
/// `adb tcpip` handoff. The pairing port is the one from the dialog — a bare IP would be
/// normalized to 5555, which is the connect port, so an explicit port is required here.
pub fn pair_address(input: &str, code: &str) -> Result<String, String> {
    let address = if input.trim().contains(':') {
        normalize_wireless_address(input)?
    } else {
        return Err(
            "Alamat pairing harus menyertakan port yang ditampilkan HP, misalnya \
             192.168.1.20:37123 — bukan IP saja."
                .to_string(),
        );
    };
    let code = validate_pair_code(code)?;

    let adb = adb_path()?;
    let output = run_with_timeout(&adb, &["pair", &address, &code], DEFAULT_TIMEOUT)
        .map_err(|error| {
            format!(
                "HP menolak pairing di {address} — {error} Kode hanya berlaku selama dialog \
                 \"Pair device with pairing code\" masih terbuka di HP; tutup lalu buka lagi \
                 dialog itu dan ulangi."
            )
        })?;
    let text = output.trim().to_string();

    if text.to_lowercase().contains("successfully paired") {
        return Ok(address);
    }
    Err(format!(
        "Pairing di {address} tidak berhasil ({text}). Pastikan kode masih tampil di HP dan \
         dialog pairing-nya belum ditutup."
    ))
}

/// Best-effort reconnect used by the background device poll. Returns a message only when
/// the attempt failed, so an unreachable phone does not raise an error on every pass but
/// the reason is still available to show when nothing else explains an empty list.
pub(crate) fn try_reconnect(address: &str) -> Option<String> {
    let adb = match adb_path() {
        Ok(path) => path,
        Err(error) => return Some(error),
    };

    match run_with_timeout(&adb, &["connect", address], RECONNECT_TIMEOUT) {
        Ok(output) => {
            let text = output.trim().to_string();
            let lowered = text.to_lowercase();
            if lowered.contains("cannot")
                || lowered.contains("unable")
                || lowered.contains("failed")
            {
                Some(format!("{address}: {text}"))
            } else {
                None
            }
        }
        Err(error) => Some(format!("{address}: {error}")),
    }
}

/// Accept `192.168.1.20`, `192.168.1.20:5555`, or a pasted `tcp://`/`adb connect` form and
/// return the canonical `ip:port` ADB expects. Only IPv4 is allowed so an address can never
/// be turned into a host name that ADB resolves elsewhere.
pub fn normalize_wireless_address(input: &str) -> Result<String, String> {
    let cleaned = input
        .trim()
        .trim_start_matches("tcp://")
        .trim_start_matches("adb connect")
        .trim()
        .trim_end_matches('/')
        .to_string();

    if cleaned.is_empty() {
        return Err("Alamat perangkat kosong.".to_string());
    }

    let (host, port) = match cleaned.rsplit_once(':') {
        Some((host, port)) => {
            let port: u16 = port
                .parse()
                .map_err(|_| format!("Port tidak valid: {port}"))?;
            if port == 0 {
                return Err("Port tidak boleh 0.".to_string());
            }
            (host, port)
        }
        None => (cleaned.as_str(), WIRELESS_PORT),
    };

    let ip = validate_ipv4(host)?;
    let address = format!("{ip}:{port}");
    if !is_valid_serial(&address) {
        return Err(format!("Alamat perangkat tidak valid: {address}"));
    }

    Ok(address)
}

fn connect_wireless(adb: &Path, address: &str) -> Result<(), String> {
    let mut last_error = String::new();

    for _ in 0..5 {
        match run(adb, &["connect", address]) {
            Ok(output) => {
                let text = output.trim().to_string();
                if text.contains("cannot") || text.contains("failed") || text.contains("unable") {
                    last_error = text;
                } else if wait_for_wireless(address) {
                    return Ok(());
                } else {
                    last_error = format!("ADB belum melaporkan {address} sebagai terhubung.");
                }
            }
            Err(error) => last_error = error,
        }
        std::thread::sleep(Duration::from_millis(600));
    }

    Err(format!(
        "Gagal terhubung ke {address}. Pastikan HP dan PC berada di jaringan Wi-Fi yang sama \
         dan port {WIRELESS_PORT} tidak diblokir firewall. ({last_error})"
    ))
}

fn wait_for_wireless(address: &str) -> bool {
    for _ in 0..12 {
        if let Ok(devices) = list_devices() {
            if devices
                .iter()
                .any(|device| device.serial == address && device.state == "device")
            {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

fn wlan_ip(adb: &Path, serial: &str) -> Result<String, String> {
    if let Ok(routes) = run(adb, &["-s", serial, "shell", "ip", "route"]) {
        if let Some(ip) = parse_route_src(&routes) {
            return validate_ipv4(&ip);
        }
    }

    let addresses = run(
        adb,
        &[
            "-s", serial, "shell", "ip", "-f", "inet", "addr", "show", "wlan0",
        ],
    )
    .map_err(|error| format!("Gagal membaca alamat IP Wi-Fi HP: {error}"))?;

    parse_inet_addr(&addresses)
        .and_then(|ip| validate_ipv4(&ip).ok())
        .ok_or_else(|| {
            "Alamat IP Wi-Fi HP tidak ditemukan. Pastikan Wi-Fi HP menyala dan terhubung ke \
             jaringan yang sama dengan PC."
                .to_string()
        })
}

fn parse_route_src(output: &str) -> Option<String> {
    for line in output.lines() {
        if !line.contains("wlan") {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(position) = tokens.iter().position(|token| *token == "src") else {
            continue;
        };
        if let Some(ip) = tokens.get(position + 1) {
            return Some(ip.to_string());
        }
    }

    None
}

fn parse_inet_addr(output: &str) -> Option<String> {
    for line in output.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(position) = tokens.iter().position(|token| *token == "inet") else {
            continue;
        };
        let value = tokens.get(position + 1)?;
        let ip = value.split('/').next()?;
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    None
}

fn validate_ipv4(ip: &str) -> Result<String, String> {
    let parsed: std::net::Ipv4Addr = ip
        .parse()
        .map_err(|_| format!("Alamat IP Wi-Fi tidak valid: {ip}"))?;

    if parsed.is_unspecified() || parsed.is_loopback() || parsed.is_link_local() {
        return Err(format!("Alamat IP Wi-Fi tidak dapat dipakai: {ip}"));
    }

    Ok(parsed.to_string())
}

/// Whether ADB can be found, without starting it. `adb version` spawns the client (and
/// starts the server when it is not running), which is more than a settings badge is worth
/// on page open; the full read is a separate, explicit action.
pub fn locate() -> AdbStatus {
    match adb_path() {
        Ok(path) => AdbStatus {
            found: true,
            path: Some(path.display().to_string()),
            version: None,
            error: None,
        },
        Err(error) => AdbStatus {
            found: false,
            path: None,
            version: None,
            error: Some(error),
        },
    }
}

pub fn status() -> AdbStatus {
    let path = match adb_path() {
        Ok(path) => path,
        Err(error) => {
            return AdbStatus {
                found: false,
                path: None,
                version: None,
                error: Some(error),
            }
        }
    };

    let display_path = path.display().to_string();
    match run(&path, &["version"]) {
        Ok(output) => AdbStatus {
            found: true,
            path: Some(display_path),
            version: output
                .lines()
                .next()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty()),
            error: None,
        },
        Err(error) => AdbStatus {
            found: true,
            path: Some(display_path),
            version: None,
            error: Some(error),
        },
    }
}

pub fn parse_devices(stdout: &str) -> Vec<AndroidDevice> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("List of devices") && !line.starts_with('*'))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let state = fields.next()?;
            if !is_valid_serial(serial) {
                return None;
            }

            let mut device = AndroidDevice {
                serial: serial.to_string(),
                state: state.to_string(),
                connection_type: if serial.contains(':') { "wifi" } else { "usb" }.to_string(),
                model: None,
                product: None,
                device_codename: None,
                transport_id: None,
            };

            for field in fields {
                let Some((key, value)) = field.split_once(':') else {
                    continue;
                };
                match key {
                    "model" => device.model = Some(value.replace('_', " ")),
                    "product" => device.product = Some(value.to_string()),
                    "device" => device.device_codename = Some(value.to_string()),
                    "transport_id" => device.transport_id = Some(value.to_string()),
                    _ => {}
                }
            }

            Some(device)
        })
        .collect()
}

pub fn is_valid_serial(serial: &str) -> bool {
    !serial.is_empty()
        && serial.len() <= 128
        && serial
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_keyguard_from_the_delegate_block() {
        // Field names and indentation copied from a locked, asleep HyperOS dump.
        let dump = concat!(
            "    KeyguardServiceDelegate\n",
            "      showing=true\n",
            "      secure=true\n",
            "      screenState=SCREEN_STATE_OFF\n",
            "      KeyguardStateMonitor\n",
            "        mIsShowing=true\n",
            "        mTrusted=false\n",
            "    Looper state:\n",
        );
        assert!(keyguard_reported(dump));
    }

    #[test]
    fn one_answer_carries_both_verdicts_without_cross_talk() {
        // The keyguard block reports its own screen state; that must not answer for
        // `dumpsys power`, which here says the panel is awake and unlocked.
        let answer = concat!(
            "  mWakefulness=Awake\n",
            "  mScreenOn=true\n",
            "    KeyguardServiceDelegate\n",
            "      showing=false\n",
            "      screenState=SCREEN_STATE_OFF\n",
            "    Looper state:\n",
        );
        let (screen_on, keyguard_up) = wake_state_reported(answer).expect("both verdicts");
        assert!(screen_on);
        assert!(!keyguard_up);
    }

    #[test]
    fn a_missing_keyguard_flag_is_not_reported_as_unlocked() {
        // No `showing=` line means "no answer", and the caller then falls back to the
        // dedicated read instead of claiming the phone is sitting open.
        let answer = "  mWakefulness=Asleep\n  mScreenOn=false\n";
        assert!(wake_state_reported(answer).is_none());
    }

    #[test]
    fn unlocked_phone_ignores_stale_display_policy_copies() {
        // The observed dump carries one DisplayPolicy per display and their keyguard
        // fields disagree (mAwake=true next to mAwake=false), and mDreamingLockscreen
        // stays true while no dream runs. None of that may outvote the delegate block.
        let dump = concat!(
            "  DisplayPolicy\n",
            "    mAwake=true mScreenOnEarly=true mScreenOnFully=true\n",
            "    mShowingDream=false mDreamingLockscreen=true\n",
            "    isKeyguardShowing=false\n",
            "  DisplayPolicy\n",
            "    mAwake=false mScreenOnEarly=false mScreenOnFully=false\n",
            "    mShowingDream=false mDreamingLockscreen=true\n",
            "    isKeyguardShowing=true\n",
            "    KeyguardServiceDelegate\n",
            "      showing=false\n",
            "      secure=true\n",
            "      screenState=SCREEN_STATE_ON\n",
            "      KeyguardStateMonitor\n",
            "        mIsShowing=false\n",
            "        mTrusted=true\n",
            "    Looper state:\n",
        );
        assert!(!keyguard_reported(dump));
    }

    #[test]
    fn falls_back_to_legacy_fields_without_the_delegate_block() {
        assert!(keyguard_reported("  isKeyguardShowing=true\n"));
        assert!(keyguard_reported("    mShowingLockscreen=true\n"));
        assert!(!keyguard_reported(
            "  isKeyguardShowing=false\nmDreamingLockscreen=true\n"
        ));
        assert!(!keyguard_reported("no keyguard fields here\n"));
    }

    #[test]
    fn parses_usb_and_wireless_devices() {
        let output = "List of devices attached\n\
            R58M12ABCDE\tdevice product:beyond1lte model:SM_G973F device:beyond1 transport_id:1\n\
            192.168.1.20:5555\tunauthorized product:panther model:Pixel_7 device:panther transport_id:2\n\
            emulator-5554\toffline\n\n";
        let devices = parse_devices(output);

        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].serial, "R58M12ABCDE");
        assert_eq!(devices[0].connection_type, "usb");
        assert_eq!(devices[0].model.as_deref(), Some("SM G973F"));
        assert_eq!(devices[1].connection_type, "wifi");
        assert_eq!(devices[1].state, "unauthorized");
        assert_eq!(devices[2].state, "offline");
    }

    #[test]
    fn ignores_invalid_serial() {
        let output = "List of devices attached\n$(rm -rf /)\tdevice\n";
        assert!(parse_devices(output).is_empty());
    }

    #[test]
    fn parses_wlan_ip_from_routes() {
        let routes = "192.168.1.0/24 dev wlan0 proto kernel scope link src 192.168.1.10\n\
            10.0.0.0/8 dev rmnet0 proto kernel scope link src 10.23.4.5\n";
        assert_eq!(parse_route_src(routes).as_deref(), Some("192.168.1.10"));
        assert_eq!(
            parse_route_src("10.0.0.0/8 dev rmnet0 proto kernel scope link src 10.23.4.5\n"),
            None
        );
    }

    #[test]
    fn falls_back_to_inet_address() {
        let output = "3: wlan0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500\n\
            inet 192.168.1.10/24 brd 192.168.1.255 scope global wlan0\n";
        assert_eq!(parse_inet_addr(output).as_deref(), Some("192.168.1.10"));
    }

    #[test]
    fn rejects_unusable_wireless_addresses() {
        assert!(validate_ipv4("127.0.0.1").is_err());
        assert!(validate_ipv4("169.254.1.2").is_err());
        assert!(validate_ipv4("192.168.1.10").is_ok());
    }

    #[test]
    fn normalizes_typed_wireless_addresses() {
        assert_eq!(
            normalize_wireless_address("192.168.1.20").unwrap(),
            "192.168.1.20:5555"
        );
        assert_eq!(
            normalize_wireless_address("  192.168.1.20:37021 ").unwrap(),
            "192.168.1.20:37021"
        );
        assert_eq!(
            normalize_wireless_address("tcp://192.168.1.20:5555").unwrap(),
            "192.168.1.20:5555"
        );
        assert!(normalize_wireless_address("nas-pintar:5555").is_err());
        assert!(normalize_wireless_address("192.168.1.20:0").is_err());
        assert!(normalize_wireless_address("").is_err());
    }

    #[test]
    fn pairing_codes_are_six_digits_or_nothing() {
        assert_eq!(validate_pair_code(" 123456 ").unwrap(), "123456");
        assert!(validate_pair_code("12345").is_err());
        assert!(validate_pair_code("1234567").is_err());
        assert!(validate_pair_code("12ab56").is_err());
        assert!(validate_pair_code("").is_err());
    }

    /// The state probes are shell pipelines that run on the phone, so their parsing must be
    /// checked against a real dump, not only against the fixtures above.
    #[test]
    #[ignore = "butuh HP nyata; jalankan dengan YAOP_TEST_ADDRESS=ip-hp cargo test -- --ignored"]
    fn reads_live_power_and_keyguard_state() {
        let serial = std::env::var("YAOP_TEST_ADDRESS").expect("set YAOP_TEST_ADDRESS dulu");
        let screen_on = screen_probe(&serial).expect("status layar gagal dibaca");
        let locked = keyguard_probe(&serial).expect("status kunci gagal dibaca");
        println!("screen_on={screen_on} keyguard_locked={locked}");
    }

    #[test]
    #[ignore = "butuh HP nyata; jalankan dengan YAOP_TEST_ADDRESS=ip-hp cargo test -- --ignored"]
    fn connects_to_a_real_wireless_device() {
        let input = std::env::var("YAOP_TEST_ADDRESS").expect("set YAOP_TEST_ADDRESS dulu");
        let address = connect_address(&input).expect("koneksi gagal");
        let devices = list_devices().expect("daftar perangkat gagal dibaca");
        assert!(
            devices
                .iter()
                .any(|device| device.serial == address && device.state == "device"),
            "{address} terhubung tetapi tidak berstatus device"
        );
    }

    #[cfg(windows)]
    #[test]
    fn handoff_connects_fake_usb_device() {
        const SCRIPT: &str = "@echo off\r\n\
            if \"%1\"==\"devices\" goto devices\r\n\
            if \"%1\"==\"connect\" goto connect\r\n\
            if \"%1\"==\"-s\" goto device\r\n\
            goto done\r\n\
            :devices\r\n\
            echo List of devices attached\r\n\
            echo USB123 device product:test model:Test_Phone device:test transport_id:1\r\n\
            if exist \"%~dp0wifi-on\" echo 192.168.1.50:5555 device product:test model:Test_Phone device:test transport_id:2\r\n\
            goto done\r\n\
            :connect\r\n\
            type nul > \"%~dp0wifi-on\"\r\n\
            echo connected to %2\r\n\
            goto done\r\n\
            :device\r\n\
            if \"%3\"==\"tcpip\" goto done\r\n\
            if \"%3\"==\"shell\" echo 192.168.1.0/24 dev wlan0 proto kernel scope link src 192.168.1.50\r\n\
            goto done\r\n\
            :done\r\n\
            exit /b 0\r\n";

        let directory = std::env::temp_dir().join(format!("yaop-fake-adb-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("adb.cmd");
        std::fs::write(&script, SCRIPT).unwrap();
        let _ = std::fs::remove_file(directory.join("wifi-on"));

        std::env::set_var("YAOP_ADB_PATH", &script);
        let info = enable_wireless("USB123");
        let second = if info.is_ok() {
            Some(enable_wireless("USB123"))
        } else {
            None
        };
        std::env::remove_var("YAOP_ADB_PATH");
        let _ = std::fs::remove_dir_all(&directory);

        let info = info.expect("handoff pertama gagal");
        assert_eq!(info.wifi_serial, "192.168.1.50:5555");
        assert_eq!(
            second
                .expect("handoff kedua tidak dijalankan")
                .expect("handoff kedua gagal")
                .wifi_serial,
            "192.168.1.50:5555"
        );
    }

    #[test]
    fn maps_adb_reachability_failures_to_actionable_text() {
        assert!(explain_device_error(
            "192.168.1.3:5555",
            "adb: device '192.168.1.3:5555' not found"
        )
        .contains("tidak tersambung"));
        assert!(
            explain_device_error("USB123", "error: device unauthorized.").contains("Izinkan")
        );
        assert_eq!(
            explain_device_error("USB123", "Failed to copy: Permission denied"),
            "Failed to copy: Permission denied"
        );
    }
}
