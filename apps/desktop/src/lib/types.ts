export type DeviceConnectionType = 'usb' | 'wifi';

export interface AndroidDevice {
  serial: string;
  state: string;
  connectionType: DeviceConnectionType;
  model?: string;
  product?: string;
  deviceCodename?: string;
  transportId?: string;
}

export interface AdbStatus {
  found: boolean;
  path?: string;
  version?: string;
  error?: string;
}

export interface DevicesSnapshot {
  devices: AndroidDevice[];
  error?: string;
}

export interface ScrcpyStatus {
  found: boolean;
  path?: string;
  version?: string;
  error?: string;
}

export interface SessionInfo {
  serial: string;
  startedAt: number;
  pid: number;
  locked: boolean;
  /** Display state at session start, read by the same prep that woke the phone. */
  screenOn: boolean;
}

export interface SessionExit {
  serial: string;
  code: number | null;
  reason: 'stopped' | 'exited';
}

export interface SessionError {
  serial: string;
  code: number | null;
  message: string;
}

export interface WirelessInfo {
  usbSerial: string;
  wifiSerial: string;
}

export interface TransferProgress {
  serial: string;
  direction: 'push' | 'pull';
  index: number;
  total: number;
  fileName: string;
  percent: number;
  done: boolean;
}

export interface TransferFailure {
  name: string;
  error: string;
}

export interface TransferResult {
  pushed: number;
  failed: TransferFailure[];
}

export interface ImageSendResult {
  fileName: string;
  shareOpened: boolean;
}

export interface AndroidEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
  modified?: string;
}

export interface PullResult {
  localPath: string;
  bytes: number;
}

export interface ScreenshotResult {
  path: string;
  fileName: string;
}

export type Lens = 'back' | 'front';
export type CaptureMode = 'photo' | 'video';

export interface LocateReport {
  action: string;
  status: 'ok' | 'partial';
  notes: string[];
  portalUrl?: string;
  portalLabel?: string;
}

export interface UnlockReport {
  status: string;
  method: 'pin' | 'text';
  length: number;
  unlocked: boolean;
  notes: string[];
}

/** What the panel is allowed to know about a saved key: which one it is and how long it
 * is, never the key itself. */
export interface SavedCredential {
  id: string;
  serial: string;
  label: string;
  method: 'pin' | 'text';
  length: number;
}

/** Live display + lock state, read from the phone in a single shell session. */
export interface WakeState {
  screenOn: boolean;
  keyguardUp: boolean;
}

/** What the mirror toolbar is allowed to know about the keypad form in the same column. */
export interface KeypadState {
  expanded: boolean;
  screenOn: boolean;
  screenBusy: boolean;
  busy: boolean;
  locked: boolean;
  message: string | null;
  messageKind: 'locked' | 'error' | null;
}

export interface Settings {
  maxSize: number;
  videoBitrateKbps: number;
  maxFps: number;
  downloadDir: string;
  closeToTray: boolean;
  mirrorScreenOff: boolean;
  lowLatency: boolean;
  audioEnabled: boolean;
}

export interface Diagnostics {
  adb: AdbStatus;
  scrcpy: ScrcpyStatus;
  configPath?: string;
  downloadDir: string;
  platform: string;
  appVersion: string;
}
