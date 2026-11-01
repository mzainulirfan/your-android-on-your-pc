use crate::adb;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocateReport {
    pub action: String,
    pub status: String,
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portal_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portal_label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lens {
    Back,
    Front,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureMode {
    Photo,
    Video,
}

/// Google Find My Device covers every Play-certified phone (including Xiaomi/HyperOS),
/// so the finder is a single known portal rather than per-vendor mapping.
const FIND_DEVICE_URL: &str = "https://www.google.com/android/find";
const FIND_DEVICE_LABEL: &str = "Google Find My Device";

#[derive(Default)]
struct Steps {
    done: usize,
    failed: usize,
}

impl Steps {
    fn record(&mut self, result: Result<String, String>, label: &str, notes: &mut Vec<String>) {
        self.done += 1;
        match result {
            Ok(output) => {
                let detail = output.trim().split('\n').next().unwrap_or_default();
                notes.push(if detail.is_empty() {
                    format!("{label}: ok.")
                } else {
                    format!("{label}: {detail}")
                });
            }
            Err(error) => {
                self.failed += 1;
                notes.push(format!("{label} gagal: {error}"));
            }
        }
    }

    fn all_failed(&self) -> bool {
        self.done > 0 && self.failed == self.done
    }

    fn status(&self) -> &'static str {
        if self.failed == 0 {
            "ok"
        } else {
            "partial"
        }
    }
}

/// Every locate action is a sequence of shell steps, so the two "phone is gone" answers are
/// translated once here instead of at each call.
fn shell(serial: &str, args: &[&str]) -> Result<String, String> {
    adb::shell(serial, args).map_err(|error| adb::explain_device_error(serial, &error))
}

/// The device only rings or shows a camera preview while its screen is interactive,
/// so waking and dismissing the keyguard is a prerequisite for every locate action.
fn wake_and_unlock(serial: &str, notes: &mut Vec<String>) {
    if let Err(error) = shell(serial, &["input", "keyevent", "KEYCODE_WAKEUP"]) {
        notes.push(format!("Layar gagal dibangunkan: {error}"));
        return;
    }

    std::thread::sleep(std::time::Duration::from_millis(700));
    match shell(serial, &["wm", "dismiss-keyguard"]) {
        Ok(_) => notes.push("Layar dibangunkan.".to_string()),
        Err(error) => notes.push(format!(
            "Layar dibangunkan, keyguard tidak tertutup: {error}"
        )),
    }
}

/// Shell ADB cannot reliably force every OEM to play its own alarm tone (this
/// Xiaomi drops the `media` command entirely and the shell user lacks the `audio`
/// group), so "ring" hands off to the phone's official Find My Device service: we
/// wake the screen, normalize the ringer, and return the vendor portal so the user
/// triggers the siren from their browser.
pub fn find_device(serial: &str) -> Result<LocateReport, String> {
    adb::validate_serial(serial)?;
    let mut notes = Vec::new();
    let mut steps = Steps::default();

    wake_and_unlock(serial, &mut notes);

    steps.record(
        shell(serial, &["cmd", "audio", "set-ringer-mode", "NORMAL"]),
        "Mode dering dinormalkan",
        &mut notes,
    );

    notes.push(
        "Layar HP sudah dibangunkan. Untuk membunyikannya, buka Google Find My Device lalu \
         ketuk \"Putar Suara\"."
            .to_string(),
    );

    Ok(LocateReport {
        action: "find-device".into(),
        status: steps.status().into(),
        notes,
        portal_url: Some(FIND_DEVICE_URL.into()),
        portal_label: Some(FIND_DEVICE_LABEL.into()),
    })
}

pub fn open_camera(serial: &str, lens: Lens, mode: CaptureMode) -> Result<LocateReport, String> {
    adb::validate_serial(serial)?;

    let mut notes = Vec::new();
    wake_and_unlock(serial, &mut notes);

    let args = camera_intent(lens, mode);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    shell(serial, &args).map_err(|error| format!("Kamera gagal dibuka: {error}"))?;

    notes.push(format!(
        "Kamera {} ({}) dibuka di HP; lihat jendela mirroring.",
        match lens {
            Lens::Back => "belakang",
            Lens::Front => "depan",
        },
        match mode {
            CaptureMode::Photo => "foto",
            CaptureMode::Video => "video",
        }
    ));

    Ok(LocateReport {
        action: "camera-open".into(),
        status: "ok".into(),
        notes,
        portal_url: None,
        portal_label: None,
    })
}

pub fn close_camera(serial: &str) -> Result<LocateReport, String> {
    adb::validate_serial(serial)?;

    let mut notes = Vec::new();
    let mut steps = Steps::default();

    for package in camera_packages() {
        steps.record(
            shell(serial, &["am", "force-stop", package]),
            &format!("{package} ditutup"),
            &mut notes,
        );
    }

    if steps.all_failed() {
        return Err(format!("Kamera gagal ditutup. {}", notes.join(" | ")));
    }

    Ok(LocateReport {
        action: "camera-close".into(),
        status: steps.status().into(),
        notes,
        portal_url: None,
        portal_label: None,
    })
}

fn camera_packages() -> [&'static str; 2] {
    ["com.android.camera", "com.sec.android.app.camera"]
}

fn camera_intent(lens: Lens, mode: CaptureMode) -> Vec<String> {
    let action = match mode {
        CaptureMode::Photo => "android.media.action.STILL_IMAGE_CAMERA",
        CaptureMode::Video => "android.media.action.VIDEO_CAMERA",
    };
    let mut intent = vec![
        "am".to_string(),
        "start-activity".to_string(),
        "-a".to_string(),
        action.to_string(),
        "-f".to_string(),
        "0x10000000".to_string(),
    ];
    if matches!(lens, Lens::Front) {
        intent.extend_from_slice(&[
            "--ez".to_string(),
            "android.intent.extra.USE_FRONT_CAMERA".to_string(),
            "true".to_string(),
        ]);
    }
    intent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_camera_uses_photo_action_only() {
        let intent = camera_intent(Lens::Back, CaptureMode::Photo).join(" ");
        assert!(intent.contains("android.media.action.STILL_IMAGE_CAMERA"));
        assert!(!intent.contains("USE_FRONT_CAMERA"));
    }

    #[test]
    fn front_camera_adds_front_extra_and_video_uses_video_action() {
        let intent = camera_intent(Lens::Front, CaptureMode::Video).join(" ");
        assert!(intent.contains("android.media.action.VIDEO_CAMERA"));
        assert!(intent.contains("USE_FRONT_CAMERA true"));
    }

    #[test]
    fn steps_report_partial_when_one_command_fails() {
        let mut steps = Steps::default();
        let mut notes = Vec::new();
        steps.record(Ok("ok".into()), "satu", &mut notes);
        steps.record(Err("gagal".into()), "dua", &mut notes);
        steps.record(Ok(String::new()), "tiga", &mut notes);

        assert!(!steps.all_failed());
        assert_eq!(steps.status(), "partial");
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[1], "dua gagal: gagal");
        assert_eq!(notes[2], "tiga: ok.");
    }

    #[test]
    fn steps_report_all_failed() {
        let mut steps = Steps::default();
        let mut notes = Vec::new();
        steps.record(Err("a".into()), "satu", &mut notes);
        assert!(steps.all_failed());
    }
}
