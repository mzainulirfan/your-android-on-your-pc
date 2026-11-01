# SECURITY — YAOP

Aplikasi memberi kontrol luas atas perangkat Android. Keamanan adalah bagian desain, bukan tambahan akhir.

## Model ancaman (ringkas)

| Aset | Ancaman | Kontrol |
|---|---|---|
| Kontrol perangkat | Input tak sah dari UI/string shell | Command tervalidasi; argumen terpisah |
| Koneksi Wi-Fi | Penyadapan / perangkat palsu | Handoff hanya lewat aksi eksplisit; autentikasi key ADB; tidak ada auto-connect |
| Data clipboard/file | Bocor ke log/cloud | Tanpa cloud; isi data tidak dilog |
| Proses ADB/scrcpy | Proses menggantung / orphan | Process manager + cleanup saat exit |
| Filesystem PC | Akses path sembarang | Batasi path yang dipakai aplikasi |

## Aturan wajib

- Tidak meminta root pada Android.
- Tidak membuka port kontrol jaringan tanpa kebutuhan dan otorisasi pengguna.
- Tidak mengirim data ke cloud secara default; tanpa telemetri.
- UI tidak boleh mengeksekusi shell bebas. Frontend hanya memanggil command yang terdaftar.
- Serial perangkat divalidasi (`[A-Za-z0-9._:-]+`) sebelum dipakai sebagai argumen.
- Jangan menyimpan kredensial/pairing key sebagai metadata biasa.
- Jangan menulis isi clipboard, file, atau screenshot ke log.
- Semua proses child yang dijalankan aplikasi harus bisa dihentikan dan dibersihkan.
- Capability Tauri dibatasi ke minimum yang dibutuhkan.

## ADB via Wi-Fi (handoff)

- Mode TCP/IP hanya diaktifkan saat pengguna menekan **Aktifkan Wi-Fi** pada perangkat USB.
- ADB tetap memakai autentikasi key; hanya host yang sudah diotorisasi di HP yang bisa terhubung.
- Port default 5555 terbuka selama adbd berjalan dalam mode TCP/IP. Tutup dengan `adb -s <ip:5555> usb`, cabut kabel lalu reboot HP, atau matikan Wireless debugging.
- Handoff mengasumsikan LAN tepercaya (satu jaringan dengan PC); jangan gunakan di jaringan publik tanpa pemahaman risikonya.

## Clipboard & transfer file

- Clipboard teks ditangani sepenuhnya oleh protokol scrcpy; aplikasi tidak membaca, menyimpan, atau mencatat isinya.
- Gambar clipboard hanya dibaca saat pengguna menekan **Kirim gambar clipboard**. Gambar disimpan sebagai PNG sementara, dikirim ke `/sdcard/Download/`, lalu file sementara dihapus. Isi gambar tidak pernah dilog.
- URI share memakai MediaStore dengan izin baca sementara (`FLAG_GRANT_READ_URI_PERMISSION`); aplikasi tidak membuka akses folder lain.
- Transfer file hanya menulis ke `/sdcard/Download/` (target tetap), dengan path yang berasal dari drag & drop pengguna dan divalidasi.
- Nama/path file tidak ditulis ke log aplikasi; yang dipancarkan hanya nama file dan persentase untuk UI.
- Transfer Android → PC belum tersedia; jangan menganggap scrcpy mendukung pull.

## Kunci layar (PIN/password)

- Ketikan manual dikirim sebagai satu operasi `unlock_device`; tidak ada pengiriman per tombol digit dan tidak ada penulisan ke log.
- Menyimpan kunci itu **opt-in per perangkat**: pengguna menekan **Simpan** di panel kontrol. Tidak ada kunci yang tersimpan tanpa tindakan itu.
- Yang disimpan adalah blob **DPAPI** (`CryptProtectData` dengan `CRYPTPROTECT_UI_FORBIDDEN` + entropy aplikasi) di `credentials.json` pada folder konfigurasi. Ikatanannya ke akun Windows saat ini, jadi file yang dicuri dari disk tidak bisa dibuka oleh proses lain maupun pengguna lain, dan tidak ada cloud atau cadangan otomatis.
- Antarmuka hanya pernah menerima `id`, label, metode, dan **panjang** kunci. Nilai plaintext dibuka di thread worker saat eksekusi unlock, langsung diketik ke perangkat, dan buffer lokal dinolkan setelah dipakai.
- Batas penyimpanan: 4 kunci per perangkat, 24 total, maksimal 64 karakter per kunci.
- Penghapusan tersedia dari chip panel kontrol dan dari halaman Settings (satu per satu atau **Hapus semua**). Di luar Windows fitur simpan ditolak, bukan diam-diam menulis plaintext.
- Kunci yang tersimpan bisa dipakai membypass layar dari PC tanpa konfirmasi tambahan, jadi fitur ini hanya boleh dipakai di PC yang terkunci sendiri.
- Pairing key ADB tetap tidak pernah ditulis sebagai metadata biasa.

## Izin Android (fase 2)

Aplikasi pendamping harus meminta izin eksplisit (notifikasi, akses file). Sinkronisasi clipboard background dibatasi sistem Android dan tidak boleh diasumsikan selalu tersedia.

## Supply chain & lisensi

- scrcpy dan platform-tools: periksa lisensi, kewajiban atribusi/distribusi source, dan checksum sebelum dibundel.
- Dependensi Rust dan npm di-audit sebelum rilis (`cargo audit`, `npm audit`).
- Jangan menambahkan dependency tanpa kebutuhan jelas.

## Yang belum diterapkan (utang teknis)

- CSP ketat untuk WebView (saat ini dibatasi, bukan final).
- Penandatanganan installer Windows.
- Rate limit / penguncian saat perangkat tidak diotorisasi.
- Audit log lokal (tanpa konten sensitif).
