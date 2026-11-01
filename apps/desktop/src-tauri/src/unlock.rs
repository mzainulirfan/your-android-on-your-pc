use crate::adb;
use serde::Serialize;
use std::thread::sleep;
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockReport {
    pub status: String,
    pub method: String,
    pub length: usize,
    pub unlocked: bool,
    pub notes: Vec<String>,
}

/// Unlock the device keyguard by injecting the credential through ADB `input`.
///
/// `FLAG_SECURE` only hides the lock screen from the mirrored surface; it does not stop an
/// injected key event from being delivered. Since Android 14, though, the keyguard's text
/// field rejects *synthetic* injection, so a digit typed this way can be dropped by the
/// system even though the command reports success — typing into the lock screen belongs in
/// the mirror window, where the session runs a real HID keyboard (`--keyboard=uhid`).
/// The credential is only passed through to the device; the report carries its length, never
/// its content, and nothing here logs it. A key the user chose to save is decrypted by
/// `crate::credentials` for the duration of this call and dropped straight after.
/// The `unlocked` verdict comes from polling the keyguard state after submission, which only
/// reads a boolean, never the protected pixels.
///
/// A PIN is typed inside the same shell session that wakes the display and opens the
/// bouncer. Splitting those into separate ADB calls put a round trip in front of every
/// digit, and the lock screen's screen-off timer kept running the whole time: the first
/// digit could land after the display had already slept again. The waits run on the phone,
/// so the whole unlock costs one spawn and the verdict arrives with the typed answer.
pub fn unlock(serial: &str, secret: &str) -> Result<UnlockReport, String> {
    let secret = secret.trim_end_matches(['\r', '\n']);
    if secret.is_empty() {
        return Err("Kunci tidak boleh kosong.".to_string());
    }

    let is_pin = secret.chars().all(|c| c.is_ascii_digit());

    let unlocked = if is_pin {
        let script = pin_script(secret);
        let text = adb::shell(serial, &[&script])
            .map_err(|error| format!("Gagal mengirim PIN: {error}"))?;
        if adb::keyguard_flag(&text) == Some(false) {
            true
        } else {
            wait_for_unlock(serial)
        }
    } else {
        if secret.chars().any(|c| c.is_control()) {
            return Err(
                "Password mengandung karakter kontrol yang tidak bisa dikirim.".to_string(),
            );
        }
        let quoted = format!("'{}'", secret.replace('\'', "'\\''"));
        adb::shell(serial, &[adb::WAKE_UP_COMMAND])
            .map_err(|error| format!("Gagal membangunkan layar: {error}"))?;
        adb::shell(serial, &["input", "text", &quoted])
            .map_err(|error| format!("Gagal mengirim password: {error}"))?;
        sleep(Duration::from_millis(120));
        adb::shell(serial, &["input", "keyevent", "66"])
            .map_err(|error| format!("Gagal mengirim Enter: {error}"))?;
        wait_for_unlock(serial)
    };

    let mut notes = Vec::new();
    if unlocked {
        notes.push(format!(
            "HP terbuka — {} {} karakter diterima.",
            if is_pin { "PIN" } else { "Password" },
            secret.chars().count()
        ));
    } else {
        notes.push(
            "Masih terkunci setelah Enter. Kemungkinan kunci salah, atau HP memakai pola/sidik \
             jari yang harus dieksekusi manual di HP."
                .to_string(),
        );
    }

    Ok(UnlockReport {
        status: if unlocked { "unlocked" } else { "locked" }.to_string(),
        method: if is_pin {
            "pin".to_string()
        } else {
            "text".to_string()
        },
        length: secret.chars().count(),
        unlocked,
        notes,
    })
}

/// Wake, open the bouncer, type the digits, press Enter and read the keyguard — one shell
/// session, with the waits on the phone. The input is validated ASCII digits, so the only
/// text that reaches the device is the generated numeric keycodes.
fn pin_script(secret: &str) -> String {
    let mut script = String::from(adb::WAKE_UP_COMMAND);
    for c in secret.chars() {
        let code = 7 + c.to_digit(10).unwrap_or(0);
        script.push_str(&format!("; input keyevent {code}; sleep 0.07"));
    }
    script.push_str("; sleep 0.12; input keyevent 66; sleep 0.8; ");
    script.push_str(adb::KEYGUARD_BLOCK_PROBE);
    script
}

fn wait_for_unlock(serial: &str) -> bool {
    for _ in 0..6 {
        sleep(Duration::from_millis(400));
        if !adb::is_keyguard_showing(serial) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_script_wakes_types_and_settles_in_one_session() {
        let script = pin_script("1234");
        assert!(script.starts_with("input keyevent KEYCODE_WAKEUP"));
        assert!(script.contains("wm dismiss-keyguard"));
        let positions: Vec<usize> = ["8", "9", "10", "11"]
            .iter()
            .map(|code| {
                script
                    .find(&format!("input keyevent {code};"))
                    .unwrap_or_else(|| panic!("keycode {code} hilang dari skrip"))
            })
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(script.contains("input keyevent 66; sleep 0.8; "));
        assert!(script.ends_with(adb::KEYGUARD_BLOCK_PROBE));
    }

    #[test]
    fn pin_script_only_carries_numeric_keycodes() {
        let script = pin_script("059");
        assert!(!script.contains("059"));
        assert!(script.contains("input keyevent 7;"));
        assert!(script.contains("input keyevent 12;"));
    }
}
