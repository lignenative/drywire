// The account this device signed in to by e-mail (ADR 0055): kept in the
// interface's own storage, so the window knows at once whether to show
// the sign-up or the spaces. The address itself is not kept here.

export interface SignedIn {
  /** The instance's origin. */
  instance: string;
  /** The account's user name; empty until chosen. */
  username: string;
}

const KEY = "drywire.account";

export function loadAccount(): SignedIn | null {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return null;
    const value = JSON.parse(raw) as SignedIn;
    return typeof value.instance === "string" && typeof value.username === "string" ? value : null;
  } catch {
    return null;
  }
}

export function saveAccount(account: SignedIn | null): void {
  try {
    if (account === null) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, JSON.stringify(account));
  } catch {
    // Without storage, the next start asks to sign in again: harmless.
  }
}

/** Only the digits of a code as typed or pasted, six at most. */
export function codeDigits(text: string): string {
  return text.replace(/\D/g, "").slice(0, 6);
}
