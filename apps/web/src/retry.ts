// When a lost session tries to join again (ADR 0035): after 1, 2, 4, 8,
// then every 15 seconds. The person may cut a wait short; the attempt
// they start restarts the sequence, so if it fails the next one comes a
// second later (ADR 0050). The desktop app's session thread does the same
// (desktop/src-tauri/src/session.rs, `Retry`).

/** ADR 0035's pauses before each attempt, in seconds, the last repeating. */
export const RETRY_S = [1, 2, 4, 8, 15];

export class Retries {
  /** Which pause comes next. */
  private step = 0;
  /** When the waiting attempt starts (performance.now()), null when
      nothing waits: an attempt runs, or none is planned. */
  private at: number | null = null;
  /** Ends the current wait; set only while waiting. */
  private wake: (() => void) | null = null;

  constructor(private readonly pausesMs: readonly number[] = RETRY_S.map((s) => s * 1000)) {}

  /** Waits for the next pause of the sequence, or less if `now()` is
      called meanwhile; resolves true when it was cut short. */
  wait(): Promise<boolean> {
    const ms = this.pausesMs[Math.min(this.step, this.pausesMs.length - 1)];
    this.step++;
    this.at = performance.now() + ms;
    return new Promise((done) => {
      const finish = (asked: boolean) => {
        clearTimeout(timer);
        this.wake = null;
        this.at = null;
        if (asked) this.step = 0;
        done(asked);
      };
      const timer = setTimeout(() => finish(false), ms);
      this.wake = () => finish(true);
    });
  }

  /** Starts the waiting attempt now. Returns false when nothing waits:
      during an attempt a click asks for what is already happening, so it
      never starts a second one. */
  now(): boolean {
    if (!this.wake) return false;
    this.wake();
    return true;
  }

  /** Whole seconds until the next attempt, rounded up so the countdown
      never shows 0 before it starts; null during an attempt. */
  secondsLeft(): number | null {
    return this.at === null ? null : Math.max(0, Math.ceil((this.at - performance.now()) / 1000));
  }
}
