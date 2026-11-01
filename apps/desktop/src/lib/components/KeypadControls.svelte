<script module lang="ts">
  // Keys that already failed to open the phone in this app run. Remember-me skips them on
  // later reveals instead of spending one of the phone's retries on every unlock attempt.
  // Module scope: it has to outlive the per-session remount of this component.
  const autoFailed = new Set<string>();
</script>

<script lang="ts">
  import {
    deleteCredential,
    keyguardLocked,
    listCredentials,
    reportUnlocked,
    saveCredential,
    screenState,
    setKeyboardOwner,
    setScreenPower,
    unlockDevice,
    unlockWithCredential,
    wakeState,
  } from '../api';
  import type { KeypadState, SavedCredential, UnlockReport } from '../types';

  interface Props {
    serial: string;
    /** Display state read before the session started; `null` makes the panel read it. */
    initialScreenOn?: boolean | null;
    /** Keyguard state read before the session started; `null` makes the panel read it. */
    initialLocked?: boolean | null;
    /** Tells the window which side just got the keyboard, so its chip stays truthful. */
    onKeyboardOwner?: (owner: 'phone' | 'pc') => void;
    /** State for the toolbar that sits above this form; the form itself owns the unlock flow. */
    onState?: (state: KeypadState) => void;
    /** The toolbar already shows the status and wake action; only the form is needed here. */
    embedded?: boolean;
  }

  let {
    serial,
    initialScreenOn = null,
    initialLocked = null,
    onKeyboardOwner,
    onState,
    embedded = false,
  }: Props = $props();

  type Notice = { kind: 'ok' | 'locked' | 'error'; text: string };

  const MIN_PIN = 4;
  const MAX_KEY = 64;
  // How often the panel checks whether the phone was opened elsewhere (mirror or device).
  // Each check is a `dumpsys` on system_server (measured 210–730 ms), so it stays well away
  // from the frames the mirror is streaming: the `device-unlocked` event covers its own
  // unlocks, and this timer only has to catch a PIN typed into the mirror.
  const WATCH_MS = 10000;

  const LOCKED_HINT: Notice = {
    kind: 'locked',
    text: 'HP terkunci · ketik kunci di kolom ini atau di cermin.',
  };
  const UNLOCKED_HINT: Notice = { kind: 'ok', text: 'HP sudah terbuka.' };

  // Typed keys live in memory only for the moment of sending. A key the user explicitly
  // saved comes back as a chip that sends by id, so the cleartext never reaches this
  // component from storage.
  let secret = $state('');
  let revealKey = $state(false);
  // Opt-in for this attempt only: the key is stored after the unlock verdict says it worked,
  // never before. Off by default so saving stays an explicit choice.
  let remember = $state(false);
  let saved = $state<SavedCredential[]>([]);
  let busy = $state(false);
  // The chip (or field) currently being used, so only that one shows the spinner.
  let busyId = $state<string | null>(null);
  // Deleting a saved key takes two clicks: the first turns the ✕ into "Hapus?".
  let confirmId = $state<string | null>(null);
  let confirmTimer: number | undefined;
  let screenBusy = $state(false);
  let screenOn = $state(true);
  // Folded by default whenever the phone's lock state is known: an unlocked phone needs no
  // field, and a locked one first spends the stored key without flashing a form that may
  // close again. Only an unknown state opens the field immediately, because there is nothing
  // else the user could do. The component is remounted per session, so this is just the seed.
  // svelte-ignore state_referenced_locally
  let compact = $state(initialLocked !== null);
  let booted = $state(false);
  let notice = $state<Notice | null>(null);
  let field: HTMLInputElement | undefined = $state();
  let watching = false;

  // The bar above the video mirrors the form instead of duplicating it: it shows where the
  // phone needs attention and calls back into `expand`/`fold`/`toggleScreen` below.
  $effect(() => {
    const kind = notice?.kind === 'locked' || notice?.kind === 'error' ? notice.kind : null;
    onState?.({
      expanded: !compact,
      screenOn,
      screenBusy,
      busy,
      locked: notice?.kind === 'locked',
      message: kind ? (notice?.text ?? null) : null,
      messageKind: kind,
    });
  });

  const headline = $derived(notice?.text ?? (serial ? `Panel · ${serial}` : 'Panel Kontrol'));
  // Amber means the user has something to do (screen off or keyguard up), not just "screen off".
  const dotTone = $derived(screenOn && notice?.kind !== 'locked' ? 'bg-emerald-400' : 'bg-amber-400');

  const digitsOnly = $derived(secret.length > 0 && /^[0-9]+$/.test(secret));
  const canSend = $derived(secret.length > 0 && (!digitsOnly || secret.length >= MIN_PIN));
  const sendTitle = $derived(
    canSend
      ? 'Kirim kunci ke HP'
      : digitsOnly
        ? `PIN minimal ${MIN_PIN} digit`
        : 'Ketik kunci dulu',
  );
  const errorText = (cause: unknown) =>
    typeof cause === 'string' ? cause : 'Terjadi kesalahan yang tidak terduga.';

  async function refreshSaved() {
    if (!serial) {
      saved = [];
      return saved;
    }
    try {
      saved = await listCredentials(serial);
    } catch {
      saved = [];
    }
    return saved;
  }

  $effect(() => {
    if (booted || !serial) return;
    booted = true;
    void boot();
  });

  async function boot() {
    // The session prep already probed the phone, so the panel starts in the shape it is shown
    // in. Only a failed probe leaves it guessing.
    const locked = initialLocked !== null ? initialLocked : await readState();
    if (initialLocked !== null) {
      notice = locked ? LOCKED_HINT : UNLOCKED_HINT;
      if (initialScreenOn !== null) {
        screenOn = initialScreenOn;
      } else {
        void refreshScreen();
      }
    }
    const list = await refreshSaved();

    if (locked === false) {
      compact = true;
      giveKeyboardBack();
      return;
    }
    if (locked === true) {
      // Remember-me: a locked phone opens with the newest usable stored key, no extra
      // click. One attempt per reveal; the form returns only when that attempt fails.
      if (list.length > 0) {
        compact = true;
        if (await autoUnlock(list)) return;
      }
      compact = false;
      focusField();
      return;
    }
    // Probe failed: leave the field available.
    if (!compact) focusField();
  }

  // The phone can also be opened outside the panel: a PIN typed into the mirror over uhid, or
  // a fingerprint on the device. While the form is showing, one filtered keyguard read every
  // few seconds lets the panel fold away on its own; it stops as soon as the panel is compact.
  $effect(() => {
    if (compact || !serial) return;
    const timer = window.setInterval(() => {
      if (busy || document.visibilityState !== 'visible') return;
      void watchUnlock();
    }, WATCH_MS);
    return () => window.clearInterval(timer);
  });

  async function readState(): Promise<boolean | null> {
    if (!serial) return null;
    try {
      const state = await wakeState(serial);
      screenOn = state.screenOn;
      if (state.keyguardUp) {
        notice = LOCKED_HINT;
        return true;
      }
      notice = UNLOCKED_HINT;
      await collapse();
      return false;
    } catch {
      // Leave the field available: a failed read must not hide the only way in.
      return null;
    }
  }

  /**
   * Remember-me: type the newest stored key into a phone that is showing its lock screen.
   * The cleartext never comes back to this component — Rust reads the blob and types it.
   */
  async function autoUnlock(keys: SavedCredential[]): Promise<boolean> {
    if (!serial || busy) return false;
    // Newest first (credentials.rs inserts at the front), skipping keys that already failed
    // in this run so a stale PIN is not retried on every reveal.
    const credential = keys.find((item) => !autoFailed.has(item.id));
    if (!credential) return false;
    busy = true;
    busyId = credential.id;
    notice = { kind: 'ok', text: `Membuka dengan ${credential.label} tersimpan…` };
    try {
      const result = await unlockWithCredential(serial, credential.id);
      if (result.unlocked) {
        screenOn = true;
        notice = { kind: 'ok', text: result.notes.join(' ') || 'HP terbuka.' };
        await collapse();
      } else {
        // The stored key is stale: stop guessing (the next reveal may try the next stored
        // key) and hand the form back with the field ready.
        autoFailed.add(credential.id);
        expand();
        notice = {
          kind: 'locked',
          text: `${credential.label} tidak membuka HP — ketik kunci atau pilih kunci lain.`,
        };
      }
    } catch (cause) {
      expand();
      notice = { kind: 'error', text: errorText(cause) };
    } finally {
      busy = false;
      busyId = null;
    }
    return true;
  }

  async function watchUnlock() {
    if (!serial || compact || watching) return;
    watching = true;
    try {
      if (await keyguardLocked(serial)) return;
      screenOn = true;
      notice = UNLOCKED_HINT;
      await collapse();
    } catch {
      // Keep waiting; a transient ADB error is not an unlock.
    } finally {
      watching = false;
    }
  }

  async function refreshScreen() {
    if (!serial) return;
    try {
      screenOn = await screenState(serial);
    } catch {
      // Leave the last known state.
    }
  }

  function focusField() {
    // The caret belongs to the page now: the field is where the key is typed.
    handKeysTo('pc');
    requestAnimationFrame(() => field?.focus());
  }

  // Tell Rust who holds the keyboard and let the window's own indicator follow.
  function handKeysTo(owner: 'phone' | 'pc') {
    onKeyboardOwner?.(owner);
    void setKeyboardOwner(owner);
  }

  /**
   * The phone owns the keyboard while the form is folded: typing in the page would only
   * steal keys that have nowhere to go, because the mirror is the thing being controlled.
   */
  function giveKeyboardBack() {
    if (compact) handKeysTo('phone');
  }

  /** Hide the field and the saved-key chips; the header row stays where it is. */
  async function fold() {
    if (!compact) compact = true;
    giveKeyboardBack();
  }

  /** Fold after an unlock: the dashboard can drop its "terkunci" banner right away. */
  async function collapse() {
    await fold();
    void reportUnlocked(serial);
  }

  /** Bring the credential rows back for a key to be typed. */
  function expand() {
    if (!compact) return;
    compact = false;
    focusField();
  }

  /** Toolbar hook: the lock button opens the form, or folds it when it is already open. */
  export function toggleForm() {
    if (compact) expand();
    else void fold();
  }

  function report(result: UnlockReport) {
    if (result.unlocked) {
      screenOn = true;
      // The key has been used once; give the keyboard back to the phone so typing reaches
      // it instead of the field, and hide the form that is no longer needed.
      notice = { kind: 'ok', text: result.notes.join(' ') || 'HP terbuka.' };
      void collapse();
    } else {
      notice = {
        kind: 'locked',
        text: result.notes.join(' ') || 'Masih terkunci — periksa kunci.',
      };
      // Keep the key for a correction, selected so typing replaces it in one go.
      if (secret) requestAnimationFrame(() => field?.select());
    }
  }

  async function send() {
    if (!serial || busy || !canSend) return;
    busy = true;
    notice = null;
    try {
      const result = await unlockDevice(serial, secret);
      report(result);
      // "Ingat" only stores a key that actually opened the phone; a wrong key never becomes
      // a chip. The value goes straight to Rust, the UI only ever sees the metadata.
      if (result.unlocked && remember) {
        const label = labelFor(secret);
        try {
          await saveCredential(serial, label, secret);
          await refreshSaved();
          remember = false;
          notice = { kind: 'ok', text: `Terbuka · ${label} tersimpan di PC ini.` };
        } catch (cause) {
          notice = {
            kind: 'error',
            text: `Terbuka, tetapi kunci gagal disimpan: ${errorText(cause)}`,
          };
        }
      }
      secret = '';
    } catch (cause) {
      notice = { kind: 'error', text: errorText(cause) };
    } finally {
      busy = false;
    }
  }

  async function pick(credential: SavedCredential) {
    if (!serial || busy) return;
    busy = true;
    busyId = credential.id;
    notice = null;
    try {
      const result = await unlockWithCredential(serial, credential.id);
      // This key works after all (the phone may just have been asleep); let remember-me
      // consider it again on the next reveal.
      if (result.unlocked) autoFailed.delete(credential.id);
      report(result);
    } catch (cause) {
      notice = { kind: 'error', text: errorText(cause) };
    } finally {
      busy = false;
      busyId = null;
    }
  }

  /** The chip says what it is ("PIN 6", "Kunci") — no name to type before saving. */
  function labelFor(value: string) {
    return /^[0-9]+$/.test(value) ? `PIN ${value.length}` : 'Kunci';
  }

  /** Deleting a saved key is destructive and silent, so the chip asks first. */
  function askRemove(credential: SavedCredential) {
    if (busy) return;
    window.clearTimeout(confirmTimer);
    if (confirmId === credential.id) {
      confirmId = null;
      void remove(credential);
      return;
    }
    confirmId = credential.id;
    confirmTimer = window.setTimeout(() => (confirmId = null), 3000);
  }

  async function remove(credential: SavedCredential) {
    if (busy) return;
    busy = true;
    busyId = credential.id;
    try {
      await deleteCredential(credential.id);
      await refreshSaved();
    } catch (cause) {
      notice = { kind: 'error', text: errorText(cause) };
    } finally {
      busy = false;
      busyId = null;
    }
  }

  /** Toolbar hook: wake the phone, or put its screen to sleep while mirroring continues. */
  export async function toggleScreen() {
    if (!serial || screenBusy) return;
    screenBusy = true;
    notice = null;
    const sleep = screenOn;
    try {
      // One Rust call does the key event, the waits, the dismiss and both verdicts on the
      // phone, so the field is usable the moment the wake settles.
      const state = await setScreenPower(serial, sleep);
      screenOn = state.screenOn;
      if (sleep) {
        notice = { kind: 'ok', text: 'Layar HP dimatikan — mirroring tetap jalan.' };
        giveKeyboardBack();
      } else if (state.keyguardUp) {
        // Waking into a lock screen is the same situation as opening a locked panel, so a
        // remembered key opens it here too — the form only comes back if that key fails.
        const list = await refreshSaved();
        if (!(await autoUnlock(list))) {
          expand();
          notice = { kind: 'locked', text: 'Layar terkunci — pilih atau ketik kunci.' };
        }
      } else {
        // No keyguard to dismiss: the panel has nothing to ask, so it stays folded and the
        // keyboard goes back to the phone.
        notice = { kind: 'ok', text: 'Layar terbuka.' };
        await collapse();
      }
    } catch (cause) {
      notice = { kind: 'error', text: errorText(cause) };
    } finally {
      screenBusy = false;
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      void send();
    } else if (event.key === 'Escape' && secret) {
      event.preventDefault();
      secret = '';
    }
  }

  $effect(() => () => window.clearTimeout(confirmTimer));

  const chip =
    'flex h-7 shrink-0 items-center overflow-hidden rounded-full border border-emerald-500/25 bg-emerald-500/10';
  const square =
    'flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-white/10 bg-white/5 text-zinc-300 transition hover:bg-white/10 hover:text-zinc-100 disabled:cursor-not-allowed disabled:opacity-30';
</script>

{#if !embedded || !compact}
  <div class="{embedded ? 'mx-3 mt-3' : 'w-full'} flex shrink-0 flex-col gap-2 rounded-xl border border-white/5 bg-zinc-950/60 px-3 py-2">
    {#if !embedded}
      <div class="flex items-center gap-2">
        {#if busy}
          <!-- The key is being typed on the phone; the spinner says so without a form row. -->
          <svg viewBox="0 0 16 16" class="h-3 w-3 shrink-0 animate-spin text-emerald-300" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <path d="M8 2a6 6 0 1 0 6 6" />
          </svg>
        {:else}
          <span class="relative flex h-2 w-2 shrink-0">
            <span class="absolute inline-flex h-full w-full animate-ping rounded-full {dotTone} opacity-60"></span>
            <span class="relative inline-flex h-2 w-2 rounded-full {dotTone}"></span>
          </span>
        {/if}
        <span
          title={headline}
          role="status"
          aria-live="polite"
          class="min-w-0 flex-1 truncate text-[11px] font-semibold tracking-wide {notice?.kind ===
          'error'
            ? 'text-rose-200'
            : notice?.kind === 'locked'
              ? 'text-amber-100'
              : notice?.kind === 'ok'
                ? 'text-emerald-100'
                : 'text-zinc-400'}"
        >
          {headline}
        </span>

        {#if compact}
          <button
            type="button"
            onclick={() => expand()}
            disabled={busy}
            title="Tampilkan kolom kunci"
            aria-label="Tampilkan kolom kunci"
            class="{square} text-emerald-300"
          >
            <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
              <rect x="3" y="7" width="10" height="7" rx="1.5" />
              <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
            </svg>
          </button>
        {:else}
          <button
            type="button"
            onclick={() => void fold()}
            disabled={busy}
            title="Sembunyikan kolom kunci"
            aria-label="Sembunyikan kolom kunci"
            class={square}
          >
            <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
              <path d="M4 10l4-4 4 4" />
            </svg>
          </button>
        {/if}
        <button
          type="button"
          onclick={() => void toggleScreen()}
          disabled={!serial || screenBusy || busy}
          title={screenOn ? 'Matikan layar fisik HP; mirroring tetap jalan' : 'Bangunkan layar HP'}
          aria-label={screenOn ? 'Matikan layar HP' : 'Bangunkan layar HP'}
          class={square}
        >
          {#if screenOn}
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
      </div>
    {/if}

  {#if !compact}
    <div class="flex items-center gap-1.5">
      <label
        class="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md border border-white/12 bg-black/40 px-2 transition focus-within:border-emerald-500/50"
      >
        <svg viewBox="0 0 16 16" class="h-3.5 w-3.5 shrink-0 text-zinc-500" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="7" width="10" height="7" rx="1.5" />
          <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
        </svg>
        <input
          bind:this={field}
          bind:value={secret}
          onkeydown={onKeydown}
          type={revealKey ? 'text' : 'password'}
          maxlength={MAX_KEY}
          autocomplete="off"
          autocorrect="off"
          autocapitalize="off"
          spellcheck="false"
          aria-label="PIN atau password"
          placeholder={saved.length ? 'Ketik PIN atau password' : `PIN / password · min. ${MIN_PIN} digit`}
          class="h-full min-w-0 flex-1 bg-transparent text-[13px] tracking-wider text-zinc-100 outline-none placeholder:text-[11px] placeholder:tracking-normal placeholder:text-zinc-600"
        />
        <button
          type="button"
          onclick={() => (revealKey = !revealKey)}
          disabled={!secret}
          title={revealKey ? 'Sembunyikan' : 'Perlihatkan'}
          aria-label={revealKey ? 'Sembunyikan kunci' : 'Perlihatkan kunci'}
          class="flex h-6 w-6 shrink-0 items-center justify-center rounded text-zinc-500 transition hover:bg-white/10 hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-0"
        >
          {#if revealKey}
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
      </label>

      <button
        type="button"
        onclick={() => void send()}
        disabled={!serial || busy || !canSend}
        title={sendTitle}
        class="flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-emerald-500/30 bg-emerald-500/15 px-3 text-[11px] font-semibold text-emerald-100 transition hover:bg-emerald-500/25 active:bg-emerald-500/30 disabled:cursor-not-allowed disabled:opacity-30"
      >
        {#if busy}
          <svg viewBox="0 0 16 16" class="h-3.5 w-3.5 animate-spin" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <path d="M8 2a6 6 0 1 0 6 6" />
          </svg>
        {:else}
          <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 8.5l3 3L13 5" />
          </svg>
        {/if}
        <span>Kirim</span>
      </button>
      <button
        type="button"
        onclick={() => (remember = !remember)}
        disabled={!serial}
        aria-pressed={remember}
        title={remember
          ? 'Kunci akan disimpan di PC ini setelah berhasil membuka'
          : 'Ingat kunci ini di PC ini setelah berhasil membuka'}
        class="flex h-8 shrink-0 items-center gap-1.5 rounded-md border px-2.5 text-[11px] font-medium transition disabled:cursor-not-allowed disabled:opacity-30 {remember
          ? 'border-emerald-500/40 bg-emerald-500/20 text-emerald-100'
          : 'border-white/10 bg-white/5 text-zinc-400 hover:bg-white/10 hover:text-zinc-200'}"
      >
        <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" fill={remember ? 'currentColor' : 'none'} stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 2.5h8v11l-4-2.5-4 2.5Z" />
        </svg>
        <span>Ingat</span>
      </button>
    </div>

    {#if saved.length}
      <div class="flex h-7 items-center gap-1.5 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        <span class="shrink-0 text-[10px] uppercase tracking-wider text-zinc-600">
          Tersimpan · {saved.length}
        </span>
        {#each saved as item (item.id)}
          <span class="{chip} {confirmId === item.id ? 'border-rose-500/40 bg-rose-500/10' : ''}">
            <button
              type="button"
              onclick={() => void pick(item)}
              disabled={busy}
              title="Kirim {item.label} ke HP"
              class="flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium text-emerald-100 transition hover:bg-emerald-500/20 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {#if busyId === item.id}
                <svg viewBox="0 0 16 16" class="h-3 w-3 animate-spin" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
                  <path d="M8 2a6 6 0 1 0 6 6" />
                </svg>
              {/if}
              {item.label}
              {#if item.method === 'text'}
                <span class="text-emerald-200/50">· {item.length}</span>
              {/if}
            </button>
            <button
              type="button"
              onclick={() => askRemove(item)}
              disabled={busy}
              aria-label={confirmId === item.id
                ? `Konfirmasi hapus ${item.label}`
                : `Hapus ${item.label}`}
              title={confirmId === item.id ? 'Klik lagi untuk menghapus' : 'Hapus kunci ini'}
              class="h-full border-l px-1.5 text-[11px] transition disabled:cursor-not-allowed disabled:opacity-40 {confirmId ===
              item.id
                ? 'border-rose-500/30 bg-rose-500/20 text-rose-100'
                : 'border-emerald-500/20 text-emerald-200/40 hover:bg-rose-500/20 hover:text-rose-200'}"
            >
              {confirmId === item.id ? 'Hapus?' : '✕'}
            </button>
          </span>
        {/each}
      </div>
    {/if}
  {/if}
  </div>
{/if}
