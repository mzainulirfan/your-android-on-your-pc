# Your Android on Your PC

Aplikasi desktop Windows untuk mengontrol Android dari PC. Fondasi: Tauri 2 + Rust + Svelte 5 + Tailwind CSS 4, dengan ADB dan scrcpy sebagai mesin perangkat.

Status: **Sprint 4** — clipboard dua arah, transfer file PC ↔ Android (halaman Files), screenshot, dan pengaturan kualitas.

## Prasyarat

- Node.js 20+
- Rust (MSVC toolchain) + Visual Studio Build Tools (C++)
- WebView2 (sudah ada di Windows 10/11)
- **scrcpy**: letakkan `scrcpy.exe` beserta DLL dan `scrcpy-server` di `binaries/scrcpy/`, set `YAOP_SCRCPY_PATH`, atau pastikan `scrcpy` ada di `PATH`

## Menjalankan

```bash
cd apps/desktop
npm install
npm run tauri dev
```

Frontend saja (tanpa akses ADB): `npm run dev`.

## Navigasi

- **Home** — status ADB/scrcpy/perangkat/sesi, aksi cepat (mulai mirroring, screenshot, kelola file), dan kartu navigasi.
- **Devices** — daftar perangkat, hubungkan sesi scrcpy, handoff Wi-Fi, clipboard gambar, screenshot.
- **Files** — jelajahi penyimpanan publik HP (`/sdcard/...`), unduh file ke PC, dan unggah dengan drag & drop ke folder yang sedang dibuka.
- **Settings** — resolusi & bitrate mirroring, folder unduhan PC, dan diagnostics (versi/path ADB, scrcpy, lokasi konfigurasi).

## Clipboard & transfer file

- **HP → PC**: salin teks di HP; clipboard PC otomatis diperbarui selama sesi aktif.
- **PC → HP**: salin teks di PC, fokuskan kolom teks di HP, lalu tekan **Alt+V** di jendela scrcpy. `Ctrl+V` tidak dipakai karena scrcpy meneruskannya langsung ke Android.
- **Gambar**: salin screenshot/gambar di PC, lalu klik **Kirim gambar clipboard**. Gambar dikirim ke Download dan lembar Bagikan Android terbuka; pilih WhatsApp lalu chat tujuan. File juga tersedia di folder Download.
- **File PC → HP**: tarik file ke jendela YAOP. Di halaman Files, file masuk ke folder yang sedang dibuka; di halaman lain ke folder **Download**.
- **File HP → PC**: halaman Files → **Unduh**; tersimpan di folder unduhan pada Settings.
- **Screenshot**: tombol **Screenshot** (banner sesi atau Home) menyimpan PNG layar HP ke folder unduhan.

## Struktur

```text
apps/desktop/       Svelte frontend + Tauri Rust backend
binaries/adb/       adb.exe lokal (opsional)
binaries/scrcpy/    scrcpy.exe lokal (opsional, Sprint 2)
docs/               PRD, arsitektur, roadmap, keamanan
```

## Dokumen

- [AGENTS.md](AGENTS.md) — aturan kerja agent
- [docs/PRD.md](docs/PRD.md) — kebutuhan produk
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — arsitektur dan kontrak command/event
- [docs/ROADMAP.md](docs/ROADMAP.md) — rencana sprint
- [docs/SECURITY.md](docs/SECURITY.md) — model ancaman dan kontrol
