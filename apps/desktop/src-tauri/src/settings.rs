use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const MAX_KNOWN_WIRELESS: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_max_size")]
    pub max_size: u32,
    #[serde(default)]
    pub video_bitrate_kbps: u32,
    /// Frame rate cap for the mirror. A phone panel runs 90–120 Hz, and without a cap
    /// scrcpy spreads the bitrate over that many frames per second, which is what makes a
    /// Wi-Fi session look choppy at a modest bitrate. 0 leaves the decision to scrcpy.
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,
    #[serde(default = "default_download_dir")]
    pub download_dir: String,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub mirror_screen_off: bool,
    /// Latency-first preset for a Wi-Fi session: cap the frame budget so every frame is
    /// small enough to cross a link that is already retransmitting. It only ever lowers what
    /// the three quality fields say, never raises them.
    #[serde(default)]
    pub low_latency: bool,
    /// Forward the phone's playback audio into the PC through the scrcpy session. The
    /// capture needs Android 13 on the phone (checked when the session starts), and it
    /// adds about 128 kbps of Opus on top of the video — negligible even on the shared
    /// 2.4 GHz link.
    #[serde(default = "default_true")]
    pub audio_enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_max_fps() -> u32 {
    60
}

/// scrcpy scales the *longest* edge to this value, and a phone panel is 2340–2560 px tall,
/// so the usual "1080p" reading would shrink a phone to about 480x1040. 1920 keeps a phone
/// near native sharpness while still taking the worst of the bitrate pressure off a tall
/// panel.
fn default_max_size() -> u32 {
    1920
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_size: default_max_size(),
            video_bitrate_kbps: 0,
            max_fps: default_max_fps(),
            download_dir: default_download_dir(),
            close_to_tray: true,
            mirror_screen_off: false,
            low_latency: false,
            audio_enabled: true,
        }
    }
}

fn default_download_dir() -> String {
    let downloads = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join("Downloads"))
        .filter(|path| path.is_dir());

    match downloads.or_else(|| std::env::current_dir().ok()) {
        Some(path) => path.display().to_string(),
        None => String::new(),
    }
}

pub fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    let Some(path) = config_path(app) else {
        return Settings::default();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Settings::default();
    };
    let Ok(mut settings) = serde_json::from_str::<Settings>(&text) else {
        return Settings::default();
    };

    if validate(&mut settings).is_err() {
        // A validation failure usually means the saved download folder moved or was deleted.
        // Repairing just that field keeps the rest — including remembered wireless
        // addresses — instead of silently resetting everything the user configured.
        settings.download_dir = default_download_dir();
        if validate(&mut settings).is_err() {
            return Settings::default();
        }
    }
    settings
}

/// Wireless ADB addresses that connected at least once, most recent first.
///
/// `adb tcpip` mode and the ADB server's TCP connections survive neither a phone reboot nor
/// `adb kill-server`, so a phone that worked yesterday disappears from `adb devices` today
/// with nothing in the list to point at. Keeping the address lets the app reconnect it, or
/// offer it in the UI, without asking for the cable again. It lives in its own file so a
/// rejected settings write can never wipe it.
pub fn known_wireless(app: &AppHandle) -> Vec<String> {
    let Some(path) = wireless_store_path(app) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut addresses: Vec<String> = serde_json::from_str::<Vec<String>>(&text)
        .unwrap_or_default()
        .into_iter()
        .filter(|address| address.contains(':') && crate::adb::is_valid_serial(address))
        .collect();
    let mut seen = std::collections::HashSet::new();
    addresses.retain(|address| seen.insert(address.clone()));
    addresses.truncate(MAX_KNOWN_WIRELESS);
    addresses
}

pub fn remember_wireless(app: &AppHandle, address: &str) -> Result<(), String> {
    let mut addresses = known_wireless(app);
    addresses.retain(|known| known != address);
    addresses.insert(0, address.to_string());
    write_wireless(app, &addresses)
}

pub fn forget_wireless(app: &AppHandle, address: &str) -> Result<(), String> {
    let mut addresses = known_wireless(app);
    addresses.retain(|known| known != address);
    write_wireless(app, &addresses)
}

fn write_wireless(app: &AppHandle, addresses: &[String]) -> Result<(), String> {
    let path = wireless_store_path(app)
        .ok_or_else(|| "Folder konfigurasi aplikasi tidak tersedia.".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Gagal membuat folder konfigurasi: {error}"))?;
    }

    let text = serde_json::to_string_pretty(addresses)
        .map_err(|error| format!("Gagal menyusun daftar perangkat: {error}"))?;
    std::fs::write(&path, text)
        .map_err(|error| format!("Gagal menyimpan daftar perangkat: {error}"))
}

fn wireless_store_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join("wireless-devices.json"))
}

pub fn save(app: &AppHandle, mut settings: Settings) -> Result<Settings, String> {
    validate(&mut settings)?;

    let path = config_path(app)
        .ok_or_else(|| "Folder konfigurasi aplikasi tidak tersedia.".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Gagal membuat folder konfigurasi: {error}"))?;
    }

    let text = serde_json::to_string_pretty(&settings)
        .map_err(|error| format!("Gagal menyusun pengaturan: {error}"))?;
    std::fs::write(&path, text)
        .map_err(|error| format!("Gagal menyimpan pengaturan ke {}: {error}", path.display()))?;

    Ok(settings)
}

fn validate(settings: &mut Settings) -> Result<(), String> {
    if settings.max_size != 0 && !(320..=4096).contains(&settings.max_size) {
        return Err("Resolusi maksimum harus antara 320 dan 4096 piksel.".to_string());
    }
    if settings.video_bitrate_kbps != 0 && !(100..=100_000).contains(&settings.video_bitrate_kbps) {
        return Err("Bitrate video harus antara 0,1 dan 100 Mbps.".to_string());
    }
    if settings.max_fps != 0 && !(1..=240).contains(&settings.max_fps) {
        return Err(
            "Batas FPS harus antara 1 dan 240, atau 0 untuk mengikuti perangkat.".to_string(),
        );
    }

    let directory = PathBuf::from(settings.download_dir.trim());
    if !directory.is_absolute() || !directory.is_dir() {
        return Err(format!(
            "Folder unduhan tidak ditemukan: {}",
            settings.download_dir
        ));
    }
    settings.download_dir = directory.display().to_string();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unbounded_quality_values() {
        let mut settings = Settings {
            max_size: 10,
            download_dir: std::env::temp_dir().display().to_string(),
            ..Settings::default()
        };
        assert!(validate(&mut settings).is_err());

        settings.max_size = 1080;
        settings.video_bitrate_kbps = 0;
        assert!(validate(&mut settings).is_ok());
    }

    #[test]
    fn rejects_absurd_frame_rate_caps() {
        let mut settings = Settings {
            download_dir: std::env::temp_dir().display().to_string(),
            ..Settings::default()
        };
        settings.max_fps = 300;
        assert!(validate(&mut settings).is_err());

        settings.max_fps = 0;
        assert!(validate(&mut settings).is_ok());
    }

    #[test]
    fn settings_written_before_the_frame_cap_exists_stay_readable() {
        // The shipped settings.json carries no maxFps; the field must default rather than
        // fail the parse, which used to reset everything else in the file.
        let settings: Settings =
            serde_json::from_str(r#"{"maxSize":0,"videoBitrateKbps":4000,"downloadDir":"C:/"}"#)
                .expect("konfigurasi lama tidak dapat dibaca");
        assert_eq!(settings.max_fps, 60);
        assert_eq!(settings.max_size, 0);
        assert!(!settings.low_latency);
        assert!(settings.audio_enabled);
    }

    #[test]
    fn rejects_missing_download_directory() {
        let mut settings = Settings {
            max_size: 0,
            download_dir: "Z:/folder-yang-tidak-ada".to_string(),
            ..Settings::default()
        };
        assert!(validate(&mut settings).is_err());
    }
}
