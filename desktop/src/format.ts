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

/** The bottom of the level meter, as the Rust side clamps it. */
export const METER_FLOOR_DBFS = -90;

/** Where a level sits on the meter, from 0 to 100 %. */
export function meterPercent(dbfs: number, floor = METER_FLOOR_DBFS): number {
  if (!Number.isFinite(dbfs)) return 0;
  return Math.min(100, Math.max(0, ((dbfs - floor) / -floor) * 100));
}

/** A volume (1 is as sent) as a whole percentage. */
export function volumePercent(volume: number): number {
  return Math.round(volume * 100);
}

/** How a link is doing, from its round trip and loss, for a colour. */
export function linkQuality(rttMs: number | null, lossPct: number | null): "good" | "fair" | "poor" {
  const loss = lossPct ?? 0;
  if (rttMs === null || rttMs > 150 || loss > 5) return "poor";
  if (rttMs > 60 || loss > 1) return "fair";
  return "good";
}
