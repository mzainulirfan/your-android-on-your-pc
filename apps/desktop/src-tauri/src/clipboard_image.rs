use crate::adb;
use crate::chat_target;
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

/// Progress note for the Ctrl+V image hand-off. The pipeline runs on its own thread (a push
/// over Wi-Fi takes seconds), so the toolbar learns about it as it happens: one `start`, then
/// exactly one `done` or `error`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteNote {
    pub phase: &'static str,
    pub text: String,
}

impl PasteNote {
    pub fn start() -> Self {
        Self {
            phase: "start",
            text: "Mengirim gambar dari PC…".to_string(),
        }
    }

    pub fn done(text: String) -> Self {
        Self { phase: "done", text }
    }

    pub fn error(text: String) -> Self {
        Self { phase: "error", text }
    }
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

/// The Ctrl+V path inside a mirror session. Android will not take a picture onto the phone's
/// clipboard from outside the app, so the paste gesture cannot be served through the clipboard
/// at all — it is served by handing the picture to the app the phone is looking at: the share
/// intent is aimed at the foreground package, and WhatsApp (or any chat app) opens its
/// send-to picker with the image already attached. If the foreground app cannot be read or
/// refuses the intent, the generic share sheet from the button path answers instead.
pub fn paste_into_foreground(app: &AppHandle, serial: &str) -> Result<String, String> {
    if !adb::is_valid_serial(serial) {
        return Err("Serial perangkat tidak valid.".to_string());
    }

    let capture = capture_png()?;
    let outcome = push_and_deliver(app, serial, &capture);
    let _ = std::fs::remove_file(&capture.path);
    outcome
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
    let share_opened = open_chooser(&adb, serial, &capture.file_name).is_ok();

    Ok(ImageSendResult {
        file_name: capture.file_name.clone(),
        share_opened,
    })
}

fn push_and_deliver(
    app: &AppHandle,
    serial: &str,
    capture: &Capture,
) -> Result<String, String> {
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
    if let Some(package) = foreground_package(serial) {
        if open_share(&adb, serial, &capture.file_name, &package).is_ok() {
            return deliver_to_chat(app, serial, &package);
        }
    }
    match open_chooser(&adb, serial, &capture.file_name) {
        Ok(()) => Ok("Gambar terkirim · pilih aplikasi di lembar Bagikan HP.".to_string()),
        // The copy survived in Download either way; say what Android actually answered
        // instead of only that the sheet failed (rule: errors must be actionable).
        Err(reason) => Err(format!("Gambar tersimpan di Download HP, tapi {reason}")),
    }
}

/// The app holds the image and its own destination list is on screen. Finish that list when YAOP
/// knows where the user sends to, and otherwise show the panel what the phone is offering so one
/// click becomes the saved destination for the next paste.
fn deliver_to_chat(app: &AppHandle, serial: &str, package: &str) -> Result<String, String> {
    let Some(label) = chat_target::lookup(app, serial, package) else {
        let rows = chat_target::visible_rows(serial, package);
        if rows.is_empty() {
            return Ok(format!(
                "Gambar terkirim ke {package} · pilih chat tujuannya di HP."
            ));
        }
        chat_target::emit_choices(app, serial, package, "new", &rows);
        return Ok(
            "Pilih chat tujuannya di panel · berikutnya YAOP menyelesaikan daftarnya sendiri."
                .to_string(),
        );
    };

    match chat_target::finish_picker(serial, package, &label) {
        chat_target::Finish::Sent => Ok(format!("Terkirim ke {label}.")),
        chat_target::Finish::NotFound(rows) => {
            chat_target::emit_choices(app, serial, package, "not-found", &rows);
            Ok(format!(
                "Chat \"{label}\" sedang tidak terlihat di HP · pilih tujuannya di panel."
            ))
        }
        chat_target::Finish::Ambiguous(rows) => {
            chat_target::emit_choices(app, serial, package, "ambiguous", &rows);
            Ok(format!(
                "Nama \"{label}\" muncul lebih dari sekali · pilih tujuannya di panel."
            ))
        }
        chat_target::Finish::NoRows => Ok(format!(
            "Gambar terkirim ke {package} · pilih chat tujuannya di HP."
        )),
        chat_target::Finish::Aborted(note) => Ok(format!("Gambar terkirim ke {package} · {note}")),
    }
}

/// The pipe filters on the phone (the full `dumpsys window` is a huge dump served by
/// system_server); the script is a constant with no caller data in it, and the serial stays
/// its own validated argument. Both markers are read because a chat with the keyboard open
/// focuses the input method's window — `mCurrentFocus` then names no app, while
/// `mFocusedApp` still does.
const FOCUS_PROBE: &str =
    "dumpsys window | grep -E 'mCurrentFocus=|mFocusedApp=' | head -4";

pub(crate) fn foreground_package(serial: &str) -> Option<String> {
    let text = adb::shell(serial, &[FOCUS_PROBE]).ok()?;
    current_focus_package(&text)
}

/// The front window's own identity, e.g. `2bc1234 u0 com.whatsapp/com.whatsapp.Main`. A window
/// keeps its hash while it is alive, so a caller can tell "the screen I opened is still there"
/// from "the phone moved on" without knowing anything about the app.
pub(crate) fn focus_window(serial: &str) -> Option<String> {
    let text = adb::shell(serial, &[FOCUS_PROBE]).ok()?;
    current_focus_window(&text)
}

fn current_focus_window(text: &str) -> Option<String> {
    let body = text.split("mCurrentFocus=Window{").nth(1)?;
    let body = body.split('}').next()?.trim();
    (!body.is_empty() && body != "null").then(|| body.to_string())
}

/// `mCurrentFocus=Window{<hash> u0 com.whatsapp/com.whatsapp.Main}` or
/// `mFocusedApp=ActivityRecord{<hash> u0 com.whatsapp/.Main}` → `com.whatsapp`.
/// Dialogs, `null`, `InputMethod` and launcher placeholders simply do not yield a safe
/// package, and the caller then falls back to the chooser.
fn current_focus_package(text: &str) -> Option<String> {
    for line in text.lines() {
        for marker in ["mCurrentFocus=Window{", "mFocusedApp=ActivityRecord{"] {
            let Some(rest) = line.split_once(marker) else {
                continue;
            };
            let package = rest
                .1
                .split('}')
                .next()
                .into_iter()
                .flat_map(|body| body.split_whitespace())
                .find_map(|token| {
                    let (package, _) = token.split_once('/')?;
                    is_safe_package(package).then(|| package.to_string())
                });
            if package.is_some() {
                return package;
            }
        }
    }
    None
}

/// The package is read off the device and travels as its own `am start -p` argument; keeping
/// it to the characters a real package name uses means a hostile or malformed dump cannot
/// smuggle an option into the command line.
pub(crate) fn is_safe_package(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.contains('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The Android 10+ media store no longer takes a pushed file into `content://media` from the
/// `MEDIA_SCANNER_SCAN_FILE` broadcast: on the test device (Android 16) the file never
/// appeared, and each `content query` cost 1.3–5.5 s, so the old six-round poll spent over
/// half a minute on a URI it never found. The share intent is handed the pushed file directly
/// instead, and the copy stays in Download either way.
///
/// The implicit form of this intent is deliberately not offered: on the test HyperOS device
/// `SEND image/png` with no package has a "best match" (the built-in scanner), so it launches
/// that app straight and the share sheet never appears. Callers either name a package here or
/// go through `open_chooser`.
fn open_share(
    adb: &std::path::Path,
    serial: &str,
    file_name: &str,
    package: &str,
) -> Result<(), String> {
    let uri = format!("file://{REMOTE_DIR}{file_name}");
    let mut command = Command::new(adb);
    command
        .args(["-s", serial, "shell", "am", "start"])
        .args(["-p", package])
        .args(["-a", "android.intent.action.SEND", "-t", "image/png"])
        .args(["--eu", "android.intent.extra.STREAM", &uri])
        .arg("--grant-read-uri-permission");
    run_am(&mut command)
}

/// The share sheet as an explicit `CHOOSER` around the send intent. Unlike the implicit send,
/// this can never be swallowed by a device-set default: the list always opens, with the
/// phone's direct-share contacts on top (verified on the test device). The inner intent
/// carries the URI-grant flag itself (`launchFlags=0x1`) because the resolver forwards that
/// one, not the chooser's own flags.
///
/// The nested URI is handed to the device shell inside literal single quotes: adb joins every
/// argument after `shell` into one command line that the phone's shell re-parses, and the
/// `;` separators of `intent:#Intent;...;end` would otherwise split the command apart. The
/// file name is already restricted by `is_safe_name`, so no quote can be smuggled in.
fn open_chooser(adb: &std::path::Path, serial: &str, file_name: &str) -> Result<(), String> {
    let nested = format!(
        "intent:#Intent;action=android.intent.action.SEND;type=image/png;launchFlags=0x1;S.android.intent.extra.STREAM=file://{REMOTE_DIR}{file_name};end"
    );
    let quoted = format!("'{nested}'");
    let mut command = Command::new(adb);
    command
        .args(["-s", serial, "shell", "am", "start"])
        .args(["-a", "android.intent.action.CHOOSER"])
        .args(["--eu", "android.intent.extra.INTENT", &quoted])
        .arg("--grant-read-uri-permission");
    run_am(&mut command)
}

fn run_am(command: &mut Command) -> Result<(), String> {
    #[cfg(windows)]
    let command = command.creation_flags(CREATE_NO_WINDOW);

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
    // `am start` exits 0 even when it refuses to launch — an intent that cannot be served
    // answers with "Error: ..." on the output streams, and the caller needs that to fall
    // back from the targeted delivery to the chooser.
    let streams = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if streams.contains("Error") {
        return Err(format!(
            "Android menolak membuka lembar Bagikan: {}",
            streams.trim()
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

    #[test]
    fn focus_line_names_the_foreground_package() {
        assert_eq!(
            current_focus_package(
                "    mCurrentFocus=Window{2bc1234 u0 com.whatsapp/com.whatsapp.Main}"
            )
            .as_deref(),
            Some("com.whatsapp")
        );
        // No window in focus, or a token that is not a package, yields nothing and the
        // caller falls back to the generic sheet.
        assert_eq!(current_focus_package("  mCurrentFocus=null"), None);
        assert_eq!(
            current_focus_package("mCurrentFocus=Window{a b null/null}"),
            None
        );
        assert_eq!(
            current_focus_package("mCurrentFocus=Window{1 u0 com.android.server.am}"),
            None
        );
    }

    #[test]
    fn focused_app_answers_when_the_keyboard_holds_the_window() {
        // A chat with the soft keyboard open focuses the input method's window; the app that
        // owns the session is still named by the second marker.
        let dump = "  mCurrentFocus=Window{6f2a8b4 u0 InputMethod}\n  mFocusedApp=ActivityRecord{1c2d3e4 u0 com.whatsapp/com.whatsapp.Main}";
        assert_eq!(
            current_focus_package(dump).as_deref(),
            Some("com.whatsapp")
        );
        // An InputMethod window with no focused activity yields nothing → chooser fallback.
        assert_eq!(
            current_focus_package("  mCurrentFocus=Window{6f2a8b4 u0 InputMethod}"),
            None
        );
    }

    #[test]
    fn focus_window_names_the_screen_and_survives_it() {
        let line = "    mCurrentFocus=Window{2bc1234 u0 com.whatsapp/com.whatsapp.contact.ui.picker.ExternalShareAlias}";
        assert_eq!(
            current_focus_window(line).as_deref(),
            Some("2bc1234 u0 com.whatsapp/com.whatsapp.contact.ui.picker.ExternalShareAlias")
        );
        // The same window answers with the same string, and no window answers with nothing.
        assert_eq!(current_focus_window(line), current_focus_window(line));
        assert_eq!(current_focus_window("  mCurrentFocus=null"), None);
        assert_eq!(current_focus_window("nothing here"), None);
    }

    #[test]
    fn package_names_are_bounded_plain_strings() {
        assert!(is_safe_package("com.whatsapp"));
        assert!(is_safe_package("org.telegram.messenger-web"));
        assert!(!is_safe_package("com.whatsapp; rm -rf /"));
        assert!(!is_safe_package("--user=0"));
        assert!(!is_safe_package("nodots"));
        assert!(!is_safe_package(""));
        assert!(!is_safe_package(&"a.".repeat(70)));
    }
}
