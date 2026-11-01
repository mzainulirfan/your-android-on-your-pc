use crate::{adb, files};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const PUSH_TARGET: &str = "/sdcard/Download/";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub serial: String,
    pub direction: String,
    pub index: usize,
    pub total: usize,
    pub file_name: String,
    pub percent: u8,
    pub done: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    pub local_path: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferFailure {
    pub name: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferResult {
    pub pushed: usize,
    pub failed: Vec<TransferFailure>,
}

pub fn push_files(
    app: &AppHandle,
    serial: &str,
    paths: &[String],
    target_dir: Option<&str>,
) -> Result<TransferResult, String> {
    adb::validate_serial(serial)?;
    if paths.is_empty() {
        return Err("Tidak ada file yang dikirim.".to_string());
    }

    let target = match target_dir {
        Some(directory) => files::validate_remote_dir(directory)?,
        None => PUSH_TARGET.to_string(),
    };

    let adb = adb::adb_path()?;
    let total = paths.len();
    let mut pushed = 0;
    let mut failed = Vec::new();

    for (position, raw) in paths.iter().enumerate() {
        let path = PathBuf::from(raw);
        let name = display_name(&path);
        let index = position + 1;

        if !path.exists() {
            failed.push(TransferFailure {
                name,
                error: "File tidak ditemukan.".to_string(),
            });
            continue;
        }

        emit_progress(app, serial, "push", index, total, &name, 0, false);
        match push_one(app, &adb, serial, &path, &target, index, total, &name) {
            Ok(()) => {
                pushed += 1;
                emit_progress(app, serial, "push", index, total, &name, 100, true);
            }
            Err(error) => failed.push(TransferFailure { name, error }),
        }
    }

    Ok(TransferResult { pushed, failed })
}

fn push_one(
    app: &AppHandle,
    adb: &Path,
    serial: &str,
    path: &Path,
    target: &str,
    index: usize,
    total: usize,
    name: &str,
) -> Result<(), String> {
    let mut command = Command::new(adb);
    command
        .arg("-s")
        .arg(serial)
        .arg("push")
        .arg("-p")
        .arg(path)
        .arg(target)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Gagal menjalankan adb push: {error}"))?;

    let stderr_handle = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut buffer = String::new();
            let _ = stderr.read_to_string(&mut buffer);
            buffer
        })
    });

    if let Some(mut stdout) = child.stdout.take() {
        let mut chunk = [0u8; 1024];
        let mut pending: Vec<u8> = Vec::new();
        let mut last = 0u8;

        loop {
            let read = match stdout.read(&mut chunk) {
                Ok(read) => read,
                Err(_) => break,
            };
            if read == 0 {
                break;
            }
            pending.extend_from_slice(&chunk[..read]);

            let text = String::from_utf8_lossy(&pending);
            if let Some(value) = last_percent(&text) {
                if value > last && value <= 100 {
                    last = value;
                    emit_progress(app, serial, "push", index, total, name, value, false);
                }
            }

            if pending.len() > 128 {
                let keep = pending.split_off(pending.len() - 64);
                pending = keep;
            }
        }
    }

    let status = child
        .wait()
        .map_err(|error| format!("Gagal menunggu adb push: {error}"))?;
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    if !status.success() {
        let message = adb::explain_device_error(serial, stderr.trim());
        return Err(if message.is_empty() {
            format!("adb push keluar dengan status {status}")
        } else {
            message
        });
    }

    Ok(())
}

pub fn pull_file(
    app: &AppHandle,
    serial: &str,
    remote_path: &str,
    local_dir: &Path,
) -> Result<PullResult, String> {
    adb::validate_serial(serial)?;
    let remote = files::validate_remote_file(remote_path)?;
    if !local_dir.is_dir() {
        return Err(format!(
            "Folder tujuan tidak ditemukan: {}",
            local_dir.display()
        ));
    }

    let name = files::remote_file_name(&remote).to_string();
    let local = files::unique_local_path(local_dir, &name);
    let adb = adb::adb_path()?;

    let mut command = Command::new(&adb);
    command
        .arg("-s")
        .arg(serial)
        .arg("pull")
        .arg("-p")
        .arg(&remote)
        .arg(&local)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Gagal menjalankan adb pull: {error}"))?;

    let stderr_handle = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut buffer = String::new();
            let _ = stderr.read_to_string(&mut buffer);
            buffer
        })
    });

    emit_progress(app, serial, "pull", 1, 1, &name, 0, false);

    if let Some(mut stdout) = child.stdout.take() {
        let mut chunk = [0u8; 1024];
        let mut pending: Vec<u8> = Vec::new();
        let mut last = 0u8;

        loop {
            let read = match stdout.read(&mut chunk) {
                Ok(read) => read,
                Err(_) => break,
            };
            if read == 0 {
                break;
            }
            pending.extend_from_slice(&chunk[..read]);

            let text = String::from_utf8_lossy(&pending);
            if let Some(value) = last_percent(&text) {
                if value > last && value <= 100 {
                    last = value;
                    emit_progress(app, serial, "pull", 1, 1, &name, value, false);
                }
            }

            if pending.len() > 128 {
                let keep = pending.split_off(pending.len() - 64);
                pending = keep;
            }
        }
    }

    let status = child
        .wait()
        .map_err(|error| format!("Gagal menunggu adb pull: {error}"))?;
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    if !status.success() {
        let _ = std::fs::remove_file(&local);
        let message = adb::explain_device_error(serial, stderr.trim());
        return Err(if message.is_empty() {
            format!("adb pull keluar dengan status {status}")
        } else {
            message
        });
    }

    let bytes = std::fs::metadata(&local)
        .map_err(|error| format!("File hasil unduhan tidak ditemukan: {error}"))?
        .len();
    emit_progress(app, serial, "pull", 1, 1, &name, 100, true);

    Ok(PullResult {
        local_path: local.display().to_string(),
        bytes,
    })
}

fn emit_progress(
    app: &AppHandle,
    serial: &str,
    direction: &str,
    index: usize,
    total: usize,
    file_name: &str,
    percent: u8,
    done: bool,
) {
    let _ = app.emit(
        "transfer-progress",
        TransferProgress {
            serial: serial.to_string(),
            direction: direction.to_string(),
            index,
            total,
            file_name: file_name.to_string(),
            percent,
            done,
        },
    );
}

fn last_percent(text: &str) -> Option<u8> {
    let position = text.rfind('%')?;
    let digits: String = text[..position]
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    digits.parse().ok()
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_adb_progress_percentages() {
        assert_eq!(last_percent("[ 42%] /sdcard/Download/example"), Some(42));
        assert_eq!(last_percent("[100%] /sdcard/Download/example"), Some(100));
        assert_eq!(last_percent("1 file pushed, 0 skipped."), None);
    }

    #[test]
    fn uses_file_name_for_display() {
        assert_eq!(display_name(Path::new("C:\\tmp\\foto.jpg")), "foto.jpg");
    }
}
