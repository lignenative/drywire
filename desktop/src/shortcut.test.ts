import { expect, test } from "bun:test";
import { shortcutFromPress } from "./shortcut";
import { recipe } from "./sounds";

const press = (code: string, mods: Partial<Record<"ctrl" | "alt" | "shift" | "meta", boolean>> = {}) => ({
  code,
  ctrlKey: !!mods.ctrl,
  altKey: !!mods.alt,
  shiftKey: !!mods.shift,
  metaKey: !!mods.meta,
});

test("a key press becomes a shortcut Tauri reads", () => {
  expect(shortcutFromPress(press("KeyM", { ctrl: true, shift: true }))).toBe("Ctrl+Shift+M");
  expect(shortcutFromPress(press("Digit5", { alt: true }))).toBe("Alt+5");
  expect(shortcutFromPress(press("F9"))).toBe("F9");
  expect(shortcutFromPress(press("Backquote", { meta: true }))).toBe("Super+Backquote");
});

test("what a game needs is refused", () => {
  expect(shortcutFromPress(press("KeyW"))).toBeNull();
  expect(shortcutFromPress(press("Space"))).toBeNull();
  expect(shortcutFromPress(press("ShiftLeft", { shift: true }))).toBeNull();
});

test("the three presence sounds differ", () => {
  const shapes = (["join", "leave", "disconnect"] as const).map((k) =>
    JSON.stringify(recipe(k).map((n) => n.hz)),
  );
  expect(new Set(shapes).size).toBe(3);
  for (const k of ["join", "leave", "disconnect"] as const) {
    const end = Math.max(...recipe(k).map((n) => n.at + n.length));
    expect(end).toBeLessThan(0.3);
  }
});
