// The global mute shortcut, recorded from a key press in the settings
// panel and written in the syntax Tauri's global-shortcut plugin reads:
// modifiers, then the key, joined by "+", such as "Ctrl+Shift+M".

export interface KeyPress {
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  /** The physical key, as KeyboardEvent.code names it: "KeyM", "F9". */
  code: string;
}

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/;

/** The shortcut a key press makes, or null when it cannot be one: a
    modifier alone, or a plain key that games use (letters, digits) with
    no modifier. Function keys may stand alone. */
export function shortcutFromPress(press: KeyPress): string | null {
  if (MODIFIER_CODES.test(press.code)) return null;
  const key = press.code.replace(/^Key/, "").replace(/^Digit/, "");
  const modifiers = [
    press.ctrlKey && "Ctrl",
    press.altKey && "Alt",
    press.shiftKey && "Shift",
    press.metaKey && "Super",
  ].filter((m): m is string => !!m);
  const functionKey = /^F([1-9]|1[0-9]|2[0-4])$/.test(key);
  if (modifiers.length === 0 && !functionKey) return null;
  return [...modifiers, key].join("+");
}
