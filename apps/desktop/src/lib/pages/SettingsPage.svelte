<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    clearCredentials,
    deleteCredential,
    getAllCredentials,
    getDiagnostics,
    getSettings,
    openDownloadDir,
    saveSettings,
  } from '../api';
  import type { Diagnostics, SavedCredential } from '../types';

  interface Props {
    onNotice: (kind: 'info' | 'error', text: string) => void;
  }

  let { onNotice }: Props = $props();

  // scrcpy membatasi SISI TERPANJANG, jadi pada panel HP 1080x2340 nilai 1080 berarti
  // ~480x1040 (lembut), dan 1920 berarti ~886x1920 (mendekati aslinya).
  const resolutions = [
    { value: 0, label: 'Bawaan perangkat (lewat Wi-Fi dibatasi 1920 px)' },
    { value: 720, label: '720 px (hemat, layar HP kecil)' },
    { value: 1080, label: '1080 px' },
    { value: 1440, label: '1440 px' },
    { value: 1920, label: '1920 px (disarankan)' },
  ];

  const bitrates = [
    { value: 0, label: 'Bawaan scrcpy' },
    { value: 4000, label: '4 Mbps' },
    { value: 8000, label: '8 Mbps' },
    { value: 16000, label: '16 Mbps' },
    { value: 32000, label: '32 Mbps' },
  ];

  // Layar HP zaman sekarang 90–120 Hz. Tanpa batas, bitrate yang sama dipecah ke dua kali
  // lebih banyak frame, jadi tiap frame kurang bit dan gambarnya terlihat patah-patah.
  const frameRates = [
    { value: 0, label: 'Bawaan perangkat' },
    { value: 30, label: '30 FPS' },
    { value: 60, label: '60 FPS' },
    { value: 90, label: '90 FPS' },
    { value: 120, label: '120 FPS' },
  ];

  let maxSize = $state(1920);
  let bitrate = $state(0);
  let maxFps = $state(60);
  let downloadDir = $state('');
  let closeToTray = $state(true);
  let mirrorScreenOff = $state(false);
  let lowLatency = $state(false);
  let audioEnabled = $state(true);
  let diagnostics = $state<Diagnostics | null>(null);
  let checkingVersions = $state(false);
  let keys = $state<SavedCredential[]>([]);
  let keysLoaded = $state(false);
  let keysBusy = $state(false);
  // Destructive key removal asks once: the button label changes for three seconds.
  let confirmKey = $state<string | null>(null);
  let confirmTimer: number | undefined;
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const [current, info] = await Promise.all([getSettings(), getDiagnostics()]);
      maxSize = current.maxSize;
      bitrate = current.videoBitrateKbps;
      maxFps = current.maxFps;
      downloadDir = current.downloadDir;
      closeToTray = current.closeToTray;
      mirrorScreenOff = current.mirrorScreenOff;
      lowLatency = current.lowLatency;
      audioEnabled = current.audioEnabled;
      diagnostics = info;
    } catch (cause) {
      error = errorText(cause);
    } finally {
      loading = false;
    }
    void loadKeys();
  }

  async function loadKeys() {
    try {
      keys = await getAllCredentials();
    } catch (cause) {
      onNotice('error', errorText(cause));
    } finally {
      keysLoaded = true;
    }
  }

  // Opening the page resolves the tool paths only; a version read runs the binaries
  // (`scrcpy --version` loads SDL), so it waits for this explicit click.
  async function checkVersions() {
    checkingVersions = true;
    try {
      diagnostics = await getDiagnostics(true);
    } catch (cause) {
      onNotice('error', errorText(cause));
    } finally {
      checkingVersions = false;
    }
  }

  function askConfirm(id: string, run: () => void) {
    window.clearTimeout(confirmTimer);
    if (confirmKey === id) {
      confirmKey = null;
      run();
      return;
    }
    confirmKey = id;
    confirmTimer = window.setTimeout(() => (confirmKey = null), 3000);
  }

  async function removeKey(id: string) {
    keysBusy = true;
    try {
      await deleteCredential(id);
      keys = keys.filter((entry) => entry.id !== id);
    } catch (cause) {
      onNotice('error', errorText(cause));
    } finally {
      keysBusy = false;
    }
  }

  async function wipeKeys() {
    keysBusy = true;
    try {
      const removed = await clearCredentials();
      keys = [];
      onNotice('info', `${removed} kunci dihapus dari PC ini.`);
    } catch (cause) {
      onNotice('error', errorText(cause));
    } finally {
      keysBusy = false;
    }
  }

  onDestroy(() => window.clearTimeout(confirmTimer));

  function errorText(cause: unknown) {
    if (typeof cause === 'string') return cause;
    if (cause instanceof Error) return cause.message;
    return 'Terjadi kesalahan yang tidak diketahui.';
  }

  async function save() {
    saving = true;
    error = null;
    try {
      const saved = await saveSettings({
        maxSize,
        videoBitrateKbps: bitrate,
        maxFps,
        downloadDir,
        closeToTray,
        mirrorScreenOff,
        lowLatency,
        audioEnabled,
      });
      maxSize = saved.maxSize;
      bitrate = saved.videoBitrateKbps;
      maxFps = saved.maxFps;
      downloadDir = saved.downloadDir;
      closeToTray = saved.closeToTray;
      mirrorScreenOff = saved.mirrorScreenOff;
      lowLatency = saved.lowLatency;
      audioEnabled = saved.audioEnabled;
      onNotice('info', 'Pengaturan disimpan. Kualitas video berlaku pada sesi mirroring berikutnya.');
    } catch (cause) {
      error = errorText(cause);
      onNotice('error', errorText(cause));
    } finally {
      saving = false;
    }
  }

  async function openFolder() {
    try {
      await openDownloadDir();
    } catch (cause) {
      onNotice('error', errorText(cause));
    }
  }
</script>

<header class="flex flex-wrap items-center justify-between gap-3 border-b border-white/5 px-4 py-4 @2xl:px-6 @5xl:px-8 @2xl:py-5">
  <div class="min-w-0 flex-1">
    <h1 class="text-lg font-semibold tracking-tight">Settings</h1>
    <p class="mt-0.5 text-xs text-zinc-500">Kualitas mirroring, folder unduhan, kunci tersimpan, diagnostics</p>
  </div>
  <div class="flex shrink-0 flex-wrap items-center gap-2">
    <button
      type="button"
      onclick={() => void load()}
      disabled={loading}
      class="rounded-lg border border-white/10 bg-white/5 px-3.5 py-1.5 text-xs font-medium text-zinc-200 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
    >
      {loading ? 'Memuat...' : 'Muat ulang'}
    </button>
    <button
      type="button"
      onclick={() => void save()}
      disabled={saving || loading}
      class="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-4 py-1.5 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40"
    >
      {saving ? 'Menyimpan...' : 'Simpan'}
    </button>
  </div>
</header>

<div class="flex-1 overflow-y-auto px-4 py-5 @2xl:px-6 @5xl:px-8 @2xl:py-6">
  <div class="mx-auto max-w-3xl space-y-5">
    {#if error}
      <div class="rounded-xl border border-rose-500/20 bg-rose-500/[0.06] px-4 py-3 text-xs text-rose-200">
        {error}
      </div>
    {/if}

    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <h2 class="text-sm font-semibold">Kualitas mirroring</h2>
      <p class="mt-1 text-xs text-zinc-500">Berlaku pada sesi scrcpy berikutnya; sesi berjalan tidak berubah.</p>

      <label class="mt-4 flex cursor-pointer items-start gap-3">
        <input
          type="checkbox"
          bind:checked={lowLatency}
          class="mt-0.5 h-4 w-4 shrink-0 accent-emerald-500"
        />
        <span class="text-xs leading-relaxed text-zinc-400">
          <span class="font-semibold text-zinc-200">Prioritas latensi rendah (Wi-Fi)</span> — dibatasi sampai
          <span class="font-mono">1080 px · 60 FPS · 4 Mbps</span>. Yang diperbaiki bukan jaringannya (terukur masih
          9–12 ms), tapi beban <span class="font-semibold text-zinc-200">encode di HP</span>: 1080 px berarti sepertiga
          pixel dibanding 1920 px, jadi animasi action center dan daftar aplikasi tidak berebut GPU dengan mirror. Nilai
          ini hanya <span class="font-semibold text-zinc-200">menurunkan</span> pilihan di bawah, tidak pernah
          menaikkannya. Gambar jadi lebih lembut, gerakan tetap 60 FPS.
        </span>
      </label>
      <div class="mt-4 grid gap-4 @lg:grid-cols-2">
        <label class="block">
          <span class="text-[11px] font-medium uppercase tracking-wider text-zinc-500">Resolusi maksimum</span>
          <select
            bind:value={maxSize}
            class="mt-1.5 w-full rounded-lg border border-white/10 bg-zinc-950/60 px-3 py-2 text-xs text-zinc-200 outline-none focus:border-emerald-500/40"
          >
            {#each resolutions as option (option.value)}
              <option value={option.value}>{option.label}</option>
            {/each}
          </select>
        </label>
        <label class="block">
          <span class="text-[11px] font-medium uppercase tracking-wider text-zinc-500">Bitrate video</span>
          <select
            bind:value={bitrate}
            class="mt-1.5 w-full rounded-lg border border-white/10 bg-zinc-950/60 px-3 py-2 text-xs text-zinc-200 outline-none focus:border-emerald-500/40"
          >
            {#each bitrates as option (option.value)}
              <option value={option.value}>{option.label}</option>
            {/each}
          </select>
        </label>
        <label class="block">
          <span class="text-[11px] font-medium uppercase tracking-wider text-zinc-500">Batas FPS</span>
          <select
            bind:value={maxFps}
            class="mt-1.5 w-full rounded-lg border border-white/10 bg-zinc-950/60 px-3 py-2 text-xs text-zinc-200 outline-none focus:border-emerald-500/40"
          >
            {#each frameRates as option (option.value)}
              <option value={option.value}>{option.label}</option>
            {/each}
          </select>
        </label>
      </div>

      <label class="mt-4 flex cursor-pointer items-start gap-3 border-t border-white/5 pt-4">
        <input
          type="checkbox"
          bind:checked={mirrorScreenOff}
          class="mt-0.5 h-4 w-4 shrink-0 accent-emerald-500"
        />
        <span class="text-xs leading-relaxed text-zinc-400">
          Mulai mirroring dengan <span class="font-semibold text-zinc-200">layar HP tetap mati</span> — semua tampilan dan
          kontrol hanya lewat jendela scrcpy, input tidak membangunkan layar HP (butuh Android 8.1+).
          <span class="font-semibold text-zinc-200">Hanya berlaku lewat kabel USB.</span> Lewat Wi-Fi pilihan ini
          diabaikan: saat layar mati, radio Wi-Fi HP ikut tertidur dan latensi naik dari 87 ms jadi 146 ms, jadi mirror
          justru patah-patah. Tekan <span class="font-semibold text-zinc-200">Alt+B</span> di jendela scrcpy kalau tetap
          ingin mematikan layar di tengah sesi.
        </span>
      </label>

      <label class="mt-4 flex cursor-pointer items-start gap-3 border-t border-white/5 pt-4">
        <input
          type="checkbox"
          bind:checked={audioEnabled}
          class="mt-0.5 h-4 w-4 shrink-0 accent-emerald-500"
        />
        <span class="text-xs leading-relaxed text-zinc-400">
          <span class="font-semibold text-zinc-200">Suara HP ikut ke PC</span> — audio dari HP (game, video, notifikasi)
          dikirim lewat sesi mirroring dan diputar di PC. Butuh <span class="font-semibold text-zinc-200">Android 13 ke
          atas</span>; di bawah itu HP tidak menyediakan jalur tangkap suara sistem, jadi sesi tetap berjalan tanpa
          suara. Biaya bandwidth kecil (Opus ±128 kbps, di atas bitrate video). Suara tetap keluar dari HP seperti
          biasa — PC menerimanya salinan, bukan pengganti. Berlaku pada sesi mirroring berikutnya.
        </span>
      </label>
    </section>

    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <h2 class="text-sm font-semibold">Folder unduhan PC</h2>
      <p class="mt-1 text-xs text-zinc-500">Tujuan file yang diunduh dari HP dan screenshot. Folder harus sudah ada.</p>
      <div class="mt-4 flex flex-wrap items-end gap-2">
        <label class="block min-w-0 flex-1">
          <span class="text-[11px] font-medium uppercase tracking-wider text-zinc-500">Path folder</span>
          <input
            bind:value={downloadDir}
            spellcheck="false"
            class="mt-1.5 w-full rounded-lg border border-white/10 bg-zinc-950/60 px-3 py-2 font-mono text-[11px] text-zinc-200 outline-none focus:border-emerald-500/40"
          />
        </label>
        <button
          type="button"
          onclick={() => void openFolder()}
          class="rounded-lg border border-white/10 bg-white/5 px-3.5 py-2 text-[11px] text-zinc-300 transition hover:bg-white/10"
        >
          Buka folder
        </button>
      </div>
    </section>

    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <h2 class="text-sm font-semibold">Perilaku jendela</h2>
      <label class="mt-3 flex cursor-pointer items-start gap-3">
        <input
          type="checkbox"
          bind:checked={closeToTray}
          class="mt-0.5 h-4 w-4 shrink-0 accent-emerald-500"
        />
        <span class="text-xs leading-relaxed text-zinc-400">
          Saat tombol tutup (X) ditekan, sembunyikan aplikasi ke icon tray di taskbar — sesi mirroring dan jendela scrcpy
          tetap berjalan. Klik kanan icon tray untuk memunculkan kembali atau keluar penuh. Jika dimatikan, menutup
          jendela akan menghentikan sesi dan keluar dari aplikasi.
        </span>
      </label>
    </section>

    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-2">
        <h2 class="flex items-center gap-2 text-sm font-semibold">
          Kunci tersimpan
          {#if keys.length > 0}
            <span class="rounded-full bg-white/[0.06] px-1.5 py-0.5 text-[10px] font-semibold text-zinc-400">
              {keys.length}
            </span>
          {/if}
        </h2>
        {#if keys.length > 0}
          <button
            type="button"
            onclick={() => askConfirm('all', () => void wipeKeys())}
            disabled={keysBusy}
            class="rounded-lg border px-3 py-1 text-[11px] font-medium transition disabled:cursor-not-allowed disabled:opacity-40 {confirmKey ===
            'all'
              ? 'border-rose-500/40 bg-rose-500/20 text-rose-100'
              : 'border-rose-500/25 bg-rose-500/10 text-rose-200 hover:bg-rose-500/20'}"
          >
            {confirmKey === 'all' ? `Yakin hapus ${keys.length} kunci?` : 'Hapus semua'}
          </button>
        {/if}
      </div>
      <p class="mt-1 text-xs text-zinc-500">
        PIN/password yang pernah disimpan dari panel kontrol. Tersimpan terenkripsi di PC ini
        (DPAPI, hanya akun Windows ini) dan tidak pernah dikirim keluar; yang tampil di sini hanya
        panjangnya.
      </p>

      {#if !keysLoaded}
        <p class="mt-3 text-xs text-zinc-500">Memuat...</p>
      {:else if keys.length === 0}
        <p class="mt-3 text-xs text-zinc-500">Belum ada kunci yang disimpan.</p>
      {:else}
        <ul class="mt-3 divide-y divide-white/5">
          {#each keys as entry (entry.id)}
            <li class="flex items-center gap-3 py-2">
              <span class="min-w-0 flex-1">
                <span class="block truncate text-xs text-zinc-200">{entry.label}</span>
                <span class="block truncate font-mono text-[10px] text-zinc-500">
                  {entry.serial} · {entry.method === 'pin' ? `${entry.length} digit` : `${entry.length} karakter`}
                </span>
              </span>
              <button
                type="button"
                onclick={() => askConfirm(entry.id, () => void removeKey(entry.id))}
                disabled={keysBusy}
                aria-label={confirmKey === entry.id
                  ? `Konfirmasi hapus ${entry.label} untuk ${entry.serial}`
                  : `Hapus ${entry.label} untuk ${entry.serial}`}
                class="shrink-0 rounded-lg border px-2 py-1 text-[11px] transition disabled:cursor-not-allowed disabled:opacity-40 {confirmKey ===
                entry.id
                  ? 'border-rose-500/40 bg-rose-500/20 text-rose-100'
                  : 'border-white/10 bg-white/5 text-zinc-400 hover:bg-white/10 hover:text-zinc-200'}"
              >
                {confirmKey === entry.id ? 'Hapus?' : '✕'}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <div class="flex items-center justify-between gap-3">
        <h2 class="text-sm font-semibold">Diagnostics</h2>
        <button
          type="button"
          onclick={() => void checkVersions()}
          disabled={checkingVersions || !diagnostics}
          title="Menjalankan adb version dan scrcpy --version"
          class="shrink-0 rounded-md border border-white/5 bg-white/[0.03] px-2 py-1 text-[10px] text-zinc-400 transition hover:bg-white/[0.06] hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {checkingVersions ? 'Memeriksa...' : 'Periksa versi'}
        </button>
      </div>
      {#if diagnostics}
        <div class="mt-3 space-y-3 text-xs">
          <div class="flex items-center gap-2.5">
            <span class="h-1.5 w-1.5 shrink-0 rounded-full {diagnostics.adb.found ? 'bg-emerald-400' : 'bg-rose-400'}"></span>
            <span class="w-16 shrink-0 text-zinc-300 @xl:w-24">ADB</span>
            <span class="min-w-0 truncate font-mono text-[10px] text-zinc-500" title={diagnostics.adb.path ?? diagnostics.adb.error ?? ''}>
              {diagnostics.adb.found ? diagnostics.adb.version ?? 'Terpasang' : diagnostics.adb.error ?? 'Tidak ditemukan'}
            </span>
          </div>
          <div class="flex items-center gap-2.5">
            <span class="h-1.5 w-1.5 shrink-0 rounded-full {diagnostics.scrcpy.found ? 'bg-emerald-400' : 'bg-rose-400'}"></span>
            <span class="w-16 shrink-0 text-zinc-300 @xl:w-24">scrcpy</span>
            <span class="min-w-0 truncate font-mono text-[10px] text-zinc-500" title={diagnostics.scrcpy.path ?? diagnostics.scrcpy.error ?? ''}>
              {diagnostics.scrcpy.found ? diagnostics.scrcpy.version ?? 'Terpasang' : diagnostics.scrcpy.error ?? 'Tidak ditemukan'}
            </span>
          </div>
          <div class="flex items-center gap-2.5">
            <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-zinc-600"></span>
            <span class="w-16 shrink-0 text-zinc-300 @xl:w-24">Konfigurasi</span>
            <span class="min-w-0 truncate font-mono text-[10px] text-zinc-500" title={diagnostics.configPath ?? ''}>
              {diagnostics.configPath ?? 'Tidak tersedia'}
            </span>
          </div>
          <div class="flex items-center gap-2.5">
            <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-zinc-600"></span>
            <span class="w-16 shrink-0 text-zinc-300 @xl:w-24">Aplikasi</span>
            <span class="min-w-0 truncate font-mono text-[10px] text-zinc-500">YAOP {diagnostics.appVersion} · {diagnostics.platform}</span>
          </div>
        </div>
      {:else}
        <p class="mt-3 text-xs text-zinc-500">Memuat diagnostics...</p>
      {/if}
    </section>
  </div>
</div>
