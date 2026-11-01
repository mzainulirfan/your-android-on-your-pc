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
    { value: 0, chip: 'Bawaan' },
    { value: 720, chip: '720 px' },
    { value: 1080, chip: '1080 px' },
    { value: 1440, chip: '1440 px' },
    { value: 1920, chip: '1920 px' },
  ];

  const bitrates = [
    { value: 0, chip: 'Bawaan' },
    { value: 4000, chip: '4 Mbps' },
    { value: 8000, chip: '8 Mbps' },
    { value: 16000, chip: '16 Mbps' },
    { value: 32000, chip: '32 Mbps' },
  ];

  // Layar HP zaman sekarang 90–120 Hz. Tanpa batas, bitrate yang sama dipecah ke dua kali
  // lebih banyak frame, jadi tiap frame kurang bit dan gambarnya terlihat patah-patah.
  const frameRates = [
    { value: 0, chip: 'Bawaan' },
    { value: 30, chip: '30' },
    { value: 60, chip: '60' },
    { value: 90, chip: '90' },
    { value: 120, chip: '120' },
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

  // The Save button only matters while the form differs from what is on disk.
  let savedSignature = $state<string | null>(null);
  function formSignature() {
    return [
      maxSize,
      bitrate,
      maxFps,
      downloadDir,
      closeToTray,
      mirrorScreenOff,
      lowLatency,
      audioEnabled,
    ].join('|');
  }
  const dirty = $derived(savedSignature !== null && formSignature() !== savedSignature);

  const keysByDevice = $derived.by(() => {
    const groups = new Map<string, SavedCredential[]>();
    for (const entry of keys) {
      const list = groups.get(entry.serial) ?? [];
      list.push(entry);
      groups.set(entry.serial, list);
    }
    return [...groups.entries()];
  });

  const chipBase = 'rounded-lg border px-3 py-1.5 text-[11px] font-medium transition';
  const chipOn = 'border-emerald-500/40 bg-emerald-500/15 text-emerald-200';
  const chipOff = 'border-white/10 bg-white/5 text-zinc-400 hover:bg-white/10 hover:text-zinc-200';

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
      savedSignature = formSignature();
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
      savedSignature = formSignature();
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

<header class="flex flex-wrap items-center justify-between gap-3 border-b border-white/5 px-4 py-5 @2xl:px-6 @5xl:px-8 @2xl:py-6">
  <div class="min-w-0 flex-1">
    <div class="flex flex-wrap items-center gap-2">
      <h1 class="text-lg font-semibold tracking-tight">Setelan</h1>
      {#if dirty}
        <span class="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-2.5 py-0.5 text-[10px] font-medium text-amber-200 ring-1 ring-inset ring-amber-500/30">
          <span class="h-1.5 w-1.5 rounded-full bg-amber-400"></span>
          Belum disimpan
        </span>
      {/if}
    </div>
    <p class="mt-1 text-xs text-zinc-500">
      Kualitas mirroring, folder unduhan, perilaku jendela, kunci tersimpan
    </p>
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
      disabled={saving || loading || !dirty}
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
      <div class="flex flex-wrap items-center gap-3">
        <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-[18px] w-[18px] text-emerald-300/80" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2.5" y="3.5" width="15" height="10" rx="1.5" />
            <path d="M7 16.5h6M10 13.5v3" />
          </svg>
        </span>
        <div class="min-w-0 flex-1">
          <h2 class="text-sm font-semibold">Kualitas mirroring</h2>
          <p class="mt-0.5 text-[11px] text-zinc-500">
            Berlaku pada sesi scrcpy berikutnya; sesi berjalan tidak berubah.
          </p>
        </div>
      </div>

      <div class="mt-4 space-y-5 rounded-xl border border-white/5 bg-zinc-950/40 p-4">
        <div>
          <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <span class="text-[11px] font-medium uppercase tracking-wider text-zinc-500">Resolusi maksimum</span>
            <span class="text-[10px] text-zinc-600">sisi terpanjang</span>
          </div>
          <div class="mt-2 flex flex-wrap gap-2" role="radiogroup" aria-label="Resolusi maksimum">
            {#each resolutions as option (option.value)}
              <button
                type="button"
                role="radio"
                aria-checked={maxSize === option.value}
                onclick={() => (maxSize = option.value)}
                class={`${chipBase} ${maxSize === option.value ? chipOn : chipOff}`}
              >
                {option.chip}
              </button>
            {/each}
          </div>
          <p class="mt-2 text-[10px] leading-relaxed text-zinc-600">
            Bawaan = resolusi asli HP; lewat Wi-Fi dibatasi 1920 px supaya bandwidth tidak meledak saat scroll.
          </p>
        </div>

        <div class="border-t border-white/5 pt-5">
          <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <span class="text-[11px] font-medium uppercase tracking-wider text-zinc-500">Batas FPS</span>
            <span class="text-[10px] text-zinc-600">frame per detik</span>
          </div>
          <div class="mt-2 flex flex-wrap gap-2" role="radiogroup" aria-label="Batas FPS">
            {#each frameRates as option (option.value)}
              <button
                type="button"
                role="radio"
                aria-checked={maxFps === option.value}
                onclick={() => (maxFps = option.value)}
                class={`${chipBase} ${maxFps === option.value ? chipOn : chipOff}`}
              >
                {option.chip}
              </button>
            {/each}
          </div>
          <p class="mt-2 text-[10px] leading-relaxed text-zinc-600">
            Layar HP 90–120 Hz. Tanpa batas, bitrate yang sama dipecah ke dua kali lebih banyak frame — tiap frame
            kurang bit dan gambar terlihat patah.
          </p>
        </div>

        <div class="border-t border-white/5 pt-5">
          <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <span class="text-[11px] font-medium uppercase tracking-wider text-zinc-500">Bitrate video</span>
            <span class="text-[10px] text-zinc-600">kualitas aliran</span>
          </div>
          <div class="mt-2 flex flex-wrap gap-2" role="radiogroup" aria-label="Bitrate video">
            {#each bitrates as option (option.value)}
              <button
                type="button"
                role="radio"
                aria-checked={bitrate === option.value}
                onclick={() => (bitrate = option.value)}
                class={`${chipBase} ${bitrate === option.value ? chipOn : chipOff}`}
              >
                {option.chip}
              </button>
            {/each}
          </div>
          <p class="mt-2 text-[10px] leading-relaxed text-zinc-600">
            Naikkan bersama resolusi; turunkan saat jaringan Wi-Fi padat.
          </p>
        </div>
      </div>

      <div class="mt-4 rounded-xl border border-white/5 bg-zinc-950/40 px-4">
        <div class="flex items-start gap-3 border-b border-white/5 py-4">
          <div class="min-w-0 flex-1">
            <p class="text-xs font-semibold text-zinc-200">Prioritas latensi rendah (Wi-Fi)</p>
            <p class="mt-1 text-[11px] leading-relaxed text-zinc-500">
              Dibatasi sampai <span class="font-mono">1080 px · 60 FPS · 4 Mbps</span>. Yang diperbaiki bukan
              jaringannya (terukur masih 9–12 ms), tapi beban <span class="text-zinc-400">encode di HP</span>: 1080
              px berarti sepertiga pixel dibanding 1920 px, jadi animasi action center dan daftar aplikasi tidak
              berebut GPU dengan mirror. Nilai ini hanya <span class="text-zinc-400">menurunkan</span> pilihan di
              atas, tidak pernah menaikkannya.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={lowLatency}
            aria-label="Prioritas latensi rendah"
            onclick={() => (lowLatency = !lowLatency)}
            class="relative h-6 w-10 shrink-0 rounded-full transition-colors {lowLatency
              ? 'bg-emerald-500/80'
              : 'bg-white/10'}"
          >
            <span class="absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform {lowLatency
              ? 'translate-x-4'
              : ''}"></span>
          </button>
        </div>

        <div class="flex items-start gap-3 border-b border-white/5 py-4">
          <div class="min-w-0 flex-1">
            <p class="text-xs font-semibold text-zinc-200">Mulai mirroring dengan layar HP tetap mati</p>
            <p class="mt-1 text-[11px] leading-relaxed text-zinc-500">
              Semua tampilan dan kontrol hanya lewat cermin, input tidak membangunkan layar HP (butuh Android 8.1+).
              <span class="text-zinc-400">Hanya berlaku lewat kabel USB</span> — saat layar mati di Wi-Fi, radio
              tertidur dan latensi naik 87 ms → 146 ms, mirror justru patah-patah. Tekan
              <span class="font-mono">Alt+B</span> di tengah sesi kalau tetap ingin mematikan layar.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={mirrorScreenOff}
            aria-label="Mulai mirroring dengan layar HP mati"
            onclick={() => (mirrorScreenOff = !mirrorScreenOff)}
            class="relative h-6 w-10 shrink-0 rounded-full transition-colors {mirrorScreenOff
              ? 'bg-emerald-500/80'
              : 'bg-white/10'}"
          >
            <span class="absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform {mirrorScreenOff
              ? 'translate-x-4'
              : ''}"></span>
          </button>
        </div>

        <div class="flex items-start gap-3 py-4">
          <div class="min-w-0 flex-1">
            <p class="text-xs font-semibold text-zinc-200">Suara HP ikut ke PC</p>
            <p class="mt-1 text-[11px] leading-relaxed text-zinc-500">
              Audio HP (game, video, notifikasi) mengalir ke PC lewat sesi mirror sebagai stream Opus ±128 kbps; HP
              tetap mengeluarkan suaranya sendiri. Butuh <span class="text-zinc-400">Android 13 ke atas</span>; di
              bawah itu sesi tetap berjalan tanpa suara.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={audioEnabled}
            aria-label="Suara HP ikut ke PC"
            onclick={() => (audioEnabled = !audioEnabled)}
            class="relative h-6 w-10 shrink-0 rounded-full transition-colors {audioEnabled
              ? 'bg-emerald-500/80'
              : 'bg-white/10'}"
          >
            <span class="absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform {audioEnabled
              ? 'translate-x-4'
              : ''}"></span>
          </button>
        </div>
      </div>
    </section>

    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <div class="flex flex-wrap items-center gap-3">
        <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-[18px] w-[18px] text-sky-300/80" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M2.5 5.5A1.5 1.5 0 0 1 4 4h4l1.6 1.8H16a1.5 1.5 0 0 1 1.5 1.5v7A1.5 1.5 0 0 1 16 15.8H4a1.5 1.5 0 0 1-1.5-1.5z" />
            <path d="M10 8.2v4M8 10.6l2 2 2-2" />
          </svg>
        </span>
        <div class="min-w-0 flex-1">
          <h2 class="text-sm font-semibold">Folder unduhan PC</h2>
          <p class="mt-0.5 text-[11px] text-zinc-500">
            Tujuan file yang diunduh dari HP dan screenshot. Folder harus sudah ada.
          </p>
        </div>
      </div>
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
      <div class="flex flex-wrap items-center gap-3">
        <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-[18px] w-[18px] text-zinc-400" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2.5" y="3.5" width="15" height="13" rx="1.5" />
            <path d="M2.5 7h15" />
          </svg>
        </span>
        <div class="min-w-0 flex-1">
          <h2 class="text-sm font-semibold">Perilaku jendela</h2>
        </div>
      </div>
      <div class="mt-3 flex items-start gap-3 rounded-xl border border-white/5 bg-zinc-950/40 px-4 py-4">
        <div class="min-w-0 flex-1">
          <p class="text-xs font-semibold text-zinc-200">Tutup (X) = sembunyi ke tray</p>
          <p class="mt-1 text-[11px] leading-relaxed text-zinc-500">
            Sesi mirroring dan jendela scrcpy tetap berjalan di belakang. Klik kanan ikon tray untuk memunculkan
            kembali atau keluar penuh. Jika dimatikan, menutup jendela menghentikan sesi dan keluar dari aplikasi.
          </p>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={closeToTray}
          aria-label="Sembunyi ke tray saat jendela ditutup"
          onclick={() => (closeToTray = !closeToTray)}
          class="relative h-6 w-10 shrink-0 rounded-full transition-colors {closeToTray
            ? 'bg-emerald-500/80'
            : 'bg-white/10'}"
        >
          <span class="absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform {closeToTray
            ? 'translate-x-4'
            : ''}"></span>
        </button>
      </div>
    </section>

    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <div class="flex flex-wrap items-start gap-3">
        <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-[18px] w-[18px] text-amber-300/80" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <rect x="4" y="8.5" width="12" height="8" rx="1.5" />
            <path d="M6.8 8.5v-2a3.2 3.2 0 0 1 6.4 0v2" />
          </svg>
        </span>
        <div class="min-w-0 flex-1">
          <h2 class="flex items-center gap-2 text-sm font-semibold">
            Kunci tersimpan
            {#if keys.length > 0}
              <span class="rounded-full bg-white/[0.06] px-1.5 py-0.5 text-[10px] font-semibold text-zinc-400">
                {keys.length}
              </span>
            {/if}
          </h2>
          <p class="mt-0.5 text-[11px] leading-relaxed text-zinc-500">
            PIN/password yang pernah disimpan dari panel kunci. Terenkripsi DPAPI — hanya terbaca oleh akun Windows
            ini, tidak pernah dikirim keluar; yang tampil di sini hanya panjangnya.
          </p>
        </div>
        {#if keys.length > 0}
          <button
            type="button"
            onclick={() => askConfirm('all', () => void wipeKeys())}
            disabled={keysBusy}
            class="shrink-0 rounded-lg border px-3 py-1.5 text-[11px] font-medium transition disabled:cursor-not-allowed disabled:opacity-40 {confirmKey ===
            'all'
              ? 'border-rose-500/40 bg-rose-500/20 text-rose-100'
              : 'border-rose-500/25 bg-rose-500/10 text-rose-200 hover:bg-rose-500/20'}"
          >
            {confirmKey === 'all' ? `Yakin hapus ${keys.length} kunci?` : 'Hapus semua'}
          </button>
        {/if}
      </div>

      {#if !keysLoaded}
        <p class="mt-4 text-xs text-zinc-500">Memuat...</p>
      {:else if keys.length === 0}
        <p class="mt-4 rounded-xl border border-dashed border-white/10 px-4 py-6 text-center text-xs text-zinc-600">
          Belum ada kunci yang disimpan. Centang <span class="text-zinc-400">Ingat</span> lalu Kirim di panel kunci
          untuk menyimpannya.
        </p>
      {:else}
        <div class="mt-4 space-y-3">
          {#each keysByDevice as [serial, entries] (serial)}
            <div class="rounded-xl border border-white/5 bg-zinc-950/40 px-4">
              <p class="border-b border-white/5 py-2.5 font-mono text-[10px] tracking-wider text-zinc-500">
                {serial}
                <span class="ml-1 text-zinc-600">· {entries.length} kunci</span>
              </p>
              <ul class="divide-y divide-white/5">
                {#each entries as entry (entry.id)}
                  <li class="flex items-center gap-3 py-2.5">
                    <span class="shrink-0 rounded bg-white/[0.05] px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wider {entry.method ===
                      'pin'
                      ? 'text-sky-300/80'
                      : 'text-violet-300/80'}">
                      {entry.method === 'pin' ? 'PIN' : 'Password'}
                    </span>
                    <span class="min-w-0 flex-1">
                      <span class="block truncate text-xs text-zinc-200">{entry.label}</span>
                      <span class="block text-[10px] text-zinc-600">
                        {entry.length} {entry.method === 'pin' ? 'digit' : 'karakter'}
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
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <div class="flex flex-wrap items-center gap-3">
        <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-[18px] w-[18px] text-zinc-400" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M2.5 10.5h4l1.8-4.5 2.6 8 2-6 1.3 2.5h3.3" />
          </svg>
        </span>
        <div class="min-w-0 flex-1">
          <h2 class="text-sm font-semibold">Diagnostics</h2>
          <p class="mt-0.5 text-[11px] text-zinc-500">Versi dan lokasi ADB, scrcpy, berkas konfigurasi.</p>
        </div>
        <button
          type="button"
          onclick={() => void checkVersions()}
          disabled={checkingVersions || !diagnostics}
          title="Menjalankan adb version dan scrcpy --version"
          class="shrink-0 rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-[11px] text-zinc-300 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {checkingVersions ? 'Memeriksa...' : 'Periksa versi'}
        </button>
      </div>

      {#if diagnostics}
        {#snippet toolRow(label: string, detail: string, sub: string | null, ok: boolean | null)}
          <div class="flex items-start gap-3 rounded-xl border border-white/5 bg-zinc-950/40 px-4 py-3">
            <span class="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full {ok === null
              ? 'bg-zinc-600'
              : ok
                ? 'bg-emerald-400'
                : 'bg-rose-400'}"></span>
            <div class="min-w-0 flex-1">
              <p class="text-xs font-medium text-zinc-200">{label}</p>
              <p class="text-[11px] text-zinc-400">{detail}</p>
              {#if sub}
                <p class="mt-0.5 truncate font-mono text-[10px] text-zinc-600" title={sub}>{sub}</p>
              {/if}
            </div>
          </div>
        {/snippet}
        <div class="mt-4 grid gap-2.5 @3xl:grid-cols-2">
          {@render toolRow(
            'ADB',
            diagnostics.adb.found ? (diagnostics.adb.version ?? 'Terpasang') : 'Tidak ditemukan',
            diagnostics.adb.path ?? diagnostics.adb.error ?? null,
            diagnostics.adb.found,
          )}
          {@render toolRow(
            'scrcpy',
            diagnostics.scrcpy.found ? (diagnostics.scrcpy.version ?? 'Terpasang') : 'Tidak ditemukan',
            diagnostics.scrcpy.path ?? diagnostics.scrcpy.error ?? null,
            diagnostics.scrcpy.found,
          )}
          {@render toolRow('Konfigurasi', 'Folder penyimpanan aplikasi', diagnostics.configPath ?? null, null)}
          {@render toolRow('Aplikasi', `YAOP ${diagnostics.appVersion} · ${diagnostics.platform}`, null, null)}
        </div>
      {:else}
        <p class="mt-4 text-xs text-zinc-500">Memuat diagnostics...</p>
      {/if}
    </section>
  </div>
</div>
