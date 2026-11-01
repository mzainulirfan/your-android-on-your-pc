import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import type {
  AdbStatus,
  AndroidEntry,
  DevicesSnapshot,
  Diagnostics,
  ImageSendResult,
  Lens,
  CaptureMode,
  LocateReport,
  PullResult,
  SavedCredential,
  ScrcpyStatus,
  ScreenshotResult,
  SessionError,
  SessionExit,
  SessionInfo,
  Settings,
  TransferProgress,
  TransferResult,
  UnlockReport,
  WakeState,
  WirelessInfo,
} from './types';

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function getAdbStatus(): Promise<AdbStatus> {
  if (!isTauri) {
    return {
      found: false,
      error: 'UI berjalan di browser. Jalankan `npm run tauri dev` untuk mengakses ADB.',
    };
  }
  return invoke<AdbStatus>('get_adb_status');
}

/**
 * The poller's device snapshot. Mount passes `fresh = false` so it does not run a second
 * `adb devices` beside the background poll; an explicit refresh passes `true`.
 */
export async function getDevicesSnapshot(fresh = false): Promise<DevicesSnapshot> {
  if (!isTauri) return { devices: [] };
  return invoke<DevicesSnapshot>('get_devices_snapshot', { fresh });
}

export async function getScrcpyStatus(): Promise<ScrcpyStatus> {
  if (!isTauri) {
    return {
      found: false,
      error: 'UI berjalan di browser. Jalankan `npm run tauri dev` untuk mengakses scrcpy.',
    };
  }
  return invoke<ScrcpyStatus>('get_scrcpy_status');
}

export async function getSessionStatus(): Promise<SessionInfo | null> {
  if (!isTauri) return null;
  return invoke<SessionInfo | null>('get_session_status');
}

export async function startSession(serial: string): Promise<SessionInfo> {
  if (!isTauri) throw 'Sesi hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<SessionInfo>('start_session', { serial });
}

export async function stopSession(): Promise<void> {
  if (!isTauri) throw 'Sesi hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<void>('stop_session');
}

export async function enableWireless(serial: string): Promise<WirelessInfo> {
  if (!isTauri) throw 'Wi-Fi hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<WirelessInfo>('enable_wireless', { serial });
}

export async function connectWireless(address: string): Promise<string> {
  if (!isTauri) throw 'Koneksi Wi-Fi hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<string>('connect_wireless', { address });
}

/** Pair with the phone's Wireless debugging dialog (Android 11+): ip:port from the dialog + 6-digit code. */
export async function pairWireless(address: string, code: string): Promise<string> {
  if (!isTauri) throw 'Pairing Wi-Fi hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<string>('pair_wireless', { address, code });
}

export async function forgetWireless(address: string): Promise<void> {
  if (!isTauri) return;
  await invoke('forget_wireless', { address });
}

export async function getKnownWireless(): Promise<string[]> {
  if (!isTauri) return [];
  return invoke<string[]>('get_known_wireless');
}

/**
 * Sleep or wake the physical display and get the resulting state back. The wake path runs on
 * the phone in one shell session, so the caller can label the result without another read.
 */
export async function setScreenPower(serial: string, sleep: boolean): Promise<WakeState> {
  if (!isTauri) throw 'Kontrol layar hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<WakeState>('set_screen_power', { serial, sleep });
}

export async function pushFiles(
  serial: string,
  paths: string[],
  targetDir?: string,
): Promise<TransferResult> {
  if (!isTauri) throw 'Transfer hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<TransferResult>('push_files', { serial, paths, targetDir: targetDir ?? null });
}

export async function pullAndroidFile(serial: string, remotePath: string): Promise<PullResult> {
  if (!isTauri) throw 'Unduhan hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<PullResult>('pull_android_file', { serial, remotePath });
}

export async function listAndroidFiles(serial: string, path: string): Promise<AndroidEntry[]> {
  if (!isTauri) return [];
  return invoke<AndroidEntry[]>('list_android_files', { serial, path });
}

export async function captureScreenshot(serial: string): Promise<ScreenshotResult> {
  if (!isTauri) throw 'Screenshot hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<ScreenshotResult>('capture_screenshot', { serial });
}

export async function getSettings(): Promise<Settings> {
  if (!isTauri) {
    // Mirrors Settings::default() on the Rust side.
    return {
      maxSize: 1920,
      videoBitrateKbps: 0,
      maxFps: 60,
      downloadDir: '',
      closeToTray: true,
      mirrorScreenOff: false,
      lowLatency: false,
      audioEnabled: true,
    };
  }
  return invoke<Settings>('get_settings');
}

export async function saveSettings(settings: Settings): Promise<Settings> {
  if (!isTauri) throw 'Pengaturan hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<Settings>('save_settings', { settings });
}

export async function getDiagnostics(deep = false): Promise<Diagnostics> {
  if (!isTauri) throw 'Diagnostics hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<Diagnostics>('get_diagnostics', { deep });
}

export async function openDownloadDir(): Promise<void> {
  if (!isTauri) throw 'Folder hanya dapat dibuka melalui aplikasi Tauri.';
  return invoke<void>('open_download_dir');
}

export async function sendClipboardImage(serial: string): Promise<ImageSendResult> {
  if (!isTauri) throw 'Kirim gambar hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<ImageSendResult>('send_clipboard_image', { serial });
}

export async function openFindDevice(serial: string): Promise<LocateReport> {
  if (!isTauri) throw 'Pelacakan hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<LocateReport>('open_find_device', { serial });
}

export async function unlockDevice(serial: string, secret: string): Promise<UnlockReport> {
  if (!isTauri) throw 'Buka kunci hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<UnlockReport>('unlock_device', { serial, secret });
}

/** Saved keys for one device. The list carries labels and lengths only — never a key. */
export async function listCredentials(serial: string): Promise<SavedCredential[]> {
  if (!isTauri) return [];
  return invoke<SavedCredential[]>('list_credentials', { serial });
}

export async function getAllCredentials(): Promise<SavedCredential[]> {
  if (!isTauri) return [];
  return invoke<SavedCredential[]>('get_all_credentials');
}

export async function saveCredential(
  serial: string,
  label: string,
  secret: string,
): Promise<SavedCredential> {
  if (!isTauri) throw 'Penyimpanan kunci hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<SavedCredential>('save_credential', { serial, label, secret });
}

export async function deleteCredential(id: string): Promise<void> {
  if (!isTauri) return;
  await invoke('delete_credential', { id });
}

/** Removes every stored key on this PC. Returns how many were deleted. */
export async function clearCredentials(): Promise<number> {
  if (!isTauri) return 0;
  return invoke<number>('clear_credentials');
}

/**
 * Send a stored key by its id. The cleartext never crosses into the interface: Rust reads
 * it on a worker thread and types it straight into the device.
 */
export async function unlockWithCredential(
  serial: string,
  id: string,
): Promise<UnlockReport> {
  if (!isTauri) throw 'Buka kunci tersimpan hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<UnlockReport>('unlock_with_credential', { serial, id });
}

export async function screenState(serial: string): Promise<boolean> {
  if (!isTauri) return true;
  return invoke<boolean>('screen_state', { serial });
}

/**
 * Screen and lock state in one device read, for the poll that runs during a mirror session.
 * Two separate reads land on the phone at the same time and `dumpsys` is served by
 * system_server — the process that also drives the animation on screen.
 */
export async function wakeState(serial: string): Promise<WakeState> {
  if (!isTauri) return { screenOn: true, keyguardUp: false };
  return invoke<WakeState>('wake_state', { serial });
}

/**
 * Only the keyguard flag. The panel polls this while it shows the credential form: the phone
 * can be unlocked outside the panel (a PIN typed into the mirror, or on the device itself)
 * and the form has to fold away by itself when that happens.
 */
export async function keyguardLocked(serial: string): Promise<boolean> {
  if (!isTauri) return false;
  return invoke<boolean>('keyguard_locked', { serial });
}

/** Tell the main window the phone is open, so its banner does not wait for the 30 s poll. */
export async function reportUnlocked(serial: string): Promise<void> {
  if (!isTauri) return;
  await emit('device-unlocked', { serial });
}

/**
 * Report the box the mirrored phone may occupy, in logical CSS pixels inside the window's
 * client area. The page owns the layout and Rust only ever follows this box, so the video never
 * resizes the window. An empty box puts the video away while the session keeps running.
 */
export async function setMirrorRect(
  x: number,
  y: number,
  width: number,
  height: number,
): Promise<void> {
  if (!isTauri) return;
  await invoke('set_mirror_rect', { x, y, width, height });
}

/**
 * The phone's proportions (width / height) from the settled scrcpy window, or `null` before
 * the mirror is embedded. The page sizes the mirror column with it so the video fills its
 * column exactly instead of leaving empty bands.
 */
export async function getMirrorAspect(): Promise<number | null> {
  if (!isTauri) return null;
  return invoke<number | null>('get_mirror_aspect');
}

/** scrcpy reported a new size (the mirror attached or the phone rotated). */
export function onMirrorAspect(callback: (aspect: number) => void) {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<number>('mirror-aspect', (event) => callback(event.payload));
}

/**
 * Resize the main window to an exact client width while keeping its height. Used when the
 * admin panel folds away so the window hugs the mirrored phone instead of leaving side bands.
 */
export async function setWindowWidth(
  width: number,
  minWidth: number,
  lockWidth = false,
): Promise<void> {
  if (!isTauri) return;
  await invoke('set_window_width', { width, minWidth, lockWidth });
}

/** Same as `setWindowWidth`, but eased by Rust so the panel does not snap open/closed. */
export async function animateWindowWidth(
  width: number,
  minWidth: number,
  lockWidth = false,
): Promise<void> {
  if (!isTauri) return;
  await invoke('animate_window_width', { width, minWidth, lockWidth });
}

/** Fit the window to the monitor's usable height (top to bottom) after connecting. */
export async function fitWindowHeight(): Promise<void> {
  if (!isTauri) return;
  await invoke('fit_window_height');
}

/** Which side gets the physical keyboard: the mirrored phone or the page and its fields. */
export async function setKeyboardOwner(owner: 'phone' | 'pc'): Promise<void> {
  if (!isTauri) return;
  await invoke('set_keyboard_owner', { owner });
}

export async function openUrl(url: string): Promise<void> {
  if (!isTauri) {
    window.open(url, '_blank', 'noopener');
    return;
  }
  await invoke('open_url', { url });
}

export async function openDeviceCamera(
  serial: string,
  lens: Lens,
  mode: CaptureMode,
): Promise<LocateReport> {
  if (!isTauri) throw 'Pelacakan hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<LocateReport>('open_device_camera', { serial, lens, mode });
}

export async function closeDeviceCamera(serial: string): Promise<LocateReport> {
  if (!isTauri) throw 'Pelacakan hanya tersedia saat aplikasi dijalankan melalui Tauri.';
  return invoke<LocateReport>('close_device_camera', { serial });
}

export function onTransferProgress(callback: (progress: TransferProgress) => void) {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<TransferProgress>('transfer-progress', (event) => callback(event.payload));
}

export function onDevicesChanged(callback: (snapshot: DevicesSnapshot) => void) {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<DevicesSnapshot>('devices-changed', (event) => callback(event.payload));
}


export function onSessionStarted(callback: (info: SessionInfo) => void) {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<SessionInfo>('session-started', (event) => callback(event.payload));
}

export function onSessionExited(callback: (info: SessionExit) => void) {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<SessionExit>('session-exited', (event) => callback(event.payload));
}

export function onSessionError(callback: (info: SessionError) => void) {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<SessionError>('session-error', (event) => callback(event.payload));
}

/** The panel (or its keyguard poll) saw the phone open; the dashboard can stop saying "terkunci". */
export function onDeviceUnlocked(callback: (serial: string) => void) {
  if (!isTauri) return Promise.resolve(() => {});
  return listen<{ serial: string }>('device-unlocked', (event) =>
    callback(event.payload.serial),
  );
}
