mod adb;
mod chat_target;
mod clipboard_image;
mod commands;
mod credentials;
mod files;
mod host;
mod locate;
mod mirror;
mod screenshot;
mod session;
mod settings;
mod transfer;
mod unlock;

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicesSnapshot {
    pub devices: Vec<adb::AndroidDevice>,
    pub error: Option<String>,
}

/// A failed scan is usually one missed round trip (`adb` timeout, server restart) rather than
/// a phone that left. Reporting an empty list for that single pass made the Files page drop
/// its device and reload the folder moments later, so the previous answer's devices are kept
/// and the error rides alongside them.
pub(crate) fn snapshot_or_previous(
    previous: Option<&DevicesSnapshot>,
    result: Result<Vec<adb::AndroidDevice>, String>,
) -> DevicesSnapshot {
    match result {
        Ok(devices) => DevicesSnapshot {
            devices,
            error: None,
        },
        Err(error) => DevicesSnapshot {
            devices: previous
                .map(|snapshot| snapshot.devices.clone())
                .unwrap_or_default(),
            error: Some(error),
        },
    }
}

/// The poller's last answer. The page asks for this on mount instead of running its own
/// `adb devices` beside the poller's, and a window that opened after the change event still
/// gets the current list instead of waiting for the next device change.
#[derive(Default)]
pub struct DevicesCache(pub Mutex<Option<DevicesSnapshot>>);

/// The last background Wi-Fi reconnect result, shared between the reconnect worker (which may
/// spend its full timeout on an address) and the poller that turns it into the explanation
/// shown while no device is listed.
#[derive(Default)]
pub struct ReconnectNote(pub Mutex<Option<String>>);

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .manage(session::SessionManager::default())
        .manage(DevicesCache::default())
        .manage(ReconnectNote::default())
        .manage(mirror::MirrorTracker::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_adb_status,
            commands::get_devices_snapshot,
            commands::enable_wireless,
            commands::connect_wireless,
            commands::pair_wireless,
            commands::get_known_wireless,
            commands::forget_wireless,
            commands::set_screen_power,
            commands::scrcpy_screen_power,
            commands::open_find_device,
            commands::unlock_device,
            commands::list_credentials,
            commands::get_all_credentials,
            commands::save_credential,
            commands::delete_credential,
            commands::clear_credentials,
            commands::unlock_with_credential,
            commands::screen_state,
            commands::wake_state,
            commands::keyguard_locked,
            commands::set_mirror_rect,
            commands::get_mirror_aspect,
            commands::set_window_width,
            commands::animate_window_width,
            commands::fit_window_height,
            commands::set_keyboard_owner,
            commands::open_url,
            commands::open_device_camera,
            commands::close_device_camera,
            commands::push_files,
            commands::pull_android_file,
            commands::list_android_files,
            commands::capture_screenshot,
            commands::send_clipboard_image,
            commands::pick_chat_target,
            commands::get_all_chat_targets,
            commands::remove_chat_target,
            commands::get_scrcpy_status,
            commands::get_session_status,
            commands::start_session,
            commands::stop_session,
            commands::get_settings,
            commands::save_settings,
            commands::get_theme,
            commands::set_theme,
            commands::get_diagnostics,
            commands::open_download_dir
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    // The app window hosts the embedded mirror, so closing it would take the
                    // running session down with it. Treat the X (or Alt+F4) as "hide to the
                    // tray"; disconnecting belongs to "Putuskan" on Home/Devices.
                    "main" => {
                        if settings::load(&window.app_handle()).close_to_tray {
                            api.prevent_close();
                            let _ = window.hide();
                        }
                    }
                    _ => {}
                }
            }
        })
        .setup(|app| {
            // Paint the native frame with the stored theme before the webview shows anything;
            // the page itself applies the CSS palette from localStorage even earlier.
            if let Some(window) = app.get_webview_window("main") {
                let theme = settings::load(app.handle()).theme;
                let native = if theme == "light" {
                    tauri::Theme::Light
                } else {
                    tauri::Theme::Dark
                };
                let _ = window.set_theme(Some(native));
            }

            let show_item =
                MenuItem::with_id(app, "show", "Tampilkan jendela", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Keluar", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("Your Android on Your PC")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick { .. } = event {
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            let _tray = tray.build(app)?;

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut last: Option<String> = None;

                loop {
                    // Every pass spawns adb.exe and talks to adbd on the phone, and scrcpy's
                    // video stream uses that same connection: while a mirror session runs, the
                    // idle 2 s cadence showed up as regular hitches in the picture. Nothing on
                    // the main window can show a change while it sits in the tray either, so
                    // the idle pass is slowed down there instead of waking ADB every 2 s.
                    let mirroring = handle.state::<session::SessionManager>().info().is_some();
                    let visible = handle
                        .get_webview_window("main")
                        .and_then(|window| window.is_visible().ok())
                        .unwrap_or(true);
                    let cadence = if mirroring {
                        Duration::from_secs(10)
                    } else if visible {
                        Duration::from_secs(2)
                    } else {
                        Duration::from_secs(15)
                    };

                    let previous = handle
                        .state::<DevicesCache>()
                        .0
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone());
                    let mut snapshot =
                        snapshot_or_previous(previous.as_ref(), adb::list_devices());

                    if snapshot.error.is_none() && snapshot.devices.is_empty() {
                        let note = handle
                            .state::<ReconnectNote>()
                            .0
                            .lock()
                            .ok()
                            .and_then(|guard| guard.clone());
                        snapshot.error = note.map(|message| {
                            format!(
                                "Perangkat Wi-Fi {message}. HP mungkin mati atau mode tcpip-nya \
                                 reset; sambungkan USB lalu tekan Aktifkan Wi-Fi, atau hubungkan \
                                 manual dengan IP HP."
                            )
                        });
                    }

                    if let Ok(mut guard) = handle.state::<DevicesCache>().0.lock() {
                        *guard = Some(snapshot.clone());
                    }

                    let encoded = serde_json::to_string(&snapshot).unwrap_or_default();
                    if last.as_deref() != Some(encoded.as_str()) {
                        last = Some(encoded);
                        let _ = handle.emit("devices-changed", &snapshot);
                    }

                    std::thread::sleep(cadence);
                }
            });

            // A wireless phone vanishes from `adb devices` whenever the ADB server or the
            // phone restarts, because the TCP connection is not kept anywhere, so every known
            // address that is missing is retried. One `adb connect` can spend its whole timeout
            // (4 s) on a host that is gone; running that here keeps the list poll — and the
            // Refresh button — responsive in the meantime.
            let reconnect = app.handle().clone();
            std::thread::spawn(move || {
                let mut cursor = 0usize;
                // Give the first list poll a moment to publish the devices, then sweep every
                // 20 s like before.
                let mut wait = Duration::from_secs(5);
                loop {
                    std::thread::sleep(wait);
                    wait = Duration::from_secs(20);

                    if reconnect
                        .state::<session::SessionManager>()
                        .info()
                        .is_some()
                    {
                        continue;
                    }
                    let devices = reconnect
                        .state::<DevicesCache>()
                        .0
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone())
                        .map(|snapshot| snapshot.devices)
                        .unwrap_or_default();
                    let missing: Vec<String> = settings::known_wireless(&reconnect)
                        .into_iter()
                        .filter(|address| {
                            !devices.iter().any(|device| {
                                device.serial == *address && device.state == "device"
                            })
                        })
                        .collect();
                    let Some(address) = missing.get(cursor % missing.len().max(1)) else {
                        if let Ok(mut note) = reconnect.state::<ReconnectNote>().0.lock() {
                            *note = None;
                        }
                        continue;
                    };

                    let message = adb::try_reconnect(address);
                    cursor += usize::from(message.is_some());
                    if let Ok(mut note) = reconnect.state::<ReconnectNote>().0.lock() {
                        *note = message;
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("gagal membangun aplikasi Tauri");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            handle.state::<session::SessionManager>().shutdown();
            mirror::stop(handle);
            host::end();
        }
    });
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        // The mirror is a child of this window now, so revealing it reveals the video too.
        // `show()` alone does not undo a minimize or Win+D — Tauri maps the window where it
        // sits, which is why the frame used to look "gone" from the tray; restore first.
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(serial: &str) -> adb::AndroidDevice {
        adb::AndroidDevice {
            serial: serial.to_string(),
            state: "device".to_string(),
            connection_type: "wifi".to_string(),
            model: None,
            product: None,
            device_codename: None,
            transport_id: None,
        }
    }

    #[test]
    fn a_failed_scan_keeps_the_previous_devices() {
        let previous = DevicesSnapshot {
            devices: vec![device("192.168.1.3:5555")],
            error: None,
        };

        let snapshot = snapshot_or_previous(
            Some(&previous),
            Err("ADB tidak merespons dalam 20 detik.".to_string()),
        );
        assert_eq!(snapshot.devices.len(), 1);
        assert_eq!(snapshot.devices[0].serial, "192.168.1.3:5555");
        assert!(snapshot.error.is_some());
    }

    #[test]
    fn an_empty_answer_is_a_real_disconnect() {
        let previous = DevicesSnapshot {
            devices: vec![device("192.168.1.3:5555")],
            error: None,
        };

        let snapshot = snapshot_or_previous(Some(&previous), Ok(Vec::new()));
        assert!(snapshot.devices.is_empty());
        assert!(snapshot.error.is_none());
    }

    #[test]
    fn a_failed_first_scan_has_nothing_to_keep() {
        let snapshot = snapshot_or_previous(None, Err("adb gagal".to_string()));
        assert!(snapshot.devices.is_empty());
        assert_eq!(snapshot.error.as_deref(), Some("adb gagal"));
    }
}
