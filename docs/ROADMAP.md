# ROADMAP — YAOP

Asumsi: satu pengembang, Windows, scrcpy sebagai fondasi.

## Sprint 1 — Foundation ✅ (vertical slice)

- [x] Scaffold Tauri 2 + Svelte 5 + Tailwind 4
- [x] Resolusi binary ADB berlapis
- [x] Command `list_devices` dan `get_adb_status`
- [x] Event `devices-changed` (polling 2 detik)
- [x] UI daftar perangkat + status koneksi + petunjuk ADB
- [x] Build frontend dan `cargo check`

**Uji terima:** buka aplikasi → perangkat USB yang terdeteksi ADB muncul; perangkat `unauthorized` diberi petunjuk; ADB tidak ada → instruksi jelas.

## Sprint 2 — Remote Control ✅

- [x] Resolusi binary scrcpy
- [x] `session_manager`: start/stop, exit code, cleanup
- [x] Command `start_session`, `stop_session`, `get_session_status`
- [x] Event `session-started`, `session-error`, `session-exited`
- [x] UI workspace: tombol connect/disconnect, status sesi
- [x] Uji: start/stop pada perangkat nyata, crash scrcpy terdeteksi, cleanup saat aplikasi keluar
- [ ] Uji manual lanjutan: input mouse/keyboard dan cabut kabel saat sesi aktif

**Uji terima:** pilih perangkat → sesi scrcpy berjalan → input berfungsi → stop menutup proses.

## Sprint 3 — Wireless (sebagian)

- [x] Handoff USB → Wi-Fi: `adb tcpip` + `adb connect` berdasarkan IP `wlan`
- [ ] Alur `adb pair` / Wireless debugging
- [ ] Input IP:port manual + validasi
- [ ] Reconnect dan timeout tanpa sesi duplikat
- [x] Preferensi kualitas (resolusi, bitrate) → diteruskan ke `--max-size` dan `--video-bit-rate` scrcpy
- [x] Penyimpanan pengaturan lokal (`settings.json` di folder konfigurasi aplikasi)
- [ ] Uji jaringan putus/lemah

## Sprint 4 — MVP Release

- [x] Clipboard teks via sesi scrcpy (dua arah, Android 11+)
- [x] Gambar clipboard PC → HP: push ke Download + lembar Bagikan Android
- [x] Transfer file PC → Android: drag & drop ke jendela + progres
- [x] Screenshot: `exec-out screencap -p` → PNG ke folder unduhan, tombol di banner sesi dan Home
- [x] Transfer Android → PC / file manager: halaman Files (daftar folder publik, unduh per file dengan progres)
- [x] Unggah drag & drop ke folder Files aktif (target `adb push` mengikuti direktori yang dibuka)
- [x] Diagnostics (versi ADB/scrcpy, path, folder konfigurasi, versi aplikasi) di halaman Settings
- [ ] Error handling menyeluruh
- [ ] Installer Windows + pemeriksaan dependensi
- [ ] Dokumentasi setup/troubleshooting
- [ ] Audit lisensi scrcpy/platform-tools

## Fase lanjutan

- [ ] Aplikasi pendamping Android (notifikasi, clipboard latar belakang)
- [ ] Multi-device workspace
- [ ] Launcher aplikasi dan workflow otomatis
- [ ] Remote access aman (opsional)

## Prinsip urutan

USB stabil dulu → Wi-Fi → fitur integrasi. Jangan menambah permukaan keamanan (cloud, remote) sebelum alur inti terbukti.
