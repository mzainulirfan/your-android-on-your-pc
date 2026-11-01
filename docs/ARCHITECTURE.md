# ARCHITECTURE — YAOP

## 1. Lapisan

```text
Svelte 5 + Tailwind          UI: Home, Devices, Files, Settings
        │ Tauri commands / events
Rust core (Tauri 2)          device, connection, session, input,
        │                    clipboard, transfer, settings, diagnostics
ADB server + scrcpy process  discovery, mirroring, input
        │
Android device               USB atau Wi-Fi lokal
```

Frontend tidak memegang logika perangkat. Semua operasi melewati command Rust; perubahan status dikirim sebagai event.

## 2. Modul backend

| Modul | Tanggung jawab | Status |
|---|---|---|
| `device_manager` (src/adb.rs) | Resolusi binary ADB, discovery, parsing status | MVP slice |
| `connection_manager` | USB, Wi-Fi, timeout, reconnect | Sebagian (handoff USB→Wi-Fi) |
| `session_manager` | Siklus hidup proses scrcpy, cleanup | Ada |
| `host` (src/host.rs) | Embed jendela scrcpy ke jendela utama (Win32 `SetParent`/`WS_CHILD`), geometri anak, relay keyboard | Ada |
| `mirror` (src/mirror.rs) | `MirrorTracker` — polling `host::sync` tiap 40 ms selama sesi ter-embed | Ada |
| `unlock` (src/unlock.rs) | Ketik PIN/password dalam satu sesi shell + verdict keyguard | Ada |
| `credentials` (src/credentials.rs) | Kunci tersimpan, blob DPAPI (`credentials.json`) | Ada |
| `locate` (src/locate.rs) | Bangunkan layar + intent Find My Device / kamera | Ada |
| `input_controller` | Kontrol input (via scrcpy) | Ada (via jendela scrcpy ter-embed) |
| `clipboard_manager` | Teks via scrcpy; gambar PC → HP via push + share | Ada |
| `transfer_manager` | Transfer file dua arah + progres | Ada |
| `files` (src/files.rs) | Daftar isi penyimpanan Android (`ls -la`), validasi path remote | Ada |
| `screenshot` (src/screenshot.rs) | `exec-out screencap -p` → PNG ke folder unduhan | Ada |
| `notification_bridge` | Notifikasi via companion app | Fase 2 |
| `settings_manager` | Preferensi lokal (kualitas, folder unduhan) di `settings.json` | Ada |
| `security_manager` | Validasi input, pembatasan akses | Sebagian |
| `diagnostics` | Status dependensi dan lokasi konfigurasi | Ada (versi/path, tanpa log) |

## 3. Kontrak frontend–backend

### Commands

| Command | Fungsi | Status |
|---|---|---|
| `get_adb_status` | Lokasi, versi, dan error ADB | Ada |
| `list_devices` | Daftar perangkat + status | Ada |
| `get_scrcpy_status` | Lokasi, versi, dan error scrcpy | Ada |
| `start_session` | Jalankan scrcpy untuk serial tertentu | Ada |
| `stop_session` | Hentikan sesi dan proses | Ada |
| `get_session_status` | Sesi aktif saat ini atau `null` | Ada |
| `enable_wireless` | Handoff perangkat USB ke ADB via Wi-Fi | Ada |
| `push_files` | Kirim file/folder PC ke direktori Files aktif atau `/sdcard/Download/` | Ada |
| `send_clipboard_image` | Ambil gambar clipboard PC → push ke Download → buka lembar Bagikan Android | Ada |
| `list_android_files` | Daftar isi folder publik Android (path divalidasi) | Ada |
| `pull_android_file` | Unduh satu file Android ke folder unduhan PC dengan progres | Ada |
| `capture_screenshot` | Screenshot perangkat → PNG ke folder unduhan | Ada |
| `get_settings` / `save_settings` | Baca/simpan preferensi kualitas dan folder unduhan | Ada |
| `get_diagnostics` | Versi/path ADB+scrcpy, lokasi konfigurasi, versi aplikasi | Ada |
| `open_download_dir` | Buka folder unduhan di Explorer | Ada |
| `set_mirror_rect` | Laporkan kotak CSS kolom kiri (px CSS) ke Rust; juga switch sembunyi (kotak nol ⇒ `SW_HIDE`) | Ada |
| `set_keyboard_owner` | Alihkan fokus keyboard ke `phone` (relay ke cermin) atau `pc` (field kartu kunci) | Ada |
| `wake_state` / `screen_state` / `set_screen_power` | Verdict layar + keyguard dalam satu sesi shell | Ada |
| `unlock_device` / `unlock_with_credential` | Ketik kunci / pakai kunci tersimpan DPAPI | Ada |
| `list_credentials` / `save_credential` / `delete_credentials` | Kelola kunci tersimpan per perangkat | Ada |
| `connect_wireless` | Pairing/koneksi Wi-Fi | Sebagian (handoff; `adb pair` belum) |
| `get_device_details` | Info perangkat (model, versi Android) | Belum |

### Events

| Event | Payload | Status |
|---|---|---|
| `devices-changed` | `{ devices, error }` | Ada |
| `session-started` | info sesi (serial, pid, startedAt, `screenOn`, `locked`) | Ada |
| `device-unlocked` | serial — dipancarkan kartu kunci saat HP terdeteksi terbuka, supaya banner Home hilang seketika | Ada |
| `session-exited` | serial, code, reason (`stopped`/`exited`) | Ada |
| `session-error` | serial, code, message | Ada |
| `transfer-progress` | serial, direction (`push`/`pull`), index, total, fileName, percent, done | Ada |

## 4. ADB

Urutan resolusi binary:

1. `YAOP_ADB_PATH`
2. `ANDROID_HOME/platform-tools/adb.exe`
3. `ANDROID_SDK_ROOT/platform-tools/adb.exe`
4. `%LOCALAPPDATA%/Android/Sdk/platform-tools/adb.exe`
5. `binaries/adb/adb.exe` di root proyek (dev)
6. `adb` dari `PATH`

Perintah yang dipakai saat ini: `adb version`, `adb devices -l`. Argumen dikirim terpisah (bukan string shell). Serial divalidasi dengan pola `[A-Za-z0-9._:-]+` sebelum dipakai di command berikutnya.

Parsing `adb devices -l`:

```text
SERIAL STATE [key:value ...]
```
`model`, `product`, `device`, `transport_id` diambil bila ada. Serial berisi `:` dianggap koneksi Wi-Fi (TCP/IP).

Polling backend setiap 2 detik; jika hasil berubah, event `devices-changed` dikirim. Polling sederhana dipilih agar UI tidak perlu memanggil ADB sendiri.

### Wireless handoff (USB → Wi-Fi)

1. Pengguna menekan **Aktifkan Wi-Fi** pada perangkat USB (aksi eksplisit, tidak otomatis).
2. Backend membaca IP Wi-Fi HP: `adb -s <serial> shell ip route` (cari `src` pada antarmuka `wlan*`), fallback `ip -f inet addr show wlan0`.
3. IP divalidasi sebagai IPv4 yang dapat dipakai (bukan loopback/link-local).
4. `adb -s <serial> tcpip 5555` lalu `adb connect <ip>:5555` dengan retry singkat.
5. Backend menunggu sampai `adb devices` melaporkan `<ip>:5555` berstatus `device` sebelum mengembalikan `{ usbSerial, wifiSerial }`.

Setelah handoff, perangkat tetap terdaftar meski kabel dicabut. Sesi scrcpy baru harus dijalankan pada serial Wi-Fi. Membalikkan: `adb -s <ip:5555> usb` (atau reboot HP).

## 5. scrcpy (Sprint 2)

- Dijalankan sebagai child process dengan argumen eksplisit: `-s <serial>`, `--no-audio`, `--window-title=YAOP - <serial>`, plus `--max-size` dan `--video-bit-rate` dari pengaturan bila bukan 0.
- Lokasi ADB diteruskan melalui environment variable `ADB` (scrcpy 3+ tidak lagi menerima `--adb`).
- `SessionManager` menyimpan handle proses, memantau exit code tiap 500 ms, dan mematikan proses saat stop/exit aplikasi.
- Satu sesi aktif pada satu waktu; start kedua ditolak sampai sesi sebelumnya berhenti.
- **Model satu jendela**: `--window-borderless` membuat scrcpy tak punya title bar, lalu jendela SDL-nya (`YAOP - <serial>`) di-reparent ke kolom kiri jendela utama lewat `host.rs` (`SetParent` + `WS_POPUP`→`WS_CHILD`, `SetWindowPos(HWND_TOP)` agar melukis di atas WebView2). `host::begin` menunggu jendela itu stabil (≥200 px, 400 ms) supaya tidak menempel ke placeholder 256×256 scrcpy. Geometri ditulis dari kotak CSS yang dilaporkan frontend (`set_mirror_rect`) dikali `scale_factor`, aspect-fit lewat `ASPECT`. Keyboard memakai relay subclass (`FORWARD_KEYS`) ke hwnd SDL karena Windows tak mengirim input asli ke `WS_CHILD` lintas proses dan SDL hanya mau fokus bila ia `GetForegroundWindow`. Bila embed gagal, `detach_standalone` memulihkan `WS_OVERLAPPEDWINDOW` (scrcpy jadi jendela mengambang).
- Distribusi binary harus memeriksa lisensi dan checksum.

### Clipboard

- scrcpy 5 menyinkronkan clipboard secara default (Android 11+): perubahan clipboard HP masuk ke clipboard PC, dan `Alt+V` di jendela scrcpy menyinkronkan clipboard PC ke HP sebelum paste.
- `Alt+V` adalah shortcut `MOD+V` scrcpy: MOD bawaan adalah Alt kiri atau Super kiri, bukan Ctrl. `Ctrl+V` hanya diteruskan ke Android dan mem-paste clipboard perangkat yang belum tersinkron.
- Setelah `Alt+V`, clipboard perangkat juga berisi teks PC, sehingga menu Paste (tekan lama) ikut berfungsi.
- Aplikasi tidak membaca/menulis clipboard teks dan tidak mencatat isinya.
- Gambar tidak didukung clipboard scrcpy. Alur gambar bersifat eksplisit: pengguna menyalin screenshot/gambar di PC lalu menekan **Kirim gambar clipboard**.
- Backend membaca gambar clipboard (`arboard`), menyimpannya sebagai PNG sementara, mengirimnya ke `/sdcard/Download/` lewat `adb push`, memindai MediaStore, lalu membuka `ACTION_SEND` dengan URI `content://media/external/images/media/<id>` dan izin baca sementara. Pengguna memilih aplikasi tujuan (mis. WhatsApp) di lembar Bagikan.
- File PNG sementara di PC dihapus setelah proses selesai; isi gambar tidak pernah dilog. Bila MediaStore belum mengindeks atau intent ditolak, gambar tetap ada di Download dan UI memberi instruksi manual.
- Sinkronisasi teks hanya aktif selama sesi scrcpy berjalan. Sinkronisasi latar belakang tanpa sesi memerlukan aplikasi pendamping Android (fase 2).

### Transfer file PC → Android

- Tarik file/folder ke jendela YAOP. Frontend memakai event drag & drop Tauri dan memanggil command `push_files`.
- Di halaman Devices/Home, tujuan tetap `/sdcard/Download/` (sesuai `--push-target` scrcpy). Di halaman Files, tujuan mengikuti folder Android yang sedang dibuka.
- Drag & drop ke jendela scrcpy juga didukung oleh scrcpy sendiri (proses terpisah); sesi dijalankan dengan `--push-target=/sdcard/Download/` agar tujuannya sama.
- Tujuan: perangkat sesi aktif; tanpa sesi, hanya jika tepat satu perangkat berstatus `device`.
- Backend menjalankan `adb -s <serial> push -p <path> <target>/` per item dan memancarkan `transfer-progress` (arah, index/total, nama file, persen, done).
- Path lokal berasal dari aksi pengguna, divalidasi keberadaannya; target remote divalidasi terhadap root penyimpanan dan karakter berbahaya. Argumen dikirim terpisah tanpa shell bebas.

### Halaman Files (Android → PC)

- `list_android_files` menjalankan `adb -s <serial> shell ls -la '<path>'` dengan path dalam petik tunggal; hanya `/sdcard` dan `/storage/emulated/0` yang diizinkan, `.`/`..` dan `'` ditolak.
- Parsing baris `ls`: symlink dilewati, folder diurutkan lebih dulu, nama berisi spasi ditangani pemecah field manual.
- `pull_android_file` menjalankan `adb -s <serial> pull -p <remote> <local>` ke folder unduhan; file parsial dihapus bila gagal; nama lokal dibuat unik (`nama (1).ext`).
- Progres memakai event `transfer-progress` dengan `direction: "pull"`.

### Screenshot

- `capture_screenshot` menjalankan `adb -s <serial> exec-out screencap -p`, memverifikasi signature PNG, lalu menyimpan `yaop-screenshot-<timestamp>.png` ke folder unduhan dari pengaturan. Isi gambar tidak dilog.

### Pengaturan lokal

- `settings.json` di `app_config_dir` (`%APPDATA%\com.yaop.desktop\`) menyimpan `maxSize`, `videoBitrateKbps`, dan `downloadDir`.
- Validasi: resolusi 0 atau 320–4096; bitrate 0 atau 100–100.000 kbps; folder unduhan harus absolut dan sudah ada.
- Sesi scrcpy membaca pengaturan saat start dan menambahkan `--max-size` / `--video-bit-rate` bila bukan 0.

## 6. Model data

```ts
type DeviceConnectionType = "usb" | "wifi";
type DeviceStatus = "device" | "unauthorized" | "offline" | "recovery" | "connecting" | "error";

interface AndroidDevice {
  serial: string;
  state: DeviceStatus;
  connectionType: DeviceConnectionType;
  model?: string;
  product?: string;
  device?: string;
  transportId?: string;
}
```

`serial` adalah identitas utama, bukan nama model.

## 7. Aplikasi pendamping Android (fase 2)

Dibuat hanya untuk fungsi yang tidak bisa ditangani ADB/scrcpy: notifikasi, clipboard background, mungkin wireless pairing yang lebih ramah. Izin diminta eksplisit; tidak ada akses root.

## 8. Struktur aktual

```text
apps/desktop/
├── src/
│   ├── App.svelte          satu jendela: kolom kiri (mirror + kartu kunci) | kolom kanan (halaman)
│   └── lib/
│       ├── api.ts / types.ts / deviceFormat.ts
│       ├── components/KeypadControls.svelte   kartu PIN/remember-me
│       └── pages/          HomePage, DevicesPage, FilesPage, SettingsPage
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs          setup, tray, event loop, close-to-tray
│   │   ├── adb.rs          resolusi + parsing ADB, verdict layar/keyguard
│   │   ├── session.rs      SessionManager scrcpy
│   │   ├── host.rs         embed SDL ke jendela utama + geometri + relay keyboard
│   │   ├── mirror.rs       MirrorTracker (poll host::sync 40 ms)
│   │   ├── unlock.rs       ketik PIN/password satu sesi shell + verdict
│   │   ├── credentials.rs  kunci tersimpan DPAPI
│   │   ├── locate.rs       Find My Device + intent kamera
│   │   ├── commands.rs     command Tauri
│   │   ├── transfer.rs     push/pull + progres
│   │   ├── files.rs        daftar isi + validasi path remote
│   │   ├── screenshot.rs   screencap → PNG
│   │   ├── settings.rs     settings.json + validasi
│   │   └── clipboard_image.rs  gambar clipboard → push + share
│   ├── capabilities/default.json   (hanya jendela "main")
│   └── tauri.conf.json             (satu window: main)
├── vite.config.ts
└── package.json
```
