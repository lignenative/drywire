// What the web client keeps in this browser, for this site only: the
// spaces it joined, with their keys, and the person's preferences. The
// identity's private key is elsewhere, non-extractable (identity.ts).
//
// The space key is what the desktop app keeps in its own files: it lets
// this browser decrypt the space's rooms, and it never leaves the browser
// (spec section 8). Clearing the site's data forgets everything.

/** A space this browser joined. */
export interface StoredSpace {
  /** The instance's canonical origin (protocol section 13.3). */
  instance: string;
  id: string;
  name: string;
  role: string;
  /** The space key from the invitation link, in hex. */
  spaceKey: string;
  /** The rooms as last read, shown while the server cannot be reached. */
  rooms: { id: string; name: string }[];
}

/** The person's preferences, as the desktop app's settings.json holds
    them, without what a browser cannot do. */
export interface WebSettings {
  gateOpenDbfs: number | null;
  /** Per person, by public key: 1 is 100 %, 2 the most. */
  volumes: Record<string, number>;
  soundVolume: number;
}

const SPACES = "drywire.web.spaces";
const RECOVERY = "drywire.web.recovery";
const SETTINGS = "drywire.web.settings";

function read<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : { ...fallback, ...JSON.parse(raw) };
  } catch {
    return fallback;
  }
}

export function loadSpaces(): StoredSpace[] {
  try {
    const raw = localStorage.getItem(SPACES);
    return raw === null ? [] : (JSON.parse(raw) as StoredSpace[]);
  } catch {
    return [];
  }
}

export function saveSpaces(spaces: StoredSpace[]): void {
  localStorage.setItem(SPACES, JSON.stringify(spaces));
}

export function loadSettings(): WebSettings {
  return read<WebSettings>(SETTINGS, { gateOpenDbfs: null, volumes: {}, soundVolume: 0.5 });
}

export function saveSettings(settings: WebSettings): void {
  localStorage.setItem(SETTINGS, JSON.stringify(settings));
}

/** The recovery code's secret, in hex, when this browser made it or was
    restored with it: it seals the recovery vault again when the spaces
    change (ADR 0051). As secret as the account key itself. */
export function loadRecovery(): string | null {
  try {
    return localStorage.getItem(RECOVERY);
  } catch {
    return null;
  }
}

export function saveRecovery(secret: string | null): void {
  if (secret === null) localStorage.removeItem(RECOVERY);
  else localStorage.setItem(RECOVERY, secret);
}
