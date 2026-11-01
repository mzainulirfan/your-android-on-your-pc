use crate::{adb, settings};
use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotResult {
    pub path: String,
    pub file_name: String,
}

pub fn capture(app: &AppHandle, serial: &str) -> Result<ScreenshotResult, String> {
    adb::validate_serial(serial)?;
    let adb = adb::adb_path()?;

    let mut command = Command::new(&adb);
    command
        .arg("-s")
        .arg(serial)
        .arg("exec-out")
        .arg("screencap")
        .arg("-p")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = command
        .output()
        .map_err(|error| format!("Gagal mengambil screenshot: {error}"))?;
    if !output.status.success() {
        let message =
            adb::explain_device_error(serial, String::from_utf8_lossy(&output.stderr).trim());
        return Err(if message.is_empty() {
            "Screenshot gagal diambil dari perangkat.".to_string()
        } else {
            format!("Screenshot gagal: {message}")
        });
    }
    if !output.stdout.starts_with(&PNG_SIGNATURE) {
        return Err("Screenshot yang diterima bukan berkas PNG yang valid.".to_string());
    }

    let directory = PathBuf::from(settings::load(app).download_dir);
    if !directory.is_dir() {
        return Err(format!(
            "Folder unduhan tidak ditemukan: {}",
            directory.display()
        ));
    }

    let file_name = format!("yaop-screenshot-{}.png", now_ms());
    let path = directory.join(&file_name);
    std::fs::write(&path, &output.stdout)
        .map_err(|error| format!("Gagal menyimpan screenshot ke {}: {error}", path.display()))?;

    Ok(ScreenshotResult {
        path: path.display().to_string(),
        file_name,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
