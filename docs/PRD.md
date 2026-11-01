# PRD — Your Android on Your PC (YAOP)

Versi 1.0 · 25 Oktober 2026

## 1. Ringkasan

Aplikasi desktop Windows yang menyatukan Android dan PC dalam satu pengalaman: kontrol HP dari komputer, mirroring layar, screenshot, clipboard, serta transfer file dan notifikasi pada tahap berikutnya.

**Visi:** Android menjadi ekstensi alami dari komputer Windows, sehingga interaksi sehari-hari dengan HP tidak lagi memutus alur kerja di PC.

**Prinsip produk**

- **Local-first** — koneksi langsung PC–HP, tanpa server cloud.
- **Simple by default** — fitur utama mudah ditemukan; fitur lanjutan tidak memenuhi layar.
- **One device, one workspace** — perangkat, kontrol, file, dan status ada di satu aplikasi.
- **Secure by design** — pemasangan perangkat disetujui pengguna dan dapat dicabut.
- **Progressive enhancement** — fitur dasar stabil dulu, fitur lanjutan menyusul.

## 2. Target pengguna

| Pengguna | Kebutuhan |
|---|---|
| Produktivitas | Mengontrol HP, mengetik, membuka aplikasi dari PC |
| Sering bertukar file | Transfer foto/dokumen/folder dengan cepat |
| Pengembang Android | Akses ADB, info perangkat, screenshot, log |
| Multi-device | Menghubungkan dan berpindah antar HP |

**Masalah:** berpindah antara PC dan HP terlalu sering memutus alur kerja.

## 3. Ruang lingkup

### Fase 1 — MVP

- Deteksi perangkat via USB
- Pairing dan koneksi ulang
- Mirroring layar + kontrol mouse/keyboard
- Koneksi Wi-Fi setelah konfigurasi pairing
- Status perangkat dan koneksi
- Screenshot dan clipboard dasar (teks)
- Pengaturan kualitas gambar
- Penanganan perangkat terputus / proses gagal

### Fase 2 — Integration

- Transfer file dua arah + progres
- Clipboard dua arah yang konsisten
- Notifikasi Android (butuh aplikasi pendamping)
- Daftar perangkat yang pernah dipasangkan
- System tray, autostart opsional

### Fase 3 — Ecosystem

- Multi-device workspace
- Launcher aplikasi Android
- Galeri / penjelajah file terintegrasi
- Workflow otomatis
- Remote access aman (bila diperlukan)

**Di luar MVP:** akun pengguna, cloud sync, remote access internet, otomasi kompleks.

## 4. Spesifikasi fungsional

| ID | Fitur | Kriteria penerimaan |
|---|---|---|
| FR-01 | Device discovery | Perangkat ADB muncul beserta status (`device`, `unauthorized`, `offline`) |
| FR-02 | Pairing | Perangkat belum diotorisasi menampilkan petunjuk tindakan |
| FR-03 | Screen mirroring | Layar HP tampil di desktop |
| FR-04 | Remote input | Klik, scroll, ketikan terkirim ke perangkat |
| FR-05 | Wireless connection | Pairing Wi-Fi dengan prosedur aman; alamat/port tidak diasumsikan tetap |
| FR-06 | Clipboard | Teks dapat dipindahkan PC↔HP; batasan Android dijelaskan di UI |
| FR-07 | Screenshot | Screenshot tersimpan ke lokasi pilihan |
| FR-08 | Recovery | Putus koneksi → status jelas + tombol reconnect |
| FR-09 | Settings | Kualitas streaming, penyimpanan, perilaku aplikasi |
| FR-10 | File transfer (fase 2) | Kirim file, progres, dan hasil terlihat |

## 5. Target kualitas MVP

Sasaran pengujian, bukan klaim performa.

- Startup ≤ 3 detik (di luar pairing awal)
- Sesi mirroring terbuka ≤ 3 detik setelah perangkat siap
- Latensi interaksi median < 100 ms pada USB yang memadai
- Sesi 30 menit tanpa crash pada perangkat uji
- Recovery tanpa restart aplikasi
- CPU/RAM/jaringan terukur pada resolusi dan frame rate yang ditetapkan

## 6. UI/UX

Satu jendela, sidebar empat area: **Home, Devices, Files, Settings**. Layar HP menjadi fokus saat sesi aktif. Status koneksi selalu terlihat. Tombol utama berlabel, bukan ikon saja. Light/dark mode dengan satu mode default. Dialog konfirmasi hanya untuk tindakan berisiko.

## 7. Definisi MVP selesai

- [ ] Aplikasi Windows menampilkan perangkat Android yang terdeteksi
- [ ] Otorisasi USB dan mirroring berjalan
- [ ] Mouse dan keyboard mengontrol perangkat
- [ ] Pairing dan koneksi Wi-Fi berfungsi
- [ ] Status `disconnected`/`unauthorized` jelas
- [ ] Sesi berhenti tanpa proses menggantung
- [ ] Screenshot tersimpan; clipboard dasar teruji
- [ ] Installer terpasang di PC pengujian tanpa setup pengembangan
- [ ] Tidak ada akses kontrol jaringan terbuka tanpa otorisasi

## 8. Risiko utama

| Risiko | Mitigasi |
|---|---|
| Perbedaan perilaku antarversi Android | Tetapkan versi minimum setelah uji perangkat nyata |
| ADB tidak ditemukan | Resolusi path berlapis + pesan error jelas |
| Proses scrcpy menggantung | Process manager + cleanup saat aplikasi keluar |
| Wi-Fi terputus | Timeout, status sesi, reconnect tanpa sesi duplikat |
| Clipboard tidak konsisten | Mulai dari teks; dokumentasikan batasan Android |
| Installer besar | Evaluasi binary yang dibundel |
| Lisensi | Audit lisensi scrcpy, platform-tools, dan dependensi sebelum rilis |
