<script lang="ts">
  import { onMount } from 'svelte';
  import {
    listAndroidFiles,
    onTransferProgress,
    pullAndroidFile,
  } from '../api';
  import type {
    AndroidDevice,
    AndroidEntry,
    SessionInfo,
    TransferProgress,
  } from '../types';

  interface Props {
    devices: AndroidDevice[];
    session: SessionInfo | null;
    onNotice: (kind: 'info' | 'error', text: string) => void;
    onNavigate: (page: 'devices' | 'settings') => void;
    refreshKey?: number;
    deviceSerial?: string | null;
    currentDir?: string;
  }

  let {
    devices,
    session,
    onNotice,
    onNavigate,
    refreshKey = 0,
    deviceSerial = $bindable<string | null>(null),
    currentDir = $bindable('/sdcard/Download'),
  }: Props = $props();

  const shortcuts = [
    { label: 'Download', path: '/sdcard/Download' },
    { label: 'DCIM', path: '/sdcard/DCIM' },
    { label: 'Pictures', path: '/sdcard/Pictures' },
    { label: 'Movies', path: '/sdcard/Movies' },
    { label: 'Music', path: '/sdcard/Music' },
    { label: 'Documents', path: '/sdcard/Documents' },
  ];

  let entries = $state<AndroidEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let busyFile = $state<string | null>(null);
  let transfer = $state<TransferProgress | null>(null);
  let loadToken = 0;

  // A camera folder can hold thousands of entries; mounting every row at once made opening
  // it take seconds. The listing is still read whole, but rows are added a page at a time.
  const PAGE_SIZE = 200;
  let visibleCount = $state(PAGE_SIZE);

  const readyDevices = $derived(devices.filter((device) => device.state === 'device'));
  const atRoot = $derived(currentDir === '/sdcard' || currentDir === '/storage/emulated/0');
  const showTransfer = $derived(
    transfer !== null && transfer.serial === deviceSerial && transfer.direction === 'pull',
  );
  const folderCount = $derived(entries.filter((entry) => entry.isDir).length);
  const fileCount = $derived(entries.length - folderCount);
  const visibleEntries = $derived(entries.slice(0, visibleCount));
  const hiddenCount = $derived(Math.max(0, entries.length - visibleEntries.length));

  $effect(() => {
    const preferred = session?.serial ?? readyDevices[0]?.serial ?? null;
    if (!deviceSerial || !readyDevices.some((device) => device.serial === deviceSerial)) {
      deviceSerial = preferred;
    }
  });

  $effect(() => {
    const serial = deviceSerial;
    const directory = currentDir;
    void refreshKey;
    void load(serial, directory);
  });

  onMount(() => {
    let stop: (() => void) | undefined;
    void onTransferProgress((progress) => {
      if (progress.direction !== 'pull') return;
      transfer = progress;
      if (progress.done) {
        window.setTimeout(() => {
          if (transfer === progress) transfer = null;
        }, 1800);
      }
    }).then((unlisten) => (stop = unlisten));

    return () => stop?.();
  });

  async function load(serial: string | null, directory: string) {
    const token = ++loadToken;
    visibleCount = PAGE_SIZE;
    if (!serial) {
      entries = [];
      loading = false;
      error = null;
      return;
    }

    loading = true;
    error = null;
    try {
      const result = await listAndroidFiles(serial, directory);
      if (token === loadToken) entries = result;
    } catch (cause) {
      if (token === loadToken) {
        entries = [];
        error = errorText(cause);
      }
    } finally {
      if (token === loadToken) loading = false;
    }
  }

  function errorText(cause: unknown) {
    if (typeof cause === 'string') return cause;
    if (cause instanceof Error) return cause.message;
    return 'Terjadi kesalahan yang tidak diketahui.';
  }

  function refresh() {
    void load(deviceSerial, currentDir);
  }

  function openEntry(entry: AndroidEntry) {
    if (!entry.isDir) return;
    if (entry.name.includes("'") || /[\u0000-\u001f]/.test(entry.name)) {
      onNotice('error', `Folder "${entry.name}" mengandung karakter yang belum didukung.`);
      return;
    }
    currentDir = entry.path;
  }

  function goUp() {
    currentDir = parentPath(currentDir);
  }

  function parentPath(path: string) {
    const trimmed = path.replace(/\/+$/, '');
    if (trimmed === '/sdcard' || trimmed === '/storage/emulated/0') return trimmed;
    const index = trimmed.lastIndexOf('/');
    if (index <= 0) return '/sdcard';
    return trimmed.slice(0, index);
  }

  function folderName(path: string) {
    const trimmed = path.replace(/\/+$/, '');
    const name = trimmed.slice(trimmed.lastIndexOf('/') + 1);
    return name === '' ? 'Berkas HP' : name;
  }

  async function download(entry: AndroidEntry) {
    const serial = deviceSerial;
    if (!serial) return;

    busyFile = entry.path;
    try {
      const result = await pullAndroidFile(serial, entry.path);
      const size = formatSize(result.bytes);
      onNotice('info', `${entry.name} (${size}) diunduh ke ${result.localPath}`);
    } catch (cause) {
      onNotice('error', errorText(cause));
    } finally {
      busyFile = null;
    }
  }

  function formatSize(bytes: number) {
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KB', 'MB', 'GB', 'TB'];
    let value = bytes / 1024;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
  }
</script>

<header class="flex flex-wrap items-center justify-between gap-3 border-b border-white/5 px-4 py-4 @2xl:px-6 @5xl:px-8 @2xl:py-5">
  <div class="min-w-0 flex-1">
    <h1 class="text-lg font-semibold tracking-tight">Berkas</h1>
    <p class="mt-0.5 text-xs text-zinc-500">
      {#if deviceSerial}
        Unduh file HP ke PC · seret file PC ke jendela ini untuk unggah ke folder yang dibuka
      {:else}
        Jelajahi penyimpanan Android setelah perangkat terhubung
      {/if}
    </p>
  </div>
  <div class="flex flex-wrap items-center gap-2">
    {#if readyDevices.length > 1}
      <select
        bind:value={deviceSerial}
        class="rounded-lg border border-white/10 bg-zinc-900 px-3 py-1.5 text-xs text-zinc-200 outline-none focus:border-emerald-500/40"
      >
        {#each readyDevices as device (device.serial)}
          <option value={device.serial}>{device.model ?? device.serial}</option>
        {/each}
      </select>
    {/if}
    <button
      type="button"
      onclick={refresh}
      disabled={loading || !deviceSerial}
      class="rounded-lg border border-white/10 bg-white/5 px-3.5 py-1.5 text-xs font-medium text-zinc-200 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
    >
      {loading ? 'Memuat...' : 'Muat ulang'}
    </button>
  </div>
</header>

<div class="flex-1 overflow-y-auto px-4 py-5 @2xl:px-6 @5xl:px-8 @2xl:py-6">
  {#if !deviceSerial}
    <div class="rounded-2xl border border-dashed border-white/10 bg-zinc-900/20 px-4 py-10 text-center @2xl:px-8 @2xl:py-12">
      <h3 class="text-sm font-medium text-zinc-200">Belum ada perangkat terhubung</h3>
      <p class="mx-auto mt-1 max-w-md text-xs leading-relaxed text-zinc-500">
        Sambungkan HP melalui USB atau Wi-Fi, lalu kembali ke halaman Perangkat.
      </p>
      <button
        type="button"
        onclick={() => onNavigate('devices')}
        class="mt-4 rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-3.5 py-1.5 text-xs font-medium text-emerald-200 transition hover:bg-emerald-500/25"
      >
        Buka Perangkat
      </button>
    </div>
  {:else}
    {#if showTransfer}
      <div class="mb-5 rounded-xl border border-sky-500/20 bg-sky-500/[0.06] px-4 py-3">
        <div class="flex items-center justify-between gap-3 text-xs text-sky-200">
          <span class="truncate">
            {transfer?.done ? 'Terunduh' : 'Mengunduh'} {transfer?.fileName}
          </span>
          <span class="shrink-0 font-mono text-[11px] text-sky-300/70">
            {transfer?.percent}%
          </span>
        </div>
        <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-black/30">
          <div
            class="h-full rounded-full bg-sky-400 transition-[width]"
            style={`width: ${transfer?.percent ?? 0}%`}
          ></div>
        </div>
      </div>
    {/if}

    <div class="mb-3 flex flex-wrap items-center gap-3">
      <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
        <svg viewBox="0 0 20 20" class="h-[18px] w-[18px] text-amber-300/80" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round">
          <path d="M3.5 5.5a1 1 0 0 1 1-1h3l1.5 2h6a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z" />
        </svg>
      </span>
      <div class="min-w-0 flex-1">
        <h2 class="flex min-w-0 items-center gap-2 text-sm font-semibold">
          <span class="truncate">{folderName(currentDir)}</span>
          {#if !loading && entries.length > 0}
            <span class="shrink-0 rounded-full bg-white/[0.06] px-1.5 py-0.5 text-[10px] font-semibold text-zinc-400">
              {entries.length}
            </span>
          {/if}
        </h2>
        <p class="mt-0.5 truncate font-mono text-[10px] text-zinc-500" title={currentDir}>
          {currentDir}
          {#if !loading && entries.length > 0}
            <span class="text-zinc-600"> · {folderCount} folder · {fileCount} file</span>
          {/if}
        </p>
      </div>
      <button
        type="button"
        onclick={goUp}
        disabled={atRoot}
        title="Naik satu folder"
        aria-label="Naik satu folder"
        class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-white/10 bg-white/5 text-sm text-zinc-300 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-30"
      >
        ↑
      </button>
    </div>

    <div class="mb-4 flex flex-wrap gap-1.5">
      {#each shortcuts as shortcut (shortcut.path)}
        <button
          type="button"
          onclick={() => (currentDir = shortcut.path)}
          class="rounded-lg border px-3 py-1.5 text-[11px] font-medium transition {currentDir === shortcut.path
            ? 'border-emerald-500/40 bg-emerald-500/15 text-emerald-200'
            : 'border-white/10 bg-white/5 text-zinc-400 hover:bg-white/10 hover:text-zinc-200'}"
        >
          {shortcut.label}
        </button>
      {/each}
      <button
        type="button"
        onclick={() => (currentDir = '/sdcard')}
        class="rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-[11px] font-medium text-zinc-400 transition hover:bg-white/10 hover:text-zinc-200"
      >
        /sdcard
      </button>
    </div>

    {#if error}
      <div class="mb-4 flex items-start justify-between gap-4 rounded-xl border border-rose-500/20 bg-rose-500/[0.06] px-4 py-3 text-xs text-rose-200">
        <span class="leading-relaxed">{error}</span>
        <button
          type="button"
          onclick={refresh}
          class="shrink-0 rounded-md px-1.5 py-0.5 text-[10px] uppercase tracking-wider opacity-60 transition hover:opacity-100"
        >
          Coba lagi
        </button>
      </div>
    {/if}

    <div class="overflow-hidden rounded-2xl border border-white/5 bg-zinc-900/40">
      <div class="grid grid-cols-[minmax(0,1fr)_72px] items-center gap-2 border-b border-white/5 bg-black/20 px-3 py-2.5 text-[10px] uppercase tracking-wider text-zinc-600 @2xl:grid-cols-[minmax(0,1fr)_80px_72px] @2xl:gap-3 @2xl:px-4 @4xl:grid-cols-[minmax(0,1fr)_90px_150px_88px]">
        <span>Nama</span>
        <span class="hidden text-right @2xl:block">Ukuran</span>
        <span class="hidden text-right @4xl:block">Diubah</span>
        <span></span>
      </div>

      {#if loading}
        <div class="space-y-1 px-2 py-3">
          {#each Array(6) as _, i (i)}
            <div class="mx-2 h-8 animate-pulse rounded-lg bg-white/[0.03]"></div>
          {/each}
        </div>
      {:else if entries.length === 0}
        <div class="px-4 py-12 text-center">
          <p class="text-xs font-medium text-zinc-400">Folder ini kosong.</p>
          <p class="mt-1 text-[11px] text-zinc-600">Tarik file dari PC ke jendela ini untuk mengunggahnya ke sini.</p>
        </div>
      {:else}
        {#each visibleEntries as entry (entry.path)}
          <div
            role="button"
            tabindex="0"
            onclick={() => (entry.isDir ? openEntry(entry) : void download(entry))}
            onkeydown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                if (entry.isDir) openEntry(entry);
                else void download(entry);
              }
            }}
            class="grid cursor-pointer grid-cols-[minmax(0,1fr)_72px] items-center gap-2 border-b border-white/[0.03] px-3 py-2.5 text-xs transition last:border-0 hover:bg-white/[0.04] @2xl:grid-cols-[minmax(0,1fr)_80px_72px] @2xl:gap-3 @2xl:px-4 @4xl:grid-cols-[minmax(0,1fr)_90px_150px_88px]"
          >
            <div class="flex min-w-0 items-center gap-2.5">
              {#if entry.isDir}
                <svg viewBox="0 0 20 20" class="h-4 w-4 shrink-0 text-amber-400" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round">
                  <path d="M3.5 5.5a1 1 0 0 1 1-1h3l1.5 2h6a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z" />
                </svg>
              {:else}
                <svg viewBox="0 0 20 20" class="h-4 w-4 shrink-0 text-zinc-500" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round">
                  <path d="M5 2.5h6l4 4v11a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-14a1 1 0 0 1 1-1z" />
                  <path d="M11 2.5v4h4" />
                </svg>
              {/if}
              <span class="truncate {entry.isDir ? 'font-medium text-zinc-200' : 'text-zinc-300'}" title={entry.name}>
                {entry.name}
              </span>
              <span class="shrink-0 font-mono text-[10px] text-zinc-600 @2xl:hidden">
                {entry.isDir ? 'Folder' : formatSize(entry.size)}
              </span>
            </div>
            <span class="hidden text-right font-mono text-[11px] text-zinc-500 @2xl:block">
              {entry.isDir ? '—' : formatSize(entry.size)}
            </span>
            <span class="hidden text-right font-mono text-[10px] text-zinc-600 @4xl:block">{entry.modified ?? '—'}</span>
            <div class="flex justify-end">
              {#if entry.isDir}
                <span class="rounded-md px-2 py-1 text-[10px] text-zinc-500 transition group-hover:text-zinc-300">Buka →</span>
              {:else}
                <button
                  type="button"
                  onclick={(event) => {
                    event.stopPropagation();
                    void download(entry);
                  }}
                  disabled={busyFile !== null}
                  class="rounded-md border border-sky-500/30 bg-sky-500/10 px-2 py-1 text-[10px] font-medium text-sky-200 transition hover:bg-sky-500/20 disabled:cursor-not-allowed disabled:opacity-40"
                >
                  {busyFile === entry.path ? '...' : 'Unduh'}
                </button>
              {/if}
            </div>
          </div>
        {/each}
        {#if hiddenCount > 0}
          <button
            type="button"
            onclick={() => (visibleCount += PAGE_SIZE)}
            class="w-full border-t border-white/5 px-4 py-3 text-[11px] text-zinc-400 transition hover:bg-white/[0.04] hover:text-zinc-200"
          >
            Muat {Math.min(PAGE_SIZE, hiddenCount)} lagi · {hiddenCount} entri tersisa
          </button>
        {/if}
      {/if}
    </div>

    <p class="mt-4 text-[11px] leading-relaxed text-zinc-600">
      Tarik file dari PC ke jendela ini untuk mengunggah ke <span class="font-mono text-zinc-400">{currentDir}</span>.
      File yang diunduh disimpan ke folder unduhan di Setelan.
    </p>
  {/if}
</div>
