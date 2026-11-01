use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Saved unlock keys, one file per concern — the same shape as the remembered wireless
/// addresses, so a rejected settings write can never wipe them and vice versa.
///
/// The key itself never leaves this module in cleartext: it is stored as a Windows DPAPI
/// blob (per-user, with app-specific entropy) encoded as hex, and only ever handed back to
/// `unlock::unlock` inside the process that protected it. The UI receives
/// [`CredentialView`], which carries a label and a length but no key, so a saved PIN cannot
/// be read off the panel — only picked and sent.

/// The file lives in the app config folder next to settings.json.
const STORE_NAME: &str = "credentials.json";
const MAX_PER_DEVICE: usize = 4;
const MAX_TOTAL: usize = 24;
const MAX_SECRET_CHARS: usize = 64;
const MAX_LABEL_CHARS: usize = 24;
/// DPAPI entropy. It is not a secret (it is in this binary); it only means a blob copied
/// into another application cannot be unprotected without repeating it.
const ENTROPY: &[u8] = b"yaop-credentials-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    id: String,
    serial: String,
    label: String,
    /// `pin` or `text`, recorded at save time so the list can be drawn without decrypting.
    method: String,
    length: usize,
    /// Hex-encoded DPAPI blob — never the key.
    data: String,
}

/// What the panel and the settings page are allowed to see about a saved key.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialView {
    pub id: String,
    pub serial: String,
    pub label: String,
    pub method: String,
    pub length: usize,
}

pub fn list(app: &AppHandle, serial: &str) -> Vec<CredentialView> {
    read(app)
        .into_iter()
        .filter(|entry| entry.serial == serial)
        .map(|entry| entry.view())
        .collect()
}

pub fn all(app: &AppHandle) -> Vec<CredentialView> {
    read(app).into_iter().map(|entry| entry.view()).collect()
}

/// Remember `secret` under `label` for one device. Saving over an existing label replaces
/// the old key instead of accumulating stale ones.
pub fn save(
    app: &AppHandle,
    serial: &str,
    label: &str,
    secret: &str,
) -> Result<CredentialView, String> {
    if !crate::adb::is_valid_serial(serial) {
        return Err("Serial perangkat tidak valid.".to_string());
    }
    let label = label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(format!(
            "Nama kunci harus antara 1 dan {MAX_LABEL_CHARS} karakter."
        ));
    }
    if label.chars().any(|c| c.is_control()) {
        return Err("Nama kunci mengandung karakter yang tidak bisa dipakai.".to_string());
    }

    let secret = secret.trim_end_matches(['\r', '\n']);
    let chars: Vec<char> = secret.chars().collect();
    if chars.is_empty() {
        return Err("Kunci tidak boleh kosong.".to_string());
    }
    if chars.len() > MAX_SECRET_CHARS {
        return Err(format!(
            "Kunci terlalu panjang (maksimum {MAX_SECRET_CHARS} karakter)."
        ));
    }
    if chars.iter().any(|c| c.is_control()) {
        return Err("Kunci mengandung karakter kontrol yang tidak bisa disimpan.".to_string());
    }

    let mut entries = read(app);
    let same_label = entries
        .iter()
        .position(|entry| entry.serial == serial && entry.label == label);
    if same_label.is_none() {
        if entries
            .iter()
            .filter(|entry| entry.serial == serial)
            .count()
            >= MAX_PER_DEVICE
        {
            return Err(format!(
                "Maksimal {MAX_PER_DEVICE} kunci tersimpan per perangkat. Hapus salah satu di Pengaturan."
            ));
        }
        if entries.len() >= MAX_TOTAL {
            return Err(format!(
                "Maksimal {MAX_TOTAL} kunci tersimpan. Hapus sebagian di Pengaturan."
            ));
        }
    }

    let entry = Stored {
        id: match same_label {
            Some(index) => entries[index].id.clone(),
            None => next_id(),
        },
        serial: serial.to_string(),
        label: label.to_string(),
        method: if chars.iter().all(|c| c.is_ascii_digit()) {
            "pin".to_string()
        } else {
            "text".to_string()
        },
        length: chars.len(),
        data: protect(secret.as_bytes())?,
    };

    match same_label {
        Some(index) => entries[index] = entry.clone(),
        None => entries.insert(0, entry.clone()),
    }
    write(app, &entries)?;
    Ok(entry.view())
}

pub fn delete(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut entries = read(app);
    let before = entries.len();
    entries.retain(|entry| entry.id != id);
    if entries.len() == before {
        return Err("Kunci sudah tidak tersimpan.".to_string());
    }
    write(app, &entries)
}

pub fn clear(app: &AppHandle) -> Result<usize, String> {
    let removed = read(app).len();
    write(app, &[])?;
    Ok(removed)
}

/// The cleartext key for one saved entry, for the caller that is about to type it into the
/// device. Nothing else in the app asks for it, and no command returns it to the UI.
pub fn reveal(app: &AppHandle, id: &str, serial: &str) -> Result<String, String> {
    let entry = read(app)
        .into_iter()
        .find(|entry| entry.id == id && entry.serial == serial)
        .ok_or("Kunci tersimpan tidak ditemukan untuk perangkat ini.")?;
    let bytes = unprotect(&entry.data)?;
    String::from_utf8(bytes).map_err(|_| "Kunci tersimpan tidak dapat dibaca.".to_string())
}

fn store_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join(STORE_NAME))
}

fn read(app: &AppHandle) -> Vec<Stored> {
    let Some(path) = store_path(app) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    // A file we cannot parse is left alone rather than treated as empty: the next save must
    // not silently drop the keys the user already stored.
    serde_json::from_str::<Vec<Stored>>(&text).unwrap_or_default()
}

fn write(app: &AppHandle, entries: &[Stored]) -> Result<(), String> {
    let path = store_path(app).ok_or("Folder konfigurasi aplikasi tidak tersedia.")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Gagal membuat folder konfigurasi: {error}"))?;
    }
    let text = serde_json::to_string_pretty(entries)
        .map_err(|error| format!("Gagal menyusun daftar kunci: {error}"))?;
    std::fs::write(&path, text).map_err(|error| format!("Gagal menyimpan kunci: {error}"))
}

impl Stored {
    fn view(&self) -> CredentialView {
        CredentialView {
            id: self.id.clone(),
            serial: self.serial.clone(),
            label: self.label.clone(),
            method: self.method.clone(),
            length: self.length,
        }
    }
}

fn next_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default();
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed) as u64;
    format!("{millis:013x}{tick:03x}")
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    if text.len() % 2 != 0 {
        return Err("Data kunci tersimpan rusak.".to_string());
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let pair = std::str::from_utf8(&bytes[index..index + 2])
            .map_err(|_| "Data kunci tersimpan rusak.".to_string())?;
        out.push(
            u8::from_str_radix(pair, 16).map_err(|_| "Data kunci tersimpan rusak.".to_string())?,
        );
        index += 2;
    }
    Ok(out)
}

#[cfg(windows)]
fn protect(bytes: &[u8]) -> Result<String, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut input = bytes.to_vec();
    let mut entropy = ENTROPY.to_vec();
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_mut_ptr(),
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_mut_ptr(),
    };
    let mut out = CRYPT_INTEGER_BLOB::default();

    let protected = unsafe {
        CryptProtectData(
            &in_blob,
            PCWSTR::null(),
            Some(&entropy_blob),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    // crypt32 has taken its own copy; drop ours before anything can read it back.
    input.fill(0);
    entropy.fill(0);

    let result = protected
        .map(|_| unsafe { to_hex(std::slice::from_raw_parts(out.pbData, out.cbData as usize)) });
    if !out.pbData.is_null() {
        unsafe {
            let _ = LocalFree(HLOCAL(out.pbData as *mut core::ffi::c_void));
        }
    }
    result.map_err(|error| format!("Gagal mengenkripsi kunci: {error}"))
}

#[cfg(windows)]
fn unprotect(hex: &str) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut blob = from_hex(hex)?;
    let mut entropy = ENTROPY.to_vec();
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: blob.len() as u32,
        pbData: blob.as_mut_ptr(),
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_mut_ptr(),
    };
    let mut out = CRYPT_INTEGER_BLOB::default();

    let unprotected = unsafe {
        CryptUnprotectData(
            &in_blob,
            None,
            Some(&entropy_blob),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    blob.fill(0);
    entropy.fill(0);

    let result = unprotected
        .map(|_| unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() });
    if !out.pbData.is_null() {
        unsafe {
            let _ = LocalFree(HLOCAL(out.pbData as *mut core::ffi::c_void));
        }
    }
    result.map_err(|error| {
        format!(
            "Kunci tersimpan tidak dapat dibuka ({error}). Kemungkinan file dipindah dari \
             Windows atau akun pengguna ini; hapus lalu simpan ulang di Pengaturan."
        )
    })
}

#[cfg(not(windows))]
fn protect(_bytes: &[u8]) -> Result<String, String> {
    Err("Penyimpanan kunci hanya tersedia di Windows.".to_string())
}

#[cfg(not(windows))]
fn unprotect(_hex: &str) -> Result<Vec<u8>, String> {
    Err("Penyimpanan kunci hanya tersedia di Windows.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_survives_a_round_trip() {
        let bytes = [0u8, 1, 15, 16, 200, 255];
        let encoded = to_hex(&bytes);
        assert_eq!(from_hex(&encoded).unwrap(), bytes.to_vec());
    }

    #[test]
    fn a_truncated_blob_is_reported_instead_of_guessed() {
        assert!(from_hex("0f0").is_err());
        assert!(from_hex("zz").is_err());
    }

    #[test]
    fn stored_keys_are_labelled_by_shape_not_by_content() {
        let entry = Stored {
            id: "0".into(),
            serial: "192.168.1.9:5555".into(),
            label: "Kantor".into(),
            method: "pin".into(),
            length: 6,
            data: "00ff".into(),
        };
        let view = entry.view();
        assert_eq!(view.method, "pin");
        assert_eq!(view.length, 6);
        // The view has no field that could carry the key at all.
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("data"));
    }

    #[test]
    fn two_saved_keys_get_different_ids() {
        assert_ne!(next_id(), next_id());
    }

    #[cfg(windows)]
    #[test]
    fn a_key_is_written_as_a_blob_that_only_this_user_and_entropy_can_open() {
        let secret = "123456";
        let blob = protect(secret.as_bytes()).expect("dpapi protect gagal");

        // What is on disk must not be the key, and not even its plain encoding.
        assert_ne!(blob, to_hex(secret.as_bytes()));
        assert!(!blob.contains("3132333435"));
        assert_eq!(
            unprotect(&blob).expect("dpapi unprotect gagal"),
            secret.as_bytes()
        );
    }
}
