<script lang="ts">
  import type {
    AdbStatus,
    AndroidDevice,
    ScrcpyStatus,
    SessionInfo,
    TransferProgress,
  } from '../types';
  import {
    connectionLabel,
    deviceName,
    guidanceFor,
    isReady,
    splitAddress,
    stateStyle,
  } from '../deviceFormat';

  interface Props {
    devices: AndroidDevice[];
    adb: AdbStatus | null;
    scrcpy: ScrcpyStatus | null;
    session: SessionInfo | null;
    loading: boolean;
    refreshing: boolean;
    error: string | null;
    transfer: TransferProgress | null;
    notice: { kind: 'info' | 'error'; text: string } | null;
    knownWireless: string[];
    startingSerial: string | null;
    wifiBusy: string | null;
    wifiConnecting: boolean;
    screenshotBusy: boolean;
    imageBusy: boolean;
    screenBusy: 'sleep' | 'wake' | null;
    screenOn: boolean | null;
    screenQueryBusy: boolean;
    onRefresh: () => void;
    onConnect: (serial: string) => void;
    onDisconnect: () => void;
    onScreenshot: (serial: string) => void;
    onScreenPower: (serial: string, sleep: boolean) => void;
    onClipboardImage: (serial: string) => void;
    onEnableWireless: (serial: string) => void;
    onConnectAddress: (address: string) => void;
    onForgetAddress: (address: string) => void;
    onDismissNotice: () => void;
  }

  let {
    devices,
    adb,
    scrcpy,
    session,
    loading,
    refreshing,
    error,
    transfer,
    notice,
    knownWireless,
    startingSerial,
    wifiBusy,
    wifiConnecting,
    screenshotBusy,
    imageBusy,
    screenBusy,
    screenOn,
    screenQueryBusy,
    onRefresh,
    onConnect,
    onDisconnect,
    onScreenshot,
    onScreenPower,
    onClipboardImage,
    onEnableWireless,
    onConnectAddress,
    onForgetAddress,
    onDismissNotice,
  }: Props = $props();

  let addressDraft = $state('');

  const readyDevices = $derived(devices.filter(isReady));
  const liveWireless = $derived(
    readyDevices.filter((device) => device.connectionType === 'wifi'),
  );

  // The row you act on first should be the first one you see: the mirrored device, then
  // anything usable, then whatever ADB is refusing.
  const orderedDevices = $derived.by(() => {
    const rank = (device: AndroidDevice) =>
      session?.serial === device.serial ? 0 : isReady(device) ? 1 : 2;
    return [...devices].sort((a, b) => rank(a) - rank(b) || a.serial.localeCompare(b.serial));
  });

  // The live device gets its own section, so the roster below never repeats it.
  const liveDevice = $derived(
    session ? (devices.find((device) => device.serial === session.serial) ?? null) : null,
  );
  const readyOnly = $derived(
    orderedDevices.filter((device) => isReady(device) && device.serial !== session?.serial),
  );
  const attentionOnly = $derived(
    orderedDevices.filter((device) => !isReady(device) && device.serial !== session?.serial),
  );

  const readyCount = $derived(readyDevices.length);

  const screenToggleBusy = $derived(screenBusy !== null || screenQueryBusy);
  const screenToggleLabel = $derived(
    screenBusy === 'sleep'
      ? 'Mematikan...'
      : screenBusy === 'wake'
        ? 'Membangunkan...'
        : screenQueryBusy
          ? 'Memeriksa layar...'
          : screenOn === null
            ? 'Kontrol layar'
            : screenOn
              ? 'Matikan layar HP'
              : 'Bangunkan layar HP',
  );

  function submitAddress(event: SubmitEvent) {
    event.preventDefault();
    const target = addressDraft.trim();
    if (target === '') return;
    onConnectAddress(target);
  }

  function detailOf(device: AndroidDevice): string {
    const parts = [device.serial];
    if (device.product && device.product !== device.model) parts.push(device.product);
    if (device.deviceCodename && device.deviceCodename !== device.product) {
      parts.push(device.deviceCodename);
    }
    if (session?.serial === device.serial) parts.push(`pid ${session.pid}`);
    return parts.join(' · ');
  }

  const setupSteps = [
    { n: '01', text: 'HP: Settings → Opsi Pengembang → USB debugging' },
    { n: '02', text: 'Pakai kabel data, bukan kabel khusus charge' },
    { n: '03', text: 'Setujui dialog izin yang muncul di layar HP' },
  ];
</script>

{#snippet deviceRow(device: AndroidDevice)}
  {@const live = session?.serial === device.serial}
  {@const usable = isReady(device)}
  {@const hint = guidanceFor(device)}
  <li class="px-4 py-4 transition @2xl:px-5 {live ? 'bg-emerald-500/[0.04]' : 'hover:bg-white/[0.015]'}">
    <div class="flex flex-wrap items-start gap-x-4 gap-y-3">
      <div
        class="relative flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border {live
          ? 'border-emerald-500/25 bg-emerald-500/10'
          : 'border-white/5 bg-zinc-950/60'}"
      >
        <svg
          viewBox="0 0 20 20"
          class="h-5 w-5 {live ? 'text-emerald-300' : usable ? 'text-zinc-400' : 'text-zinc-600'}"
          fill="none"
          stroke="currentColor"
          stroke-width="1.4"
          stroke-linecap="round"
        >
          <rect x="6" y="2.5" width="8" height="15" rx="2" />
          <path d="M9 15.2h2" />
        </svg>
        <span
          class="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full ring-2 ring-zinc-900 {live
            ? 'bg-emerald-400'
            : stateStyle(device.state).dot}"
        ></span>
      </div>

      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <h3 class="truncate text-[13px] font-semibold @2xl:text-sm">{deviceName(device)}</h3>
          {#if !live}
            <span class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-medium ring-1 ring-inset {stateStyle(device.state).badge}">
              <span class="h-1.5 w-1.5 rounded-full {stateStyle(device.state).dot}"></span>
              {stateStyle(device.state).label}
            </span>
          {/if}
          <span class="rounded bg-white/[0.05] px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wider text-zinc-500">
            {connectionLabel(device)}
          </span>
        </div>
        <p class="mt-1 truncate font-mono text-[11px] text-zinc-500" title={detailOf(device)}>
          {detailOf(device)}
        </p>
        {#if hint}
          <p class="mt-1.5 max-w-xl text-[11px] leading-relaxed text-amber-200/80">{hint}</p>
        {/if}
      </div>

      <div class="flex shrink-0 flex-wrap items-center gap-2 basis-full justify-start pl-14 @3xl:basis-auto @3xl:justify-end @3xl:pl-0">
        {#if live}
          <button
            type="button"
            onclick={() => onScreenPower(device.serial, screenOn === true)}
            disabled={screenToggleBusy}
            title={screenOn === true
              ? 'Matikan layar fisik HP; mirroring dan kontrol tetap jalan'
              : 'Bangunkan layar fisik HP'}
            class="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-3 py-1.5 text-[11px] font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {screenToggleLabel}
          </button>
          <button
            type="button"
            onclick={() => onScreenshot(device.serial)}
            disabled={screenshotBusy}
            title="Simpan screenshot ke folder unduhan PC"
            class="rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-[11px] font-medium text-zinc-200 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {screenshotBusy ? 'Mengambil...' : 'Screenshot'}
          </button>
          <button
            type="button"
            onclick={() => onClipboardImage(device.serial)}
            disabled={imageBusy}
            title="Kirim gambar dari clipboard PC lalu buka lembar Bagikan Android"
            class="rounded-lg border border-sky-500/30 bg-sky-500/10 px-3 py-1.5 text-[11px] font-medium text-sky-200 transition hover:bg-sky-500/20 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {imageBusy ? 'Mengirim...' : 'Kirim gambar clipboard'}
          </button>
          <button
            type="button"
            onclick={onDisconnect}
            class="rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-1.5 text-[11px] font-medium text-rose-200 transition hover:bg-rose-500/20"
          >
            Putuskan
          </button>
        {:else if usable}
          {#if device.connectionType === 'usb'}
            <button
              type="button"
              onclick={() => onEnableWireless(device.serial)}
              disabled={session !== null || startingSerial !== null || wifiBusy !== null}
              title={session ? 'Hentikan sesi aktif terlebih dahulu' : 'Aktifkan ADB via Wi-Fi agar kabel bisa dicabut'}
              class="rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-[11px] font-medium text-zinc-300 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {wifiBusy === device.serial ? 'Mengaktifkan...' : 'Aktifkan Wi-Fi'}
            </button>
          {:else}
            <button
              type="button"
              onclick={() => onForgetAddress(device.serial)}
              title="Putuskan koneksi adb connect dan hapus alamat dari daftar otomatis"
              class="rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-[11px] font-medium text-zinc-300 transition hover:bg-white/10"
            >
              Lepas & lupa
            </button>
          {/if}
          <button
            type="button"
            onclick={() => onConnect(device.serial)}
            disabled={session !== null || startingSerial !== null || wifiBusy !== null || !scrcpy?.found}
            title={!scrcpy?.found
              ? 'scrcpy tidak ditemukan'
              : session
                ? 'Hentikan sesi aktif terlebih dahulu'
                : ''}
            class="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-4 py-1.5 text-[11px] font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {startingSerial === device.serial ? 'Membuka...' : 'Mulai mirroring'}
          </button>
        {:else}
          <button
            type="button"
            onclick={onRefresh}
            class="rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-[11px] font-medium text-zinc-300 transition hover:bg-white/10"
          >
            Coba lagi
          </button>
        {/if}
      </div>
    </div>
  </li>
{/snippet}

<header class="flex flex-wrap items-end justify-between gap-3 border-b border-white/5 px-4 py-5 @2xl:px-6 @5xl:px-8 @2xl:py-6">
  <div class="min-w-0">
    <h1 class="text-2xl font-semibold tracking-tight">Perangkat</h1>
    <p class="mt-1 truncate text-xs text-zinc-500">
      {#if loading}
        Memindai perangkat melalui ADB...
      {:else if devices.length === 0}
        Belum ada perangkat terdeteksi
      {:else if attentionOnly.length > 0}
        {readyCount} siap · {attentionOnly.length} perlu perhatian
      {:else}
        {readyCount} perangkat siap dikontrol
      {/if}
    </p>
  </div>
  <div class="flex shrink-0 flex-wrap items-center gap-2 text-[11px]">
    <span
      class="hidden items-center gap-1.5 rounded-full border border-white/10 bg-white/[0.04] px-2.5 py-1 text-zinc-500 @2xl:inline-flex"
      title="Daftar diperbarui otomatis; lebih lambat saat mirroring berjalan atau jendela disembunyikan"
    >
      <span class="h-1.5 w-1.5 rounded-full bg-emerald-400/70"></span>
      Pindai otomatis
    </span>
    {#if session}
      <span class="inline-flex items-center gap-2 rounded-full border border-emerald-500/25 bg-emerald-500/10 px-3 py-1 font-medium text-emerald-300">
        <span class="relative flex h-1.5 w-1.5">
          <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60"></span>
          <span class="relative inline-flex h-1.5 w-1.5 rounded-full bg-emerald-400"></span>
        </span>
        Live
      </span>
    {:else if devices.length > 0}
      <span class="rounded-full border border-white/10 bg-white/[0.04] px-2.5 py-1 text-zinc-400">
        {readyCount}/{devices.length} siap
      </span>
    {/if}
    <button
      type="button"
      onclick={onRefresh}
      disabled={refreshing}
      class="rounded-lg border border-white/10 bg-white/5 px-3.5 py-1.5 text-xs font-medium text-zinc-200 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
    >
      {refreshing ? 'Memindai...' : 'Muat ulang'}
    </button>
  </div>
</header>

<div class="flex-1 overflow-y-auto px-4 py-5 @2xl:px-6 @5xl:px-8 @2xl:py-6">
  <div class="mx-auto max-w-5xl space-y-4">
    {#if transfer}
      <section class="rounded-xl border border-sky-500/20 bg-sky-500/[0.06] px-4 py-3">
        <div class="flex items-center justify-between gap-3 text-xs text-sky-200">
          <span class="truncate">
            {transfer.direction === 'pull'
              ? transfer.done ? 'Terunduh' : 'Mengunduh'
              : transfer.done ? 'Terkirim' : 'Mengirim'}
            {transfer.fileName}
          </span>
          <span class="shrink-0 font-mono text-[11px] text-sky-300/70">
            {transfer.index}/{transfer.total} · {transfer.percent}%
          </span>
        </div>
        <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-black/30">
          <div class="h-full rounded-full bg-sky-400 transition-[width]" style={`width: ${transfer.percent}%`}></div>
        </div>
      </section>
    {/if}

    {#if notice}
      <section
        class="flex items-start justify-between gap-4 rounded-xl border px-4 py-3 text-xs {notice.kind === 'error'
          ? 'border-rose-500/20 bg-rose-500/[0.06] text-rose-200'
          : 'border-sky-500/20 bg-sky-500/[0.06] text-sky-200'}"
      >
        <span class="leading-relaxed">{notice.text}</span>
        <button
          type="button"
          onclick={onDismissNotice}
          class="shrink-0 rounded-md px-1.5 py-0.5 text-[10px] uppercase tracking-wider opacity-60 transition hover:opacity-100"
        >
          Tutup
        </button>
      </section>
    {/if}

    {#if adb && !adb.found}
      <section class="rounded-2xl border border-amber-500/20 bg-amber-500/[0.05] px-5 py-4">
        <div class="flex items-center gap-2.5">
          <span class="h-1.5 w-1.5 rounded-full bg-amber-400"></span>
          <h2 class="text-sm font-semibold text-amber-200">ADB belum ditemukan</h2>
        </div>
        <p class="mt-1.5 max-w-2xl text-xs leading-relaxed text-amber-100/70">
          {adb.error ?? 'Aplikasi membutuhkan Android Debug Bridge untuk mendeteksi perangkat.'}
        </p>
        <p class="mt-2 text-[11px] leading-relaxed text-amber-100/60">
          Pasang Android SDK Platform Tools, arahkan
          <span class="font-mono">ANDROID_HOME</span> ke instalasinya, atau salin
          <span class="font-mono">adb.exe</span> ke <span class="font-mono">binaries/adb/</span>.
        </p>
      </section>
    {/if}

    {#if error && adb?.found}
      <p class="rounded-xl border border-rose-500/20 bg-rose-500/[0.06] px-4 py-3 text-xs leading-relaxed text-rose-200">
        {error}
      </p>
    {/if}

    {#if loading}
      <section class="rounded-2xl border border-white/5 bg-zinc-900/40 px-5 py-12 text-center">
        <p class="text-xs text-zinc-500">Memindai perangkat...</p>
      </section>
    {:else if devices.length === 0}
      <section class="overflow-hidden rounded-2xl border border-dashed border-white/10 bg-zinc-900/30 px-6 py-10 text-center">
        <div class="mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-white/[0.03] ring-1 ring-white/5">
          <svg viewBox="0 0 20 20" class="h-7 w-7 text-zinc-600" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
            <rect x="6" y="2.5" width="8" height="15" rx="2" />
            <path d="M9 15.2h2" />
          </svg>
        </div>
        <h2 class="mt-4 text-sm font-medium text-zinc-200">Belum ada perangkat</h2>
        <p class="mx-auto mt-1 max-w-md text-xs leading-relaxed text-zinc-500">
          HP yang pernah diaktifkan ADB-nya lewat kabel bisa dihubungkan tanpa kabel melalui alamat
          IP pada panel “Koneksi Wi-Fi” di bawah.
        </p>
        <ol class="mx-auto mt-6 flex max-w-2xl flex-col gap-2 text-left @2xl:flex-row">
          {#each setupSteps as step (step.n)}
            <li class="flex flex-1 items-start gap-2.5 rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2.5">
              <span class="font-mono text-[10px] text-emerald-400/80">{step.n}</span>
              <span class="text-[11px] leading-relaxed text-zinc-400">{step.text}</span>
            </li>
          {/each}
        </ol>
        <button
          type="button"
          onclick={onRefresh}
          class="mt-6 rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-xs font-medium text-zinc-200 transition hover:bg-white/10"
        >
          Pindai ulang sekarang
        </button>
      </section>
    {:else}
      {#if session}
        <section class="overflow-hidden rounded-2xl border border-emerald-500/20 bg-zinc-900/40">
          <div class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 border-b border-white/5 px-5 py-3.5">
            <div class="flex items-center gap-2">
              <span class="relative flex h-2 w-2">
                <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-50"></span>
                <span class="relative inline-flex h-2 w-2 rounded-full bg-emerald-400"></span>
              </span>
              <h2 class="text-sm font-semibold text-emerald-200">Sesi aktif</h2>
            </div>
            {#if screenOn !== null}
              <p class="text-[11px] text-zinc-500">
                Layar {screenOn ? 'menyala' : 'mati'} · mirroring berjalan
              </p>
            {:else}
              <p class="text-[11px] text-zinc-500">Mirroring berjalan</p>
            {/if}
          </div>
          {#if liveDevice}
            <ul class="divide-y divide-white/5">
              {@render deviceRow(liveDevice)}
            </ul>
          {:else}
            <div class="flex flex-wrap items-center justify-between gap-3 px-5 py-4">
              <p class="text-xs leading-relaxed text-zinc-400">
                Perangkat sesi ini tidak muncul di daftar ADB. Cermin mungkin masih berjalan
                sebentar; putuskan sesi kalau HP sudah tidak terhubung.
              </p>
              <button
                type="button"
                onclick={onDisconnect}
                class="rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-1.5 text-[11px] font-medium text-rose-200 transition hover:bg-rose-500/20"
              >
                Putuskan sesi
              </button>
            </div>
          {/if}
        </section>
      {/if}

      {#if readyOnly.length > 0}
        <section class="overflow-hidden rounded-2xl border border-white/5 bg-zinc-900/40">
          <div class="flex flex-wrap items-center justify-between gap-2 border-b border-white/5 px-5 py-3.5">
            <h2 class="text-sm font-semibold">Siap dipakai</h2>
            <p class="text-[11px] text-zinc-500">
              {readyOnly.length} perangkat siap dicerminkan
            </p>
          </div>
          <ul class="divide-y divide-white/5">
            {#each readyOnly as device (device.serial)}
              {@render deviceRow(device)}
            {/each}
          </ul>
        </section>
      {/if}

      {#if attentionOnly.length > 0}
        <section class="overflow-hidden rounded-2xl border border-amber-500/15 bg-zinc-900/40">
          <div class="flex flex-wrap items-center justify-between gap-2 border-b border-amber-500/10 px-5 py-3.5">
            <h2 class="text-sm font-semibold text-amber-200">Perlu perhatian</h2>
            <p class="text-[11px] text-amber-200/60">
              {attentionOnly.length} perangkat belum bisa dicerminkan
            </p>
          </div>
          <ul class="divide-y divide-white/5">
            {#each attentionOnly as device (device.serial)}
              {@render deviceRow(device)}
            {/each}
          </ul>
        </section>
      {/if}
    {/if}

    <!-- Wireless transport -->
    <section class="overflow-hidden rounded-2xl border border-white/5 bg-zinc-900/40">
      <div class="flex flex-wrap items-center justify-between gap-2 border-b border-white/5 px-5 py-3.5">
        <div class="flex items-center gap-2.5">
          <svg viewBox="0 0 20 20" class="h-4 w-4 text-zinc-500" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
            <path d="M2.8 7.8a10 10 0 0 1 14.4 0M5.4 10.6a6.5 6.5 0 0 1 9.2 0M8 13.4a3 3 0 0 1 4 0" />
            <circle cx="10" cy="16" r="0.9" fill="currentColor" stroke="none" />
          </svg>
          <h2 class="text-sm font-semibold">Koneksi Wi-Fi</h2>
        </div>
        <span class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-medium ring-1 ring-inset {liveWireless.length > 0
            ? 'bg-emerald-500/10 text-emerald-300 ring-emerald-500/20'
            : 'bg-white/[0.04] text-zinc-400 ring-white/10'}">
          <span class="h-1.5 w-1.5 rounded-full {liveWireless.length > 0 ? 'bg-emerald-400' : 'bg-zinc-500'}"></span>
          {liveWireless.length > 0 ? `${liveWireless.length} aktif` : 'Belum aktif'}
        </span>
      </div>

      <div class="grid gap-5 px-4 py-4 @2xl:px-5 @3xl:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)]">
        <div class="space-y-3">
          <p class="max-w-2xl text-[11px] leading-relaxed text-zinc-500">
            Tulis alamat IP HP (Settings → Wi-Fi → jaringan → IP); port 5555 ditambahkan otomatis.
            Mode <span class="font-mono">adb tcpip</span> hilang saat ADB di HP restart atau HP
            menyala ulang — kalau daftar perangkat kosong padahal HP baik-baik saja, hubungkan lagi
            lewat alamat tersimpan.
          </p>

          <form onsubmit={submitAddress}>
            <label for="wifi-address" class="text-[10px] font-medium uppercase tracking-wider text-zinc-500">
              Alamat IP HP
            </label>
            <div class="mt-1.5 flex flex-wrap gap-2">
              <input
                id="wifi-address"
                bind:value={addressDraft}
                type="text"
                inputmode="url"
                autocomplete="off"
                spellcheck="false"
                placeholder="192.168.1.20"
                class="min-w-0 flex-1 rounded-lg border border-white/10 bg-zinc-950/60 px-3 py-2 font-mono text-xs text-zinc-100 outline-none transition placeholder:text-zinc-600 focus:border-emerald-500/40"
              />
              <button
                type="submit"
                disabled={wifiConnecting}
                class="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-4 py-2 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40"
              >
                {wifiConnecting ? 'Menghubungkan...' : 'Hubungkan'}
              </button>
            </div>
          </form>

          {#if liveWireless.length > 0}
            <p class="rounded-lg border border-emerald-500/20 bg-emerald-500/[0.07] px-3 py-2 font-mono text-[11px] leading-relaxed text-emerald-200">
              Terhubung lewat Wi-Fi: {liveWireless.map((device) => device.serial).join(', ')}
            </p>
          {/if}
        </div>

        <div>
          <div class="flex items-baseline justify-between gap-2">
            <h3 class="text-[10px] font-medium uppercase tracking-wider text-zinc-500">Alamat tersimpan</h3>
            {#if knownWireless.length > 0}
              <span class="text-[10px] text-zinc-600">{knownWireless.length} alamat</span>
            {/if}
          </div>
          {#if knownWireless.length === 0}
            <p class="mt-1.5 rounded-lg border border-dashed border-white/10 bg-black/20 px-3 py-2.5 text-[11px] leading-relaxed text-zinc-500">
              Belum ada alamat tersimpan. Setiap koneksi Wi-Fi yang berhasil dicatat di
              <span class="font-mono">wireless-devices.json</span> dan dicoba ulang otomatis oleh
              aplikasi.
            </p>
          {:else}
            <ul class="mt-1.5 divide-y divide-white/5 overflow-hidden rounded-lg border border-white/5 bg-black/20">
              {#each knownWireless as address (address)}
                {@const live = liveWireless.some((device) => device.serial === address)}
                <li class="flex flex-wrap items-center gap-x-2.5 gap-y-1.5 px-3 py-2">
                  <span class="h-1.5 w-1.5 shrink-0 rounded-full {live ? 'bg-emerald-400' : 'bg-zinc-600'}"></span>
                  <span class="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-300">
                    {splitAddress(address).host}<span class="text-zinc-600">:{splitAddress(address).port ?? '5555'}</span>
                  </span>
                  {#if live}
                    <span class="shrink-0 text-[10px] text-emerald-300/80">terhubung</span>
                  {/if}
                  <button
                    type="button"
                    onclick={() => onConnectAddress(address)}
                    disabled={wifiConnecting}
                    title="Coba sambungkan lagi ke alamat ini"
                    class="shrink-0 rounded-md border border-white/10 bg-white/5 px-2.5 py-1 text-[11px] font-medium text-zinc-200 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    Hubungkan
                  </button>
                  <button
                    type="button"
                    onclick={() => onForgetAddress(address)}
                    aria-label={`Hapus ${address}`}
                    title="Putuskan dan lupakan alamat ini"
                    class="shrink-0 rounded-md px-1.5 text-zinc-600 transition hover:text-rose-300"
                  >
                    ✕
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      </div>
    </section>
  </div>
</div>
