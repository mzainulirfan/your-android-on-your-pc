import type { AndroidDevice } from './types';

export interface StateStyle {
  label: string;
  badge: string;
  dot: string;
}

const stateStyles: Record<string, StateStyle> = {
  device: {
    label: 'Terhubung',
    badge: 'bg-emerald-500/10 text-emerald-300 ring-emerald-500/20',
    dot: 'bg-emerald-400',
  },
  unauthorized: {
    label: 'Perlu otorisasi',
    badge: 'bg-amber-500/10 text-amber-300 ring-amber-500/20',
    dot: 'bg-amber-400',
  },
  offline: {
    label: 'Offline',
    badge: 'bg-rose-500/10 text-rose-300 ring-rose-500/20',
    dot: 'bg-rose-400',
  },
  recovery: {
    label: 'Recovery',
    badge: 'bg-sky-500/10 text-sky-300 ring-sky-500/20',
    dot: 'bg-sky-400',
  },
};

const usbGuidance: Record<string, string> = {
  unauthorized: 'Setujui dialog “Izinkan debugging USB” di layar HP, lalu tekan Muat ulang.',
  offline: 'Perangkat tidak merespons ADB. Cabut lalu sambungkan ulang kabelnya.',
  recovery: 'HP sedang di mode recovery; mirroring baru bisa dimulai setelah ia boot normal.',
};

const wifiGuidance: Record<string, string> = {
  unauthorized: 'Buka layar HP lalu setujui dialog “Izinkan debugging USB”, kemudian Muat ulang.',
  offline:
    'Transport Wi-Fi sering hilang setelah HP restart. Hubungkan lagi lewat alamat IP di bawah.',
  recovery: 'HP sedang di mode recovery; mirroring baru bisa dimulai setelah ia boot normal.',
};

export function stateStyle(state: string): StateStyle {
  return (
    stateStyles[state] ?? {
      label: state,
      badge: 'bg-zinc-500/10 text-zinc-300 ring-zinc-500/20',
      dot: 'bg-zinc-400',
    }
  );
}

export function isReady(device: AndroidDevice): boolean {
  return device.state === 'device';
}

/** One actionable sentence per state that blocks mirroring. */
export function guidanceFor(device: AndroidDevice): string | null {
  if (isReady(device)) return null;
  const table = device.connectionType === 'wifi' ? wifiGuidance : usbGuidance;
  return (
    table[device.state] ??
    `Status ADB "${device.state}" tidak bisa dicerminkan. Tekan Muat ulang untuk keadaan terbaru.`
  );
}

export function deviceName(device: AndroidDevice): string {
  return device.model ?? device.product ?? 'Perangkat Android';
}

export function connectionLabel(device: AndroidDevice): string {
  return device.connectionType === 'wifi' ? 'Wi-Fi' : 'USB';
}

/** A wireless serial is `ip:port`; splitting it lets the port read quieter in a list. */
export function splitAddress(address: string): { host: string; port: string | null } {
  const index = address.lastIndexOf(':');
  if (index <= 0) return { host: address, port: null };
  return { host: address.slice(0, index), port: address.slice(index + 1) };
}
