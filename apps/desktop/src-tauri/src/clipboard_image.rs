use crate::adb;
use crate::transfer;
use serde::Serialize;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const REMOTE_DIR: &str = "/sdcard/Download/";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSendResult {
    pub file_name: String,
    pub share_opened: bool,
}

pub fn send_clipboard_image(app: &AppHandle, serial: &str) -> Result<ImageSendResult, String> {
    if !adb::is_valid_serial(serial) {
        return Err("Serial perangkat tidak valid.".to_string());
    }

    let capture = capture_png()?;
    let outcome = push_and_share(app, serial, &capture);
    let _ = std::fs::remove_file(&capture.path);
    outcome
}

struct Capture {
    path: PathBuf,
    file_name: String,
}

fn capture_png() -> Result<Capture, String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Clipboard tidak dapat dibuka: {error}"))?;

    let image = match clipboard.get_image() {
        Ok(image) => image,
        Err(arboard::Error::ContentNotAvailable) => {
            return Err(
                "Clipboard tidak berisi gambar. Salin screenshot/gambar terlebih dahulu."
                    .to_string(),
            )
        }
        Err(error) => return Err(format!("Gagal membaca gambar dari clipboard: {error}")),
    };

    let expected = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Ukuran gambar clipboard tidak valid.".to_string())?;
    if image.width == 0 || image.height == 0 || image.bytes.len() < expected {
        return Err("Gambar clipboard tidak valid.".to_string());
    }

    let file_name = format!("yaop-clipboard-{}.png", now_ms());
    let path = std::env::temp_dir().join(&file_name);
    let file = File::create(&path)
        .map_err(|error| format!("Gagal menyiapkan file gambar sementara: {error}"))?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(file),
        image.width as u32,
        image.height as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("Gagal menulis PNG: {error}"))?;
    writer
        .write_image_data(&image.bytes)
        .map_err(|error| format!("Gagal menulis PNG: {error}"))?;
    writer
        .finish()
        .map_err(|error| format!("Gagal menyelesaikan PNG: {error}"))?;

    Ok(Capture { path, file_name })
}

fn push_and_share(
    app: &AppHandle,
    serial: &str,
    capture: &Capture,
) -> Result<ImageSendResult, String> {
    if !is_safe_name(&capture.file_name) {
        return Err("Nama file gambar tidak valid.".to_string());
    }

    let path = capture.path.to_string_lossy().to_string();
    let pushed = transfer::push_files(app, serial, &[path], None)?;
    if pushed.pushed == 0 {
        let reason = pushed
            .failed
            .first()
            .map(|failure| failure.error.clone())
            .unwrap_or_else(|| "Gambar gagal dikirim ke perangkat.".to_string());
        return Err(reason);
    }

    let adb = adb::adb_path()?;
    let share_opened = open_share(&adb, serial, &capture.file_name).is_ok();

    Ok(ImageSendResult {
        file_name: capture.file_name.clone(),
        share_opened,
    })
}

/// The Android 10+ media store no longer takes a pushed file into `content://media` from the
/// `MEDIA_SCANNER_SCAN_FILE` broadcast: on the test device (Android 16) the file never
/// appeared, and each `content query` cost 1.3–5.5 s, so the old six-round poll spent over
/// half a minute on a URI it never found. The share sheet is handed the pushed file directly
/// instead, and the copy stays in Download either way.
fn open_share(adb: &std::path::Path, serial: &str, file_name: &str) -> Result<(), String> {
    let uri = format!("file://{REMOTE_DIR}{file_name}");
    let mut command = Command::new(adb);
    command
        .args(["-s", serial, "shell", "am", "start"])
        .args(["-a", "android.intent.action.SEND", "-t", "image/png"])
        .args(["--eu", "android.intent.extra.STREAM", &uri])
        .arg("--grant-read-uri-permission");
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = command
        .output()
        .map_err(|error| format!("Gagal membuka lembar Bagikan: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Android menolak membuka lembar Bagikan: {}",
            stderr.trim()
        ));
    }
    Ok(())
}

fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
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
    fn accepts_generated_names_only() {
        assert!(is_safe_name("yaop-clipboard-1792905050.png"));
        assert!(!is_safe_name("../evil.png"));
        assert!(!is_safe_name("a b.png"));
        assert!(!is_safe_name(""));
    }
}
