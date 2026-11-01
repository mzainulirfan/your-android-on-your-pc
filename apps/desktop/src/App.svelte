<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import {
    animateWindowWidth,
    captureScreenshot,
    connectWireless,
    enableWireless,
    fitWindowHeight,
    forgetWireless,
    getAdbStatus,
    getDevicesSnapshot,
    getKnownWireless,
    getMirrorAspect,
    getScrcpyStatus,
    getSessionStatus,
    isTauri,
    onDevicesChanged,
    onDeviceUnlocked,
    onMirrorAspect,
    onSessionError,
    onSessionExited,
    onSessionStarted,
    onTransferProgress,
    pushFiles,
    sendClipboardImage as requestClipboardImage,
    setScreenPower,
    startSession,
    stopSession,
    setKeyboardOwner,
    setMirrorRect,
    setWindowWidth,
    wakeState,
  } from './lib/api';
  import FilesPage from './lib/pages/FilesPage.svelte';
  import DevicesPage from './lib/pages/DevicesPage.svelte';
  import HomePage from './lib/pages/HomePage.svelte';
  import KeypadControls from './lib/components/KeypadControls.svelte';
  import SettingsPage from './lib/pages/SettingsPage.svelte';
  import type {
    AdbStatus,
    AndroidDevice,
    DevicesSnapshot,
    KeypadState,
    ScrcpyStatus,
    SessionInfo,
    TransferProgress,
  } from './lib/types';

  type Page = 'home' | 'devices' | 'files' | 'settings';

  let devices = $state<AndroidDevice[]>([]);
  let adb = $state<AdbStatus | null>(null);
  let scrcpy = $state<ScrcpyStatus | null>(null);
  let session = $state<SessionInfo | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let refreshing = $state(false);
  let startingSerial = $state<string | null>(null);
  let wifiBusy = $state<string | null>(null);
  let wifiConnecting = $state(false);
  let knownWireless = $state<string[]>([]);
  let imageBusy = $state(false);
  let screenshotBusy = $state(false);
  let screenBusy = $state<'sleep' | 'wake' | null>(null);
  let dragging = $state(false);
  let transfer = $state<TransferProgress | null>(null);
  let sessionNotice = $state<{ kind: 'info' | 'error'; text: string } | null>(null);
  let page = $state<Page>('home');
  let filesDir = $state('/sdcard/Download');
  let filesSerial = $state<string | null>(null);
  let filesRefreshKey = $state(0);

  const connectedCount = $derived(devices.filter((device) => device.state === 'device').length);
  const readyDevices = $derived(devices.filter((device) => device.state === 'device'));
  const transferTarget = $derived(
    session?.serial ?? (readyDevices.length === 1 ? readyDevices[0].serial : null),
  );

  // Only one device can be mirrored at a time, so the physical screen state and the
  // keyguard state are read once here and shared with every surface that shows them.
  let screenOn = $state<boolean | null>(null);
  let screenQueryBusy = $state(false);
  let keyguardUp = $state<boolean | null>(null);
  // The last time both verdicts were read from the phone. Page switches reuse them while
  // they are fresh: a `dumpsys` on every navigation landed in the middle of the mirrored
  // stream, and the banner already follows `device-unlocked`.
  let lastStateReadAt = 0;
  const STATE_FRESH_MS = 30000;

  // The lock screen is dismissed on the phone itself (or through the dock), so the
  // "HP masih terkunci" banner has to follow the device instead of the snapshot taken
  // when the session started. A failed read keeps the last known state: a transient ADB
  // error must not flip the banner.
  async function refreshDeviceState(interactive = false) {
    if (!session) {
      screenOn = null;
      keyguardUp = null;
      return;
    }
    if (interactive) screenQueryBusy = true;
    try {
      // One read for both verdicts: `dumpsys` is answered by system_server, the same process
      // that renders the animations in the mirror, so two concurrent dumps took CPU away
      // from the frames the user was watching.
      const state = await wakeState(session.serial);
      screenOn = state.screenOn;
      keyguardUp = state.keyguardUp;
      lastStateReadAt = Date.now();
    } catch {
      // Keep the previous reading.
    } finally {
      if (interactive) screenQueryBusy = false;
    }
  }

  // Below 1024 px the sidebar becomes a 68 px icon rail: labels and badges would not fit,
  // so they are hidden and the `title` on each button carries the name instead.
  function navClass(active: boolean) {
    return active
      ? 'flex items-center gap-3 rounded-lg bg-white/[0.06] px-3 py-2 text-left text-[13px] font-medium text-zinc-100 max-lg:justify-center max-lg:px-0'
      : 'flex items-center gap-3 rounded-lg px-3 py-2 text-left text-[13px] text-zinc-400 transition hover:bg-white/[0.04] hover:text-zinc-200 max-lg:justify-center max-lg:px-0';
  }

  // While the mirror holds the left column there is no room for a labelled rail: navigation
  // shrinks to the same four icons in the header of the page column.
  function iconNavClass(active: boolean) {
    return active
      ? 'flex h-8 w-8 items-center justify-center rounded-lg bg-white/[0.06] text-zinc-100'
      : 'flex h-8 w-8 items-center justify-center rounded-lg text-zinc-400 transition hover:bg-white/[0.04] hover:text-zinc-200';
  }

  function navigate(target: Page) {
    page = target;
    sessionNotice = null;
    // Returning to a page should show the phone as it is now, not as it was when the
    // session started; the lock screen is often dismissed on the device itself. The read
    // is skipped while the last one is still fresh, because system_server answers it in
    // the middle of the mirrored frames.
    if (session && Date.now() - lastStateReadAt > STATE_FRESH_MS) {
      void refreshDeviceState(true);
    }
  }

  const navPages: { page: Page; label: string }[] = [
    { page: 'home', label: 'Home' },
    { page: 'devices', label: 'Devices' },
    { page: 'files', label: 'Files' },
    { page: 'settings', label: 'Settings' },
  ];

  function navIconClass(target: Page) {
    return `h-4 w-4 shrink-0 ${page === target ? 'text-emerald-400' : ''}`;
  }

  // One window, two columns: the phone on the left, the page it belongs to on the right.
  const mirroring = $derived(!!session && isTauri);

  // The page owns the video's box. Rust never resizes the window; it only moves the embedded
  // frame to whatever this element measures, in CSS pixels — the webview fills the client area,
  // so the two coordinate systems are the same.
  let stage = $state<HTMLDivElement | null>(null);
  let stageHost = $state<HTMLDivElement | null>(null);
  let mirrorAspect = $state<number | null>(null);
  let mirrorHeight = $state(0);
  let videoHidden = $state(false);
  let keyboardOnPhone = $state(false);
  // The form below the toolbar owns the unlock flow; it publishes the few values the toolbar
  // needs instead of the toolbar duplicating its state.
  let keypad = $state<{ toggleForm: () => void; toggleScreen: () => Promise<void> }>();
  let keypadState = $state<KeypadState>({
    expanded: false,
    screenOn: true,
    screenBusy: false,
    busy: false,
    locked: false,
    message: null,
    messageKind: null,
  });

  function onKeypadState(state: KeypadState) {
    keypadState = state;
  }

  function onKeypadKeyboardOwner(owner: 'phone' | 'pc') {
    keyboardOnPhone = owner === 'phone';
  }

  // Seed the toolbar before the form's first publish: a locked session must not flash an
  // "unlocked" toolbar for one frame while the card is mounting.
  function primeKeypad(info: SessionInfo) {
    keypadState = {
      expanded: info.locked,
      screenOn: info.screenOn,
      screenBusy: false,
      busy: false,
      locked: info.locked,
      message: info.locked ? 'HP terkunci' : null,
      messageKind: info.locked ? 'locked' : null,
    };
  }

  const toolbarBusy = $derived(keypadState.busy || keypadState.screenBusy);
  const toolbarAttention = $derived(!keypadState.screenOn || keypadState.locked);
  // A phone that locks mid-session is reported by the 30 s watcher (`keyguardUp`), not by
  // the panel: the panel only probes the keyguard while its form is open. The cover has to
  // follow both, or a power-button lock stays a black hole until the form is opened.
  const mirrorLocked = $derived(keypadState.locked || keyguardUp === true);
  const toolbarHasMessage = $derived(!toolbarBusy && keypadState.message !== null);
  const toolbarText = $derived(
    toolbarBusy
      ? keypadState.busy
        ? 'Mengirim kunci…'
        : 'Mengatur layar…'
      : (keypadState.message ?? session?.serial ?? ''),
  );

  let panelCollapsed = $state(false);
  let panelAutoCollapsed = $state(false);
  // A manual hide/show lasts until the window is resized; after that the width rules decide
  // again. This keeps the button from being overruled the instant it is clicked.
  let panelManual = $state(false);
  let viewportWidth = $state(typeof window === 'undefined' ? 1180 : window.innerWidth);
  // The width to return to when the panel is shown again. Plain (not `$state`): this value
  // must never make the layout effects chase the window's own resize.
  let expandedWidth = typeof window === 'undefined' ? 1180 : window.innerWidth;
  // Whether Rust currently pins the window's width. Plain for the same reason as
  // `expandedWidth`: it is a mirror of the OS state, not a rendering input.
  let windowWidthLocked = false;
  // The window animation is owned by Rust; these plain values keep the resize listeners from
  // mistaking its intermediate frames for a user resize.
  let panelAnimating = false;
  let panelAnimationGeneration = 0;
  let lastPanelCollapsed: boolean | null = null;
  // The new aspect must not be applied before the height fit finishes: otherwise a width
  // animation could capture the old (shorter) height and write it back every frame.
  let heightFit: Promise<void> = Promise.resolve();
  const adminHidden = $derived(mirroring && panelCollapsed);

  // The admin panel is only hidden automatically when it would be too narrow to use. The
  // mirror's own width is known from the phone's proportions, so a wide window keeps the
  // panel and a small one gives the picture the space.
  const ADMIN_MIN_WIDTH = 620;

  const mirrorWidthPx = $derived(
    mirrorAspect && mirrorHeight > 0
      ? Math.round(mirrorHeight * mirrorAspect) + 25
      : null,
  );

  // The column is exactly as wide as the phone at the height it has, so a portrait phone fills
  // it from edge to edge instead of sitting between empty bands. Keeping the px width even
  // while the panel folds away lets the frame animate around a stable video: the panel column
  // shrinks to zero instead of the phone being letterboxed on every intermediate frame.
  const mirrorColumnWidth = $derived(
    mirrorWidthPx ? `${mirrorWidthPx}px` : panelCollapsed ? '100%' : '50%',
  );

  function togglePanel() {
    panelCollapsed = !panelCollapsed;
    panelAutoCollapsed = false;
    panelManual = true;
  }

  function resizeWindowTo(width: number, minWidth: number, lockWidth = false) {
    const target = Math.round(width);
    if (!isTauri) return;
    // A locked frame still has to be re-sent when the height changes, because Rust owns the
    // min/max width pair; an unlocked one only moves when the size is actually different.
    // Releasing the lock must always reach Rust even if the width already matches.
    if (!lockWidth && !windowWidthLocked && Math.abs(window.innerWidth - target) <= 2) return;
    windowWidthLocked = lockWidth;
    void setWindowWidth(target, Math.round(minWidth), lockWidth).catch(() => {});
  }

  // Opening and closing the panel gets the eased path. Width changes caused by the stage height
  // (keypad card folding, phone rotation) stay immediate: the video has already changed shape,
  // so the frame must follow on the same frame or it would letterbox while catching up.
  async function animatePanelWindow(width: number, minWidth: number, lockWidth: boolean) {
    const generation = ++panelAnimationGeneration;
    panelAnimating = true;
    windowWidthLocked = lockWidth;
    try {
      await animateWindowWidth(Math.round(width), Math.round(minWidth), lockWidth);
    } catch {
      // Keep the layout usable even if the OS refuses one frame.
    } finally {
      if (generation !== panelAnimationGeneration) return;
      panelAnimating = false;
      if (!panelCollapsed) expandedWidth = window.innerWidth;
    }
  }

  // Track the window width so a shrinking frame can fold the panel away on its own. A resize
  // also clears the manual override, giving the automatic rule another chance.
  $effect(() => {
    const onResize = () => {
      viewportWidth = window.innerWidth;
      // The eased open/close emits one resize per frame; those are ours, not the user's, so
      // they must not wipe the manual choice or move the remembered width mid-flight.
      if (panelAnimating) return;
      panelManual = false;
      // Remember the last width the user chose while the panel was visible, so folding it
      // away and back does not leave the window at the portrait width.
      if (!panelCollapsed) expandedWidth = window.innerWidth;
    };
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  $effect(() => {
    if (!mirroring || panelManual || panelAnimating) return;
    const mirrorWidth = mirrorWidthPx ?? Math.round(viewportWidth / 2);
    const narrow = viewportWidth - mirrorWidth < ADMIN_MIN_WIDTH;
    if (narrow && !panelCollapsed) {
      panelCollapsed = true;
      panelAutoCollapsed = true;
    } else if (!narrow && panelCollapsed && panelAutoCollapsed) {
      panelCollapsed = false;
      panelAutoCollapsed = false;
    }
  });

  // Fold the frame itself when the panel goes away: the window shrinks to the mirror's width,
  // so the phone fills it edge to edge instead of leaving bands on either side. The width is
  // pinned in Rust while folded, so a horizontal drag cannot break the ratio; a vertical drag
  // changes the stage height and this effect feeds the new width back. Showing the panel
  // restores the last user-chosen width (or the minimum needed for the panel).
  $effect(() => {
    const collapsed = panelCollapsed;
    const mirrorWidth = mirrorWidthPx;
    if (!mirroring || !mirrorWidth) return;
    const changed = lastPanelCollapsed !== collapsed;
    lastPanelCollapsed = collapsed;
    const width = collapsed
      ? mirrorWidth
      : Math.max(expandedWidth, mirrorWidth + ADMIN_MIN_WIDTH);
    const minWidth = collapsed ? mirrorWidth : 720;
    if (changed) {
      void animatePanelWindow(width, minWidth, collapsed);
    } else {
      resizeWindowTo(width, minWidth, collapsed);
    }
  });

  // The stage's height is set by the window, the header and the keypad card; its width is
  // whatever the column gives it. Measure the available height (p-3 = 12 px each side) so
  // the column width can be derived from the phone's proportions.
  $effect(() => {
    const node = stageHost;
    if (!node) return;
    let last = -1;
    const measure = () => {
      const next = Math.max(0, node.clientHeight - 24);
      if (Math.abs(next - last) <= 0.5) return;
      last = next;
      mirrorHeight = next;
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  });

  $effect(() => {
    const node = stage;
    const hidden = videoHidden;
    if (!node) return;
    let reported: number[] | null = null;
    let frame = 0;
    const report = () => {
      const box = node.getBoundingClientRect();
      const next = hidden ? [box.left, box.top, 0, 0] : [box.left, box.top, box.width, box.height];
      if (reported && reported.every((value, index) => Math.abs(value - next[index]) <= 1)) return;
      reported = next;
      void setMirrorRect(next[0], next[1], next[2], next[3]);
    };
    report();
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(report);
    });
    observer.observe(node);
    window.addEventListener('resize', report);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      window.removeEventListener('resize', report);
      // The column is gone (session ended or the user left it): leave no box for Rust to aim at,
      // so a stale rectangle can never place the next session's video.
      if (reported) void setMirrorRect(reported[0], reported[1], 0, 0);
    };
  });

  // Last pointer press on the page. Activating the window (Alt+Tab, taskbar, tray) can
  // restore DOM focus to an input the user did not click — often the field used before
  // mirroring started — and that focus event must not hand the keyboard back.
  let lastPagePointerAt = 0;
  let lastPagePointerTarget: EventTarget | null = null;

  $effect(() => {
    const mark = (event: PointerEvent) => {
      lastPagePointerAt = Date.now();
      lastPagePointerTarget = event.target;
    };
    document.addEventListener('pointerdown', mark, true);
    return () => document.removeEventListener('pointerdown', mark, true);
  });

  // Typing into a field of the page has to win over the mirror: while the phone holds the
  // keyboard every keystroke is relayed to it, so the caret would never move here. Only a
  // pointer press on (or around) that field counts as intent; a focus event replayed by
  // window activation does not.
  function onPageFocusIn(event: FocusEvent) {
    const target = event.target as HTMLElement | null;
    if (!mirroring || !target) return;
    const editable =
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target.isContentEditable;
    if (editable && keyboardOnPhone) {
      const pressed = lastPagePointerTarget;
      const fromPointer =
        Date.now() - lastPagePointerAt <= 500 &&
        pressed instanceof Node &&
        (pressed === target || pressed.contains(target) || target.contains(pressed));
      if (!fromPointer) return;
      keyboardOnPhone = false;
      void setKeyboardOwner('pc');
    }
  }

  function toggleKeyboardOwner() {
    keyboardOnPhone = !keyboardOnPhone;
    void setKeyboardOwner(keyboardOnPhone ? 'phone' : 'pc');
  }

  function applySnapshot(snapshot: DevicesSnapshot) {
    devices = snapshot.devices;
    error = snapshot.error ?? null;
    loading = false;
  }

  async function refresh(fresh = true) {
    refreshing = true;
    try {
      const [adbStatus, snapshot] = await Promise.all([
        getAdbStatus(),
        getDevicesSnapshot(fresh),
      ]);
      adb = adbStatus;
      applySnapshot(snapshot);
    } catch (cause) {
      error = typeof cause === 'string' ? cause : 'Gagal membaca daftar perangkat.';
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  function errorText(cause: unknown) {
    if (typeof cause === 'string') return cause;
    if (cause instanceof Error) return cause.message;
    return 'Terjadi kesalahan yang tidak diketahui.';
  }

  async function connect(serial: string) {
    startingSerial = serial;
    sessionNotice = null;
    try {
      session = await startSession(serial);
      if (session) primeKeypad(session);
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    } finally {
      startingSerial = null;
    }
  }

  async function activateWireless(serial: string) {
    wifiBusy = serial;
    sessionNotice = null;
    try {
      const info = await enableWireless(serial);
      sessionNotice = {
        kind: 'info',
        text: `Wi-Fi aktif di ${info.wifiSerial}. Cabut kabel USB, lalu tekan Hubungkan pada kartu Wi-Fi.`,
      };
      await refresh();
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    } finally {
      wifiBusy = null;
    }
  }

  async function loadKnownWireless() {
    try {
      knownWireless = await getKnownWireless();
    } catch {
      knownWireless = [];
    }
  }

  async function connectByAddress(address: string) {
    const target = address.trim();
    if (!target) {
      sessionNotice = { kind: 'error', text: 'Tulis dulu IP HP-nya, misalnya 192.168.1.20.' };
      return;
    }

    wifiConnecting = true;
    sessionNotice = null;
    try {
      const connected = await connectWireless(target);
      sessionNotice = { kind: 'info', text: `Terhubung ke ${connected}.` };
      await Promise.all([refresh(), loadKnownWireless()]);
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    } finally {
      wifiConnecting = false;
    }
  }

  async function removeKnownWireless(address: string) {
    try {
      await forgetWireless(address);
      await loadKnownWireless();
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    }
  }

  async function sendImageFromClipboard(serial: string) {
    imageBusy = true;
    sessionNotice = null;
    try {
      const result = await requestClipboardImage(serial);
      sessionNotice = result.shareOpened
        ? {
            kind: 'info',
            text: `Gambar dikirim (${result.fileName}). Pilih WhatsApp di lembar Bagikan Android, lalu chat tujuan.`,
          }
        : {
            kind: 'info',
            text: `Gambar tersimpan di Download (${result.fileName}). Buka WhatsApp > lampiran > Galeri untuk mengirimnya.`,
          };
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    } finally {
      imageBusy = false;
    }
  }

  async function handleDrop(paths: string[]) {
    dragging = false;

    const uploadingToFiles = page === 'files' && filesSerial !== null;
    const serial = uploadingToFiles ? filesSerial : transferTarget;
    if (!serial) {
      sessionNotice = {
        kind: 'error',
        text: 'Tidak ada perangkat tujuan. Sambungkan satu perangkat atau mulai sesi mirroring.',
      };
      return;
    }

    transfer = null;
    try {
      const result = await pushFiles(serial, paths, uploadingToFiles ? filesDir : undefined);
      if (result.failed.length > 0) {
        const names = result.failed.map((item) => item.name).join(', ');
        sessionNotice = {
          kind: 'error',
          text: `${result.pushed} file terkirim; gagal: ${names}.`,
        };
      } else if (uploadingToFiles) {
        filesRefreshKey += 1;
        sessionNotice = {
          kind: 'info',
          text: `${result.pushed} file diunggah ke ${filesDir}.`,
        };
      } else {
        sessionNotice = {
          kind: 'info',
          text: `${result.pushed} file dikirim ke ${serial}. Buka Files > Download di HP untuk melihatnya.`,
        };
      }
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    } finally {
      transfer = null;
    }
  }

  async function takeScreenshot(serial: string) {
    screenshotBusy = true;
    sessionNotice = null;
    try {
      const result = await captureScreenshot(serial);
      sessionNotice = {
        kind: 'info',
        text: `Screenshot disimpan: ${result.path}`,
      };
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    } finally {
      screenshotBusy = false;
    }
  }

  async function controlScreen(serial: string, sleep: boolean) {
    screenBusy = sleep ? 'sleep' : 'wake';
    sessionNotice = null;
    try {
      // The wake path reads both states on the phone inside the same call, so there is no
      // second `dumpsys` here just to label the button.
      const state = await setScreenPower(serial, sleep);
      screenOn = state.screenOn;
      keyguardUp = state.keyguardUp;
      sessionNotice = sleep
        ? { kind: 'info', text: 'Layar HP dimatikan — mirroring dan kontrol tetap berjalan.' }
        : { kind: 'info', text: 'Layar HP dibangunkan.' };
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    } finally {
      screenBusy = null;
    }
  }

  async function disconnect() {
    try {
      await stopSession();
    } catch (cause) {
      sessionNotice = { kind: 'error', text: errorText(cause) };
    }
  }

  onMount(() => {
    void refresh(false);
    void loadKnownWireless();

    void getScrcpyStatus()
      .then((status) => (scrcpy = status))
      .catch(() => {});
    void getSessionStatus()
      .then((status) => {
        session = status;
        // A reloaded page can adopt the states the running session already read.
        if (status) {
          primeKeypad(status);
          screenOn = status.screenOn;
          keyguardUp = status.locked;
          lastStateReadAt = Date.now();
          // A reloaded page adopts a session that was already fitted; re-running this is a
          // no-op but keeps the window full-height if it was resized before the reload.
          heightFit = fitWindowHeight().catch(() => {});
          void heightFit
            .then(() => getMirrorAspect())
            .then((aspect) => {
              if (aspect) mirrorAspect = aspect;
            })
            .catch(() => {});
        }
      })
      .catch(() => {});

    const unsubscribe = onDevicesChanged(applySnapshot);

    const unsubscribeStarted = onSessionStarted((info) => {
      session = info;
      primeKeypad(info);
      startingSerial = null;
      // A new phone brings new proportions; they arrive once the scrcpy window settles.
      mirrorAspect = null;
      panelCollapsed = false;
      panelAutoCollapsed = false;
      panelManual = false;
      panelAnimationGeneration += 1;
      panelAnimating = false;
      lastPanelCollapsed = null;
      // The session prep already read both verdicts; asking the phone again here would put
      // a `dumpsys` in the middle of scrcpy's startup and leave the toggle on
      // "Memeriksa layar..." while the video is coming up.
      screenOn = info.screenOn;
      keyguardUp = info.locked;
      // Rust hands the keyboard to the mirror at attach when the phone is already open, and
      // keeps the caret in this window when it is locked; the chip has to say the same thing.
      keyboardOnPhone = !info.locked;
      lastStateReadAt = Date.now();
      // Use the PC screen properly as soon as the session is up: the window spans the
      // monitor's work area top to bottom, then the aspect effect picks the width.
      heightFit = fitWindowHeight().catch(() => {});
    });

    const unsubscribeExited = onSessionExited((info) => {
      session = null;
      startingSerial = null;
      screenOn = null;
      keyguardUp = null;
      mirrorAspect = null;
      mirrorHeight = 0;
      panelCollapsed = false;
      panelAutoCollapsed = false;
      panelManual = false;
      panelAnimationGeneration += 1;
      panelAnimating = false;
      lastPanelCollapsed = null;
      heightFit = Promise.resolve();
      lastStateReadAt = 0;
      // A collapsed session lowered the window floor to the phone's width. Put the admin
      // layout back on its normal minimum so the page is usable again after disconnecting.
      if (isTauri) {
        windowWidthLocked = false;
        void setWindowWidth(Math.max(expandedWidth, 720), 720, false).catch(() => {});
      }
      if (info.reason === 'stopped') {
        sessionNotice = { kind: 'info', text: 'Sesi dihentikan.' };
      } else if (info.code === 0) {
        sessionNotice = null;
      }
    });

    const unsubscribeError = onSessionError((info) => {
      sessionNotice = { kind: 'error', text: info.message };
    });

    // The panel noticed the phone open — by its own unlock, by a PIN typed into the mirror,
    // or on the device itself. Without this the dashboard would keep the "masih terkunci"
    // banner until the 30 s poll.
    const unsubscribeUnlocked = onDeviceUnlocked((serial) => {
      if (serial === session?.serial) {
        keyguardUp = false;
        screenOn = true;
      }
    });

    // The mirror learned the phone's proportions (attached, or rotated): resize the column
    // so the picture fills it instead of leaving bands on either side.
    const unsubscribeAspect = onMirrorAspect((aspect) => {
      void heightFit.then(() => {
        if (aspect > 0) mirrorAspect = aspect;
      });
    });

    const unsubscribeTransfer = onTransferProgress((progress) => {
      transfer = progress;
    });

    const dragListener = isTauri
      ? getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === 'enter' || event.payload.type === 'over') {
            dragging = true;
          } else if (event.payload.type === 'drop') {
            dragging = false;
            void handleDrop(event.payload.paths);
          } else {
            dragging = false;
          }
        })
      : null;

    // One timer, one job: the live lock/screen state while a session runs and the window
    // can be seen. The read costs system_server 180–670 ms on the test device, and a dump
    // during the mirrored animation shows up as a stall, so 30 s is deliberate. ADB and
    // scrcpy cannot appear or move while the app runs; Refresh re-reads them on demand.
    const statusTimer = window.setInterval(() => {
      if (session && document.visibilityState === 'visible') {
        void refreshDeviceState();
      }
    }, 30000);

    return () => {
      void unsubscribe.then((stop) => stop());
      void unsubscribeStarted.then((stop) => stop());
      void unsubscribeExited.then((stop) => stop());
      void unsubscribeError.then((stop) => stop());
      void unsubscribeUnlocked.then((stop) => stop());
      void unsubscribeAspect.then((stop) => stop());
      void unsubscribeTransfer.then((stop) => stop());
      if (dragListener) void dragListener.then((stop) => stop());
      window.clearInterval(statusTimer);
    };
  });
</script>

{#snippet navIconSvg(target: Page)}
  {#if target === 'home'}
    <svg viewBox="0 0 20 20" class={navIconClass(target)} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
      <path d="M4 9.5 10 4l6 5.5V16a1 1 0 0 1-1 1h-3.5v-4h-3v4H5a1 1 0 0 1-1-1z" />
    </svg>
  {:else if target === 'devices'}
    <svg viewBox="0 0 20 20" class={navIconClass(target)} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
      <rect x="6" y="2.5" width="8" height="15" rx="2" />
      <path d="M9 15.2h2" />
    </svg>
  {:else if target === 'files'}
    <svg viewBox="0 0 20 20" class={navIconClass(target)} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
      <path d="M3.5 5.5a1 1 0 0 1 1-1h3l1.5 2h6a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z" />
    </svg>
  {:else}
    <svg viewBox="0 0 20 20" class={navIconClass(target)} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
      <path d="M4 6h8M16 6h0M4 12h4M12 12h4M4 18h10" />
      <circle cx="13" cy="6" r="1.6" />
      <circle cx="9" cy="12" r="1.6" />
      <circle cx="15" cy="18" r="1.6" />
    </svg>
  {/if}
{/snippet}

<div class="flex h-full overflow-hidden bg-zinc-950 text-zinc-100">
  {#if session && isTauri}
    <!-- Cermin: kotak ini yang dilaporkan ke Rust lewat ResizeObserver, jadi telepon mengisi
         tingginya dan tidak pernah meregang. -->
    <section
      class="flex min-w-0 shrink-0 flex-col border-r border-white/5 bg-zinc-900/40"
      style:width={mirrorColumnWidth}
    >
      <header class="flex items-center gap-2 border-b border-white/5 px-3 py-2">
        {#if toolbarBusy}
          <svg viewBox="0 0 16 16" class="h-3 w-3 shrink-0 animate-spin text-emerald-300" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <path d="M8 2a6 6 0 1 0 6 6" />
          </svg>
        {:else}
          <span class="relative flex h-1.5 w-1.5 shrink-0">
            <span class="absolute inline-flex h-full w-full animate-ping rounded-full {toolbarAttention ? 'bg-amber-400' : 'bg-emerald-400'} opacity-60"></span>
            <span class="relative inline-flex h-1.5 w-1.5 rounded-full {toolbarAttention ? 'bg-amber-400' : 'bg-emerald-400'}"></span>
          </span>
        {/if}
        <p
          class="min-w-0 flex-1 truncate text-[11px] {toolbarBusy
            ? 'text-zinc-400'
            : toolbarHasMessage
              ? keypadState.messageKind === 'error'
                ? 'text-rose-200'
                : 'text-amber-100'
              : 'font-mono text-zinc-400'}"
          title={toolbarText}
          role={toolbarBusy || toolbarHasMessage ? 'status' : undefined}
          aria-live={toolbarBusy || toolbarHasMessage ? 'polite' : 'off'}
        >
          {toolbarText}
        </p>
        <div class="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onclick={toggleKeyboardOwner}
            aria-pressed={keyboardOnPhone}
            title={keyboardOnPhone
              ? 'Keyboard dikirim ke HP — klik untuk kembali ke halaman'
              : 'Kirim keyboard PC ke HP (klik video juga bisa)'}
            aria-label={keyboardOnPhone ? 'Keyboard aktif ke HP' : 'Kirim keyboard ke HP'}
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border transition {keyboardOnPhone
              ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-300'
              : 'border-white/10 text-zinc-400 hover:text-zinc-200'}"
          >
            <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
              <rect x="1.5" y="4.5" width="13" height="7" rx="1.5" />
              <path d="M4 6.5h.5M6 6.5h.5M8 6.5h.5M10 6.5h.5M12 6.5h.5M4.5 9h7" />
            </svg>
          </button>
          <button
            type="button"
            onclick={() => void keypad?.toggleScreen()}
            disabled={toolbarBusy}
            title={keypadState.screenOn ? 'Matikan layar fisik HP; mirroring tetap jalan' : 'Bangunkan layar HP'}
            aria-label={keypadState.screenOn ? 'Matikan layar HP' : 'Bangunkan layar HP'}
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/10 text-zinc-400 transition hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-30"
          >
            {#if keypadState.screenOn}
              <svg viewBox="0 0 16 16" class="h-3.5 w-3.5 text-amber-300" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                <circle cx="8" cy="8" r="3" />
                <path d="M8 1v1.5M8 13.5V15M1 8h1.5M13.5 8H15M3 3l1 1M12 12l1 1M13 3l-1 1M4 12l-1 1" />
              </svg>
            {:else}
              <svg viewBox="0 0 16 16" class="h-3.5 w-3.5 text-indigo-300" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                <path d="M13 9.5A5.5 5.5 0 1 1 6.5 3 4.5 4.5 0 0 0 13 9.5Z" />
              </svg>
            {/if}
          </button>
          <button
            type="button"
            onclick={() => keypad?.toggleForm()}
            disabled={keypadState.busy}
            aria-expanded={keypadState.expanded}
            title={keypadState.expanded
              ? 'Sembunyikan kolom kunci'
              : keypadState.locked
                ? 'HP terkunci — buka kolom kunci'
                : 'Buka kolom kunci'}
            aria-label={keypadState.expanded ? 'Sembunyikan kolom kunci' : 'Buka kolom kunci'}
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border transition disabled:cursor-not-allowed disabled:opacity-30 {keypadState.expanded
              ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-300'
              : keypadState.locked
                ? 'border-amber-500/40 bg-amber-500/10 text-amber-200'
                : 'border-white/10 text-zinc-400 hover:text-zinc-200'}"
          >
            {#if keypadState.expanded}
              <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                <path d="M4 10l4-4 4 4" />
              </svg>
            {:else}
              <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
                <rect x="3" y="7" width="10" height="7" rx="1.5" />
                {#if keypadState.locked}
                  <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
                {:else}
                  <path d="M5.5 7V5a2.5 2.5 0 0 1 4.9-.6" />
                {/if}
              </svg>
            {/if}
          </button>
          <button
            type="button"
            onclick={() => (videoHidden = !videoHidden)}
            title={videoHidden ? 'Tampilkan layar HP lagi' : 'Sembunyikan layar, sesi tetap jalan'}
            aria-label={videoHidden ? 'Tampilkan layar HP' : 'Sembunyikan layar HP'}
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/10 text-zinc-400 transition hover:text-zinc-200"
          >
            {#if videoHidden}
              <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                <path d="M2 8s2.4-4 6-4 6 4 6 4-2.4 4-6 4-6-4-6-4Z" />
                <circle cx="8" cy="8" r="1.6" />
                <path d="M3 13L13 3" />
              </svg>
            {:else}
              <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                <path d="M2 8s2.4-4 6-4 6 4 6 4-2.4 4-6 4-6-4-6-4Z" />
                <circle cx="8" cy="8" r="1.6" />
              </svg>
            {/if}
          </button>
          <button
            type="button"
            onclick={togglePanel}
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/10 text-zinc-400 transition hover:text-zinc-200"
            title={panelCollapsed ? 'Tampilkan panel admin' : 'Sembunyikan panel admin'}
            aria-label={panelCollapsed ? 'Tampilkan panel admin' : 'Sembunyikan panel admin'}
            aria-expanded={!panelCollapsed}
          >
            <svg
              viewBox="0 0 20 20"
              class="h-3.5 w-3.5"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <rect x="3" y="4" width="14" height="12" rx="2" />
              <path d="M12 4v12" />
              {#if panelCollapsed}
                <path d="m7.5 8.5 2 1.5-2 1.5" />
              {:else}
                <path d="m9.5 8.5-2 1.5 2 1.5" />
              {/if}
            </svg>
          </button>
        </div>
      </header>
      <!-- The toolbar owns every action; the form below it appears only when the lock button
           asks for it (or the phone is locked), so an open phone costs one row, not three. -->
      {#key session.serial}
        <KeypadControls
          bind:this={keypad}
          serial={session.serial}
          initialScreenOn={session.screenOn}
          initialLocked={session.locked}
          embedded
          onKeyboardOwner={onKeypadKeyboardOwner}
          onState={onKeypadState}
        />
      {/key}
      <div bind:this={stageHost} class="min-h-0 flex-1 p-3">
        {#if videoHidden}
          <div class="flex h-full flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-white/10 text-center">
            <p class="text-xs text-zinc-500">Layar HP sedang disembunyikan · sesi tetap jalan</p>
            <button
              type="button"
              onclick={() => (videoHidden = false)}
              class="rounded-lg border border-white/10 px-3 py-1.5 text-[11px] text-zinc-300 transition hover:border-emerald-500/40 hover:text-emerald-300"
            >
              Tampilkan lagi
            </button>
          </div>
        {:else if mirrorLocked}
          <!-- Android never mirrors the keyguard (FLAG_SECURE): the native frame is only a
               black hole there. Un-mounting `stage` reports an empty box, which hides the
               scrcpy child while the session keeps running, and this card takes its place
               until the phone is open again. -->
          <div class="flex h-full flex-col items-center justify-center gap-3 rounded-xl border border-amber-500/25 bg-zinc-900/60 px-6 text-center">
            <svg viewBox="0 0 20 20" class="h-7 w-7 text-amber-300" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <rect x="4" y="9" width="12" height="8.5" rx="2" />
              <path d="M7 9V6.5a3 3 0 0 1 6 0V9" />
            </svg>
            <p class="text-xs font-semibold text-amber-200">HP masih terkunci</p>
            <p class="max-w-xs text-[11px] leading-relaxed text-zinc-400">
              Layar kunci tidak boleh dicerminkan oleh aplikasi apa pun, jadi area ini tetap gelap.
              Bangunkan layar lalu ketik kunci di panel atas — cermin muncul sendiri begitu HP terbuka.
            </p>
            <div class="flex flex-wrap items-center justify-center gap-2">
              <button
                type="button"
                onclick={() => void keypad?.toggleScreen()}
                disabled={toolbarBusy}
                class="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-3 py-1.5 text-[11px] font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40"
              >
                Bangunkan layar
              </button>
              {#if !keypadState.expanded}
                <button
                  type="button"
                  onclick={() => keypad?.toggleForm()}
                  class="rounded-lg border border-white/10 px-3 py-1.5 text-[11px] text-zinc-300 transition hover:border-emerald-500/40 hover:text-emerald-300"
                >
                  Buka kolom kunci
                </button>
              {/if}
            </div>
          </div>
        {:else}
          <div
            bind:this={stage}
            class="h-full w-full rounded-xl border border-dashed border-white/10 bg-black/40"
          ></div>
        {/if}
      </div>
    </section>
  {:else}
    <aside class="flex w-60 shrink-0 flex-col border-r border-white/5 bg-zinc-900/30 max-lg:w-[68px]">
      <div class="flex items-center gap-3 px-5 py-5 max-lg:justify-center max-lg:px-0">
      <div
        class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-emerald-500/15 ring-1 ring-emerald-500/30"
      >
        <svg viewBox="0 0 20 20" class="h-4 w-4 text-emerald-400" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
          <rect x="6" y="2.5" width="8" height="15" rx="2" />
          <path d="M9 15.2h2" />
        </svg>
      </div>
      <div class="min-w-0 max-lg:hidden">
        <p class="truncate text-[13px] font-semibold tracking-tight">Your Android</p>
        <p class="truncate text-[11px] text-zinc-500">on Your PC</p>
      </div>
    </div>

    <nav class="flex flex-col gap-0.5 px-3 max-lg:px-2">
      {#each navPages as item (item.page)}
        <button
          type="button"
          onclick={() => navigate(item.page)}
          class={navClass(page === item.page)}
          title={item.label}
        >
          {@render navIconSvg(item.page)}
          <span class="max-lg:hidden">{item.label}</span>
          {#if item.page === 'devices' && session}
            <span class="ml-auto flex items-center gap-1.5 text-[10px] font-medium text-emerald-400 max-lg:hidden">
              <span class="relative flex h-1.5 w-1.5">
                <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60"></span>
                <span class="relative inline-flex h-1.5 w-1.5 rounded-full bg-emerald-400"></span>
              </span>
              live
            </span>
          {:else if item.page === 'devices' && devices.length > 0}
            <span class="ml-auto rounded-full bg-white/[0.06] px-1.5 py-0.5 text-[10px] font-semibold text-zinc-400 max-lg:hidden">
              {connectedCount}/{devices.length}
            </span>
          {/if}
        </button>
      {/each}
    </nav>

    <div class="mt-auto border-t border-white/5 px-5 py-4 max-lg:px-0 max-lg:py-3">
      <div class="space-y-1.5 text-[11px] text-zinc-500 max-lg:flex max-lg:flex-col max-lg:items-center max-lg:gap-2">
        <div
          class="flex items-center gap-2 max-lg:gap-0"
          title={adb?.found ? 'ADB siap' : 'ADB tidak ditemukan'}
        >
          <span class="h-1.5 w-1.5 rounded-full {adb?.found ? 'bg-emerald-400' : 'bg-rose-400'}"></span>
          <span class="max-lg:hidden">ADB</span>
          <span class="ml-auto {adb?.found ? 'text-zinc-600' : 'text-rose-300/80'} max-lg:hidden">
            {adb?.found ? 'siap' : 'tidak ada'}
          </span>
        </div>
        <div
          class="flex items-center gap-2 max-lg:gap-0"
          title={scrcpy?.found ? 'scrcpy siap' : 'scrcpy tidak ditemukan'}
        >
          <span class="h-1.5 w-1.5 rounded-full {scrcpy?.found ? 'bg-emerald-400' : 'bg-rose-400'}"></span>
          <span class="max-lg:hidden">scrcpy</span>
          <span class="ml-auto {scrcpy?.found ? 'text-zinc-600' : 'text-rose-300/80'} max-lg:hidden">
            {scrcpy?.found ? 'siap' : 'tidak ada'}
          </span>
        </div>
      </div>
    </div>
  </aside>
  {/if}

  <!-- `@container`: page layouts size themselves from the panel they actually get, not from
       the window, because the mirror column can take most of it. The panel stays mounted while
       folding so its column can shrink/fade in sync with the Rust window animation. -->
  <main
    class="flex min-w-0 flex-1 flex-col overflow-hidden transition-opacity duration-200 ease-out @container"
    class:opacity-0={adminHidden}
    class:pointer-events-none={adminHidden}
    aria-hidden={adminHidden ? 'true' : undefined}
    inert={adminHidden}
    onfocusin={onPageFocusIn}
  >
    {#if mirroring}
      <!-- Rail ikon digantikan kolom cermin, jadi navigasi pindah ke kepala kolom halaman. -->
      <nav class="flex items-center gap-1 border-b border-white/5 px-3 py-2">
        {#each navPages as item (item.page)}
          <button
            type="button"
            onclick={() => navigate(item.page)}
            class={iconNavClass(page === item.page)}
            title={item.label}
          >
            {@render navIconSvg(item.page)}
            <span class="sr-only">{item.label}</span>
          </button>
        {/each}
      </nav>
    {/if}
    {#if page === 'devices'}
      <DevicesPage
        {devices}
        {adb}
        {scrcpy}
        {session}
        {loading}
        {refreshing}
        {error}
        {transfer}
        notice={sessionNotice}
        {knownWireless}
        {startingSerial}
        {wifiBusy}
        {wifiConnecting}
        {screenshotBusy}
        {imageBusy}
        {screenBusy}
        {screenOn}
        {screenQueryBusy}
        onRefresh={() => void refresh()}
        onConnect={(serial) => void connect(serial)}
        onDisconnect={() => void disconnect()}
        onScreenshot={(serial) => void takeScreenshot(serial)}
        onScreenPower={(serial, sleep) => void controlScreen(serial, sleep)}
        onClipboardImage={(serial) => void sendImageFromClipboard(serial)}
        onEnableWireless={(serial) => void activateWireless(serial)}
        onConnectAddress={(address) => void connectByAddress(address)}
        onForgetAddress={(address) => void removeKnownWireless(address)}
        onDismissNotice={() => (sessionNotice = null)}
      />

    {:else if page === 'files'}
      <FilesPage
        {devices}
        {session}
        bind:deviceSerial={filesSerial}
        bind:currentDir={filesDir}
        refreshKey={filesRefreshKey}
        onNotice={(kind, text) => (sessionNotice = { kind, text })}
        onNavigate={navigate}
      />
    {:else if page === 'settings'}
      <SettingsPage onNotice={(kind, text) => (sessionNotice = { kind, text })} />
    {:else}
      <HomePage
        {devices}
        {adb}
        {scrcpy}
        {session}
        {startingSerial}
        {screenshotBusy}
        {screenBusy}
        {screenOn}
        {screenQueryBusy}
        {keyguardUp}
        onNavigate={navigate}
        onConnect={(serial) => void connect(serial)}
        onDisconnect={() => void disconnect()}
        onScreenshot={(serial) => void takeScreenshot(serial)}
        onScreenPower={controlScreen}
      />
    {/if}
  </main>

  {#if sessionNotice && page !== 'devices'}
    <div class="fixed bottom-5 right-5 z-40 max-w-[calc(100vw-2.5rem)] rounded-xl border px-4 py-3 text-xs shadow-xl sm:max-w-md {sessionNotice.kind === 'error'
      ? 'border-rose-500/30 bg-rose-950/95 text-rose-200'
      : 'border-sky-500/30 bg-sky-950/95 text-sky-200'}">
      <div class="flex items-start gap-4">
        <span class="leading-relaxed">{sessionNotice.text}</span>
        <button
          type="button"
          onclick={() => (sessionNotice = null)}
          class="shrink-0 rounded-md px-1.5 py-0.5 text-[10px] uppercase tracking-wider opacity-60 transition hover:opacity-100"
        >
          Tutup
        </button>
      </div>
    </div>
  {/if}


  {#if dragging}
    <div class="pointer-events-none fixed inset-0 z-50 flex items-center justify-center bg-zinc-950/75 backdrop-blur-sm">
      <div class="rounded-2xl border-2 border-dashed border-emerald-500/50 bg-zinc-900/80 px-10 py-8 text-center">
        <p class="text-sm font-semibold text-emerald-200">
          {page === 'files' ? 'Lepaskan untuk mengunggah file' : 'Lepaskan untuk mengirim file'}
        </p>
        <p class="mt-1 text-xs text-zinc-400">
          {#if page === 'files' && filesSerial}
            Ke {filesSerial} · folder {filesDir}
          {:else if transferTarget}
            Ke {transferTarget} · folder Download di HP (bukan folder yang sedang dibuka)
          {:else}
            Belum ada perangkat tujuan — sambungkan satu perangkat atau mulai sesi
          {/if}
        </p>
      </div>
    </div>
  {/if}
</div>
