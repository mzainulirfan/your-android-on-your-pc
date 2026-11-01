use crate::adb;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const MAX_PATH_LEN: usize = 512;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<String>,
}

pub fn list(serial: &str, path: &str) -> Result<Vec<AndroidEntry>, String> {
    adb::validate_serial(serial)?;
    let directory = validate_remote_dir(path)?;
    let adb = adb::adb_path()?;

    let mut command = Command::new(&adb);
    command
        .arg("-s")
        .arg(serial)
        .arg("shell")
        .arg("ls")
        .arg("-la")
        .arg(shell_quote(&directory))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = command
        .output()
        .map_err(|error| format!("Gagal membaca folder HP: {error}"))?;
    if !output.status.success() {
        let message = adb::explain_device_error(
            serial,
            String::from_utf8_lossy(&output.stderr).trim(),
        );
        return Err(if message.is_empty() {
            format!("Gagal membaca folder {directory}.")
        } else {
            format!("Gagal membaca folder {directory}: {message}")
        });
    }

    Ok(parse_listing(
        &directory,
        &String::from_utf8_lossy(&output.stdout),
    ))
}

pub fn validate_remote_dir(path: &str) -> Result<String, String> {
    validate_remote(path)
}

pub fn validate_remote_file(path: &str) -> Result<String, String> {
    let path = validate_remote(path)?;
    if path == "/sdcard" || path == "/storage/emulated/0" {
        return Err("Pilih satu file, bukan folder akar.".to_string());
    }
    Ok(path)
}

fn validate_remote(path: &str) -> Result<String, String> {
    let trimmed_input = path.trim();
    if trimmed_input.is_empty() {
        return Err("Path folder HP tidak boleh kosong.".to_string());
    }
    if trimmed_input.len() > MAX_PATH_LEN {
        return Err("Path folder HP terlalu panjang.".to_string());
    }

    let trimmed = trimmed_input.trim_end_matches('/');
    let root = if trimmed == "/sdcard" || trimmed.starts_with("/sdcard/") {
        "/sdcard"
    } else if trimmed == "/storage/emulated/0" || trimmed.starts_with("/storage/emulated/0/") {
        "/storage/emulated/0"
    } else {
        return Err("Hanya folder publik /sdcard yang dapat diakses.".to_string());
    };

    let relative = trimmed.strip_prefix(root).unwrap_or("");
    if relative.is_empty() {
        return Ok(root.to_string());
    }

    for segment in relative.trim_start_matches('/').split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err("Path folder HP tidak valid.".to_string());
        }
        if segment
            .chars()
            .any(|character| character == '\'' || character.is_control())
        {
            return Err(format!(
                "Nama \"{segment}\" mengandung karakter yang tidak didukung."
            ));
        }
    }

    Ok(trimmed.to_string())
}

pub fn remote_file_name(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("file")
}

pub fn unique_local_path(directory: &Path, name: &str) -> std::path::PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "file".to_string());
    let extension = path
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();

    for index in 1..1000 {
        let candidate = directory.join(format!("{stem} ({index}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }

    directory.join(format!(
        "{stem}-{}{extension}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default()
    ))
}

fn shell_quote(path: &str) -> String {
    format!("'{path}'")
}

fn parse_listing(directory: &str, output: &str) -> Vec<AndroidEntry> {
    let mut entries: Vec<AndroidEntry> = output
        .lines()
        .filter_map(|line| {
            let (fields, raw_name) = split_fields(line, 7)?;
            let permissions = fields[0];
            let name = raw_name.trim_end_matches('\r').to_string();
            if name.is_empty() || permissions.starts_with('l') {
                return None;
            }

            if name == "." || name == ".." {
                return None;
            }
            if name.contains('/') || name.chars().any(|character| character.is_control()) {
                return None;
            }

            Some(AndroidEntry {
                path: format!("{}/{}", directory.trim_end_matches('/'), name),
                name,
                is_dir: permissions.starts_with('d'),
                size: fields[4].parse().unwrap_or(0),
                modified: Some(format!("{} {}", fields[5], fields[6])),
            })
        })
        .collect();

    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    entries
}

fn split_fields(line: &str, count: usize) -> Option<(Vec<&str>, &str)> {
    let bytes = line.as_bytes();
    let mut fields = Vec::with_capacity(count);
    let mut position = 0;

    while fields.len() < count {
        while position < bytes.len() && bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        if position >= bytes.len() {
            return None;
        }

        let start = position;
        while position < bytes.len() && !bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        fields.push(&line[start..position]);
    }

    while position < bytes.len() && bytes[position].is_ascii_whitespace() {
        position += 1;
    }
    let rest = &line[position..];
    if rest.is_empty() {
        return None;
    }

    Some((fields, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_android_listing_with_spaces_and_directories() {
        let output = "total 24\n\
            -rw-rw---- 1 u0_a268 media_rw 27939 2026-10-25 12:27 foto liburan.png\n\
            drwxrws--- 2 u0_a268 media_rw 3452 2026-05-06 19:11 stiker anak\n\
            lrwxrwxrwx 1 root root 21 2026-10-25 10:00 sdcard -> /storage/self/primary\n";

        let entries = parse_listing("/sdcard/Download", output);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "stiker anak");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "foto liburan.png");
        assert_eq!(entries[1].size, 27939);
        assert_eq!(entries[1].path, "/sdcard/Download/foto liburan.png");
    }

    #[test]
    fn rejects_paths_outside_public_storage() {
        assert!(validate_remote_dir("/data/local/tmp").is_err());
        assert!(validate_remote_dir("/sdcard/../data").is_err());
        assert!(validate_remote_dir("/sdcard/Download").is_ok());
        assert!(validate_remote_dir("/storage/emulated/0/DCIM").is_ok());
    }

    #[test]
    fn rejects_single_quote_in_remote_name() {
        assert!(validate_remote_dir("/sdcard/Download/it's").is_err());
    }

    #[test]
    fn extracts_remote_file_name() {
        assert_eq!(
            remote_file_name("/sdcard/Download/foto liburan.png"),
            "foto liburan.png"
        );
    }
}
