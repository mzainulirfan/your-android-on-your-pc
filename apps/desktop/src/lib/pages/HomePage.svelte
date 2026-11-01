<script lang="ts">
  import type { AndroidDevice, AdbStatus, CaptureMode, Lens, LocateReport, ScrcpyStatus, SessionInfo } from '../types';
  import {
    closeDeviceCamera,
    openDeviceCamera,
    openFindDevice,
    openUrl,
  } from '../api';
  import KeypadControls from '../components/KeypadControls.svelte';
  import { connectionLabel, deviceName, isReady, stateStyle } from '../deviceFormat';

  interface Props {
    devices: AndroidDevice[];
    adb: AdbStatus | null;
    scrcpy: ScrcpyStatus | null;
    session: SessionInfo | null;
    startingSerial: string | null;
    screenshotBusy: boolean;
    screenBusy: 'sleep' | 'wake' | null;
    screenOn: boolean | null;
    screenQueryBusy: boolean;
    /** Live keyguard reading; null when it could not be read yet. */
    keyguardUp: boolean | null;
    onNavigate: (page: 'devices' | 'files' | 'settings') => void;
    onConnect: (serial: string) => void;
    onDisconnect: () => void;
    onScreenshot: (serial: string) => void;
    onScreenPower: (serial: string, sleep: boolean) => Promise<void>;
  }

  let {
    devices,
    adb,
    scrcpy,
    session,
    startingSerial,
    screenshotBusy,
    screenBusy,
    screenOn,
    screenQueryBusy,
    keyguardUp,
    onNavigate,
    onConnect,
    onDisconnect,
    onScreenshot,
    onScreenPower,
  }: Props = $props();

  const readyDevices = $derived(devices.filter(isReady));
  const attentionDevices = $derived(devices.filter((device) => !isReady(device)));
  const heroDevice = $derived(
    session ? (devices.find((device) => device.serial === session.serial) ?? null) : (readyDevices[0] ?? null),
  );
  const actionSerial = $derived(session?.serial ?? readyDevices[0]?.serial ?? null);

  // Tone of the hero: live session first, then a device that needs authorisation, then
  // the plain "nothing connected yet" state.
  const mood = $derived<'live' | 'attention' | 'idle' | 'empty'>(
    session ? 'live' : readyDevices.length > 0 ? 'idle' : attentionDevices.length > 0 ? 'attention' : 'empty',
  );

  const heroTitle = $derived(
    heroDevice
      ? deviceName(heroDevice)
      : session
        ? session.serial
        : mood === 'attention'
          ? 'Perangkat perlu perhatian'
          : 'Belum ada perangkat',
  );

  const heroCaption = $derived.by(() => {
    if (heroDevice) return heroDevice.serial;
    if (session) return `pid ${session.pid}`;
    if (adb && !adb.found) return 'ADB tidak ditemukan';
    if (scrcpy && !scrcpy.found) return 'scrcpy tidak ditemukan';
    return 'Menunggu koneksi';
  });

  const heroHint = $derived.by(() => {
    if (session) {
      return 'Kontrol HP dengan mouse dan keyboard langsung di jendela cermin. Tarik file ke jendela ini untuk mengirimnya ke folder Download HP.';
    }
    if (readyDevices.length > 0) {
      return 'Mulai mirroring menampilkan layar HP di kolom kiri jendela ini, halaman tetap di kanan.';
    }
    if (attentionDevices.length > 0) {
      return 'Setujui dialog USB debugging di layar HP, atau cabut dan sambungkan ulang kabelnya.';
    }
    return 'Aktifkan Opsi Pengembang → USB debugging di HP, sambungkan kabel data, lalu setujui dialog yang muncul.';
  });

  // Live reading first: `session.locked` is the moment the session started, and the phone
  // is usually unlocked from the dock or by hand right afterwards.
  const heroLocked = $derived(keyguardUp ?? (session?.locked ?? false));

  const moodFrame: Record<string, string> = {
    live: 'border-emerald-500/25 bg-zinc-900/50',
    idle: 'border-white/5 bg-zinc-900/40',
    attention: 'border-amber-500/20 bg-zinc-900/40',
    empty: 'border-dashed border-white/10 bg-zinc-900/20',
  };

  const moodGlow: Record<string, string> = {
    live: 'bg-emerald-500/10',
    idle: 'bg-white/[0.03]',
    attention: 'bg-amber-500/[0.07]',
    empty: 'bg-transparent',
  };

  // The app owns the physical screen state (there is only one live session), so the hero
  // can show one toggle whose label says what it will do instead of two guess-the-state
  // buttons.
  async function toggleScreenPower() {
    if (!session) return;
    await onScreenPower(session.serial, screenOn === true);
  }

  let locateBusy = $state<'find' | 'camera' | 'close-camera' | null>(null);
  let cameraActive = $state(false);
  let cameraLens = $state<Lens>('back');
  let cameraMode = $state<CaptureMode>('photo');
  let locateNotice = $state<{ kind: 'ok' | 'partial' | 'error'; text: string } | null>(null);
  let locatePortal = $state<{ url: string; label: string } | null>(null);

  function noticeText(cause: unknown, fallback: string) {
    return typeof cause === 'string' ? cause : fallback;
  }

  let panelError = $state<string | null>(null);
  // The credential card is only mounted when the user asks for it, so a plain Home visit
  // never probes the keyguard or spends a stored PIN on a phone that is not being unlocked.
  let keyCardOpen = $state(false);

  function requireSerial(): string | null {
    if (!actionSerial) {
      panelError = 'Belum ada perangkat siap. Sambungkan HP lewat USB atau Wi-Fi dulu.';
      return null;
    }
    return actionSerial;
  }

  async function runLocate(
    busy: 'find' | 'camera' | 'close-camera',
    action: () => Promise<LocateReport>,
  ): Promise<LocateReport | null> {
    locateBusy = busy;
    locateNotice = null;
    locatePortal = null;
    try {
      const report = await action();
      locateNotice = {
        kind: report.status === 'partial' ? 'partial' : 'ok',
        text: report.notes.join(' '),
      };
      return report;
    } catch (cause) {
      locateNotice = { kind: 'error', text: noticeText(cause, 'Aksi pelacakan gagal dijalankan.') };
      return null;
    } finally {
      locateBusy = null;
    }
  }

  async function openFind() {
    const serial = requireSerial();
    if (!serial) return;
    const report = await runLocate('find', () => openFindDevice(serial));
    if (report?.portalUrl) {
      locatePortal = { url: report.portalUrl, label: report.portalLabel ?? 'portal Find My Device' };
    }
  }

  async function toggleCamera() {
    const serial = requireSerial();
    if (!serial) return;
    if (cameraActive) {
      if (await runLocate('close-camera', () => closeDeviceCamera(serial))) cameraActive = false;
      return;
    }
    const lens = cameraLens;
    const mode = cameraMode;
    if (await runLocate('camera', () => openDeviceCamera(serial, lens, mode))) cameraActive = true;
  }

  const shortcuts = [
    { keys: 'Seret file', text: 'PC → jendela ini = masuk Download HP' },
    { keys: 'Alt+V', text: 'tempel clipboard PC ke HP' },
    { keys: 'Alt+P / Alt+B', text: 'matikan / nyalakan layar HP tanpa putus mirroring' },
    { keys: 'Tutup (X)', text: 'sembunyi ke tray, sesi tetap jalan' },
  ];
</script>

<header class="flex flex-wrap items-end justify-between gap-3 border-b border-white/5 px-4 py-5 @2xl:px-6 @5xl:px-8 @2xl:py-6">
  <div class="min-w-0">
    <h1 class="text-lg font-semibold tracking-tight">Beranda</h1>
    <p class="mt-1 truncate text-xs text-zinc-500">
      {#if session}
        {heroTitle} sedang dicerminkan
      {:else if readyDevices.length > 0}
        {readyDevices.length} perangkat siap dikontrol
      {:else if attentionDevices.length > 0}
        {attentionDevices.length} perangkat perlu perhatian sebelum bisa dipakai
      {:else}
        Sambungkan HP untuk mulai mengontrol dari PC
      {/if}
    </p>
  </div>
  <div class="flex shrink-0 flex-wrap items-center gap-2 text-[11px]">
    <span
      class="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 {adb?.found
        ? 'border-white/10 bg-white/[0.04] text-zinc-400'
        : 'border-amber-500/25 bg-amber-500/10 text-amber-200'}"
      title={adb?.path ?? adb?.error ?? 'ADB belum diperiksa'}
    >
      <span class="h-1.5 w-1.5 rounded-full {adb?.found ? 'bg-emerald-400' : 'bg-amber-400'}"></span>
      ADB
    </span>
    <span
      class="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 {scrcpy?.found
        ? 'border-white/10 bg-white/[0.04] text-zinc-400'
        : 'border-amber-500/25 bg-amber-500/10 text-amber-200'}"
      title={scrcpy?.path ?? scrcpy?.error ?? 'scrcpy belum diperiksa'}
    >
      <span class="h-1.5 w-1.5 rounded-full {scrcpy?.found ? 'bg-emerald-400' : 'bg-amber-400'}"></span>
      scrcpy
    </span>
    {#if session}
      <span class="inline-flex items-center gap-2 rounded-full border border-emerald-500/25 bg-emerald-500/10 px-3 py-1 font-medium text-emerald-300">
        <span class="relative flex h-1.5 w-1.5">
          <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60"></span>
          <span class="relative inline-flex h-1.5 w-1.5 rounded-full bg-emerald-400"></span>
        </span>
        Live
      </span>
    {/if}
  </div>
</header>

<div class="flex-1 overflow-y-auto px-4 py-5 @2xl:px-6 @5xl:px-8 @2xl:py-6">
  <div class="mx-auto max-w-5xl space-y-4">
    <!-- Hero: identity of the device in focus plus the single action that matters now -->
    <section class="relative overflow-hidden rounded-2xl border p-4 {moodFrame[mood]} @2xl:p-6">
      <div
        class="pointer-events-none absolute -top-24 left-1/3 h-48 w-72 rounded-full blur-3xl {moodGlow[mood]}"
      ></div>

      <div class="relative flex flex-wrap items-start gap-4 @2xl:gap-5">
        <div
          class="flex h-14 w-14 shrink-0 items-center justify-center rounded-2xl border @2xl:h-16 @2xl:w-16 {session
            ? 'border-emerald-500/25 bg-emerald-500/10'
            : 'border-white/5 bg-white/[0.03]'}"
        >
          <svg viewBox="0 0 20 20" class="h-8 w-8 {session ? 'text-emerald-400' : 'text-zinc-500'}" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
            <rect x="6" y="2.5" width="8" height="15" rx="2" />
            <path d="M9 15.2h2" />
          </svg>
        </div>

        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <h2 class="truncate text-xl font-semibold tracking-tight @2xl:text-2xl">{heroTitle}</h2>
            {#if heroDevice}
              <span class="rounded-md bg-white/[0.05] px-2 py-0.5 text-[10px] font-medium uppercase tracking-wider text-zinc-400">
                {connectionLabel(heroDevice)}
              </span>
              <span class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-medium ring-1 ring-inset {stateStyle(heroDevice.state).badge}">
                <span class="h-1.5 w-1.5 rounded-full {stateStyle(heroDevice.state).dot}"></span>
                {stateStyle(heroDevice.state).label}
              </span>
            {/if}
          </div>
          <p class="mt-1.5 truncate font-mono text-xs text-zinc-500">
            {heroCaption}{#if session} · pid {session.pid}{/if}
          </p>
          <p class="mt-3 max-w-xl text-xs leading-relaxed text-zinc-400">{heroHint}</p>

          {#if heroLocked && (session || actionSerial)}
            <p class="mt-3 max-w-xl rounded-lg border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-100">
              HP masih terkunci — layar kunci tidak bisa dicerminkan (dilindungi sistem). Ketik PIN
              {#if session}
                di panel kunci pada kolom kiri.
              {:else}
                di kartu Kunci HP di bawah.
              {/if}
            </p>
          {/if}
        </div>
      </div>

      <div class="relative mt-5 flex flex-wrap items-center gap-2 @2xl:mt-6">
        {#if session}
          <button
            type="button"
            onclick={() => void toggleScreenPower()}
            disabled={screenBusy !== null || screenQueryBusy}
            title={screenOn === true
              ? 'Matikan layar fisik HP; mirroring dan kontrol tetap jalan'
              : 'Bangunkan layar fisik HP'}
            class="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-4 py-2 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {#if screenBusy === 'sleep'}
              Mematikan...
            {:else if screenBusy === 'wake'}
              Membangunkan...
            {:else if screenQueryBusy}
              Memeriksa layar...
            {:else if screenOn === null}
              Kontrol layar
            {:else if screenOn}
              Matikan layar HP
            {:else}
              Bangunkan layar HP
            {/if}
          </button>
          <button
            type="button"
            onclick={() => onScreenshot(session.serial)}
            disabled={screenshotBusy}
            class="rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-xs font-medium text-zinc-200 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {screenshotBusy ? 'Mengambil...' : 'Screenshot ke PC'}
          </button>
          <button
            type="button"
            onclick={onDisconnect}
            class="ml-auto rounded-lg border border-rose-500/30 bg-rose-500/10 px-4 py-2 text-xs font-medium text-rose-200 transition hover:bg-rose-500/20"
          >
            Putuskan
          </button>
        {:else if readyDevices.length > 0}
          <button
            type="button"
            onclick={() => onConnect(readyDevices[0].serial)}
            disabled={startingSerial !== null || !scrcpy?.found}
            title={!scrcpy?.found ? 'scrcpy tidak ditemukan' : ''}
            class="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-5 py-2 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {startingSerial === readyDevices[0].serial ? 'Membuka...' : 'Mulai mirroring'}
          </button>
          <button
            type="button"
            onclick={() => (keyCardOpen = !keyCardOpen)}
            aria-expanded={keyCardOpen}
            title="Buka kunci HP tanpa memulai mirroring"
            class="rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-xs font-medium text-zinc-200 transition hover:bg-white/10"
          >
            {keyCardOpen ? 'Tutup kolom kunci' : 'Kunci HP'}
          </button>
          {#if readyDevices.length > 1}
            <button
              type="button"
              onclick={() => onNavigate('devices')}
              class="text-[11px] text-zinc-500 underline-offset-2 transition hover:text-zinc-300 hover:underline"
            >
              +{readyDevices.length - 1} perangkat lain terdeteksi
            </button>
          {/if}
        {:else}
          <button
            type="button"
            onclick={() => onNavigate('devices')}
            class="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-5 py-2 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/25"
          >
            Sambungkan perangkat
          </button>
          {#if adb && !adb.found}
            <span class="text-[11px] text-amber-200/80">ADB belum ditemukan.</span>
          {:else if scrcpy && !scrcpy.found}
            <span class="text-[11px] text-amber-200/80">scrcpy belum ditemukan.</span>
          {/if}
        {/if}
      </div>
    </section>

    <!-- Shortcuts: one line instead of a collapsible list that nobody opens -->
    <section class="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-xl border border-white/5 bg-zinc-900/30 px-5 py-3">
      {#each shortcuts as shortcut (shortcut.keys)}
        <p class="flex items-center gap-2 text-[11px] text-zinc-500">
          <span class="rounded border border-white/10 bg-white/[0.04] px-1.5 py-0.5 font-mono text-[10px] text-zinc-300">
            {shortcut.keys}
          </span>
          {shortcut.text}
        </p>
      {/each}
    </section>

    {#if !session && actionSerial && keyCardOpen}
      <!-- Kunci HP tanpa cermin: dulu jendela panel mengambang, sekarang kartu di halaman
           yang sama. Hanya muncul setelah diminta, supaya kolom kunci tidak mem-probe HP
           (dan mencoba PIN tersimpan) setiap kali Home dibuka. -->
      <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-2">
        <KeypadControls serial={actionSerial} />
      </section>
    {/if}

    {#if panelError}
      <p class="rounded-lg border border-rose-500/25 bg-rose-500/10 px-4 py-2.5 text-[11px] leading-relaxed text-rose-200">
        {panelError}
      </p>
    {/if}

    {#if attentionDevices.length > 0}
      <button
        type="button"
        onclick={() => onNavigate('devices')}
        class="flex w-full items-center gap-3 rounded-xl border border-amber-500/20 bg-amber-500/[0.06] px-5 py-3 text-left transition hover:bg-amber-500/10"
      >
        <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-400"></span>
        <span class="min-w-0 flex-1 truncate text-xs text-amber-100/90">
          {attentionDevices.length} perangkat belum siap ({attentionDevices.map((device) => `${deviceName(device)} · ${stateStyle(device.state).label}`).join(', ')}).
        </span>
        <span class="shrink-0 text-[11px] text-amber-200/70">Buka halaman Perangkat →</span>
      </button>
    {/if}

    <!-- Locate -->
    <section class="rounded-2xl border border-white/5 bg-zinc-900/40 p-4 @2xl:p-5">
      <div class="flex flex-wrap items-center gap-3">
        <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-[18px] w-[18px] text-sky-300/80" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
            <circle cx="10" cy="10" r="6" />
            <circle cx="10" cy="10" r="1.4" fill="currentColor" stroke="none" />
            <path d="M10 1.8v2.4M10 15.8v2.4M1.8 10h2.4M15.8 10h2.4" />
          </svg>
        </span>
        <div class="min-w-0 flex-1">
          <h2 class="text-sm font-semibold">Cari HP</h2>
          <p class="mt-0.5 text-[11px] leading-relaxed text-zinc-500">
            Untuk HP yang terselip: layar dibangunkan dan pintasan portal Find My Device muncul.
          </p>
        </div>
      </div>
      <div class="mt-4 grid grid-cols-1 gap-2 @xl:grid-cols-2">
        <select
          bind:value={cameraLens}
          disabled={cameraActive || locateBusy !== null}
          aria-label="Lensa kamera"
          class="rounded-lg border border-white/10 bg-zinc-950/60 px-2 py-1.5 text-[11px] text-zinc-200 disabled:opacity-40"
        >
          <option value="back">Kamera belakang</option>
          <option value="front">Kamera depan</option>
        </select>
        <select
          bind:value={cameraMode}
          disabled={cameraActive || locateBusy !== null}
          aria-label="Mode kamera"
          class="rounded-lg border border-white/10 bg-zinc-950/60 px-2 py-1.5 text-[11px] text-zinc-200 disabled:opacity-40"
        >
          <option value="photo">Foto</option>
          <option value="video">Video</option>
        </select>
      </div>
      <div class="mt-2 flex flex-col gap-2 @xl:flex-row">
        <button
          type="button"
          onclick={() => void openFind()}
          disabled={locateBusy !== null || actionSerial === null}
          class="flex-1 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-left text-[11px] font-medium text-zinc-200 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {locateBusy === 'find' ? 'Membuka lacak...' : 'Lacak via Find My Device'}
        </button>
        <button
          type="button"
          onclick={() => void toggleCamera()}
          disabled={locateBusy !== null || actionSerial === null}
          class="flex-1 rounded-lg border px-3 py-2 text-left text-[11px] font-medium transition disabled:cursor-not-allowed disabled:opacity-40 {cameraActive
            ? 'border-rose-500/40 bg-rose-500/15 text-rose-100 hover:bg-rose-500/25'
            : 'border-white/10 bg-white/5 text-zinc-200 hover:bg-white/10'}"
        >
          {#if locateBusy === 'camera'}
            Membuka kamera...
          {:else if locateBusy === 'close-camera'}
            Menutup kamera...
          {:else if cameraActive}
            Tutup kamera
          {:else}
            Buka kamera HP
          {/if}
        </button>
      </div>

      {#if locateNotice}
        <p
          class="mt-3 rounded-lg border px-3 py-2 text-[10px] leading-relaxed {locateNotice.kind ===
            'error'
            ? 'border-rose-500/25 bg-rose-500/10 text-rose-200'
            : locateNotice.kind === 'partial'
              ? 'border-amber-500/25 bg-amber-500/10 text-amber-100'
              : 'border-emerald-500/25 bg-emerald-500/10 text-emerald-100'}"
        >
          {locateNotice.text}
        </p>
      {/if}

      {#if locatePortal}
        <button
          type="button"
          onclick={() => void openUrl(locatePortal?.url ?? '')}
          class="mt-2 w-full rounded-lg border border-sky-500/30 bg-sky-500/10 px-3 py-2 text-left text-[11px] font-medium text-sky-100 transition hover:bg-sky-500/20"
        >
          Buka {locatePortal.label} — {locatePortal.url.replace('https://', '')} → “Putar Suara”
        </button>
      {/if}
    </section>

    <!-- Destinations -->
    <div class="grid gap-3 @xl:grid-cols-3">
      <button
        type="button"
        onclick={() => onNavigate('files')}
        class="rounded-xl border border-white/5 bg-zinc-900/40 p-4 text-left transition hover:border-emerald-500/20 hover:bg-zinc-900/60"
      >
        <div class="flex h-9 w-9 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-4 w-4 text-sky-300/80" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round">
            <path d="M3.5 5.5a1 1 0 0 1 1-1h3l1.5 2h6a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z" />
          </svg>
        </div>
        <p class="mt-3 text-xs font-semibold text-zinc-100">Berkas</p>
        <p class="mt-1 text-[11px] leading-relaxed text-zinc-500">
          Jelajahi penyimpanan HP, unduh, atau unggah.
        </p>
      </button>
      <button
        type="button"
        onclick={() => onNavigate('devices')}
        class="rounded-xl border border-white/5 bg-zinc-900/40 p-4 text-left transition hover:border-emerald-500/20 hover:bg-zinc-900/60"
      >
        <div class="flex h-9 w-9 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-4 w-4 text-emerald-300/80" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
            <rect x="6" y="2.5" width="8" height="15" rx="2" />
            <path d="M9 15.2h2" />
          </svg>
        </div>
        <p class="mt-3 text-xs font-semibold text-zinc-100">Perangkat</p>
        <p class="mt-1 text-[11px] leading-relaxed text-zinc-500">
          {devices.length > 0
            ? `${readyDevices.length} siap dari ${devices.length} perangkat`
            : 'Perangkat, Wi-Fi, dan koneksi ulang'}
        </p>
      </button>
      <button
        type="button"
        onclick={() => onNavigate('settings')}
        class="rounded-xl border border-white/5 bg-zinc-900/40 p-4 text-left transition hover:border-emerald-500/20 hover:bg-zinc-900/60"
      >
        <div class="flex h-9 w-9 items-center justify-center rounded-xl border border-white/5 bg-zinc-950/60">
          <svg viewBox="0 0 20 20" class="h-4 w-4 text-zinc-400" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
            <path d="M4 6h8M4 12h4M4 18h10" />
            <circle cx="15" cy="6" r="1.6" />
            <circle cx="10" cy="12" r="1.6" />
            <circle cx="16" cy="18" r="1.6" />
          </svg>
        </div>
        <p class="mt-3 text-xs font-semibold text-zinc-100">Setelan</p>
        <p class="mt-1 text-[11px] leading-relaxed text-zinc-500">
          Kualitas mirroring, folder unduhan, diagnostik.
        </p>
      </button>
    </div>
  </div>
</div>
