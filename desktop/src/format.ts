// Small formatting helpers shared by the components.

/** Milliseconds for display: one decimal under 10 ms, none above. */
export function formatMs(ms: number): string {
  if (!Number.isFinite(ms)) return "–";
  return ms < 10 ? `${ms.toFixed(1)} ms` : `${Math.round(ms)} ms`;
}

/** The host of an instance's origin: https://voice.example.org → voice.example.org. */
export function hostOf(instance: string): string {
  try {
    return new URL(instance).host;
  } catch {
    return instance;
  }
}

/** A long key cut for display: the start and the end, joined by an ellipsis. */
export function shortKey(key: string, keep = 8): string {
  return key.length <= 2 * keep + 1 ? key : `${key.slice(0, keep)}…${key.slice(-keep)}`;
}
