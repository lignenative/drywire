import { describe, expect, test } from "bun:test";
import { detectLanguage, dictionaries, placeholders, translate } from "./i18n";

describe("i18n", () => {
  test("both languages have the same keys and placeholders", () => {
    const en = dictionaries.en;
    const fr = dictionaries.fr;
    expect(Object.keys(fr).sort()).toEqual(Object.keys(en).sort());
    for (const key of Object.keys(en) as (keyof typeof en)[]) {
      expect({ key, vars: placeholders(fr[key]) }).toEqual({
        key,
        vars: placeholders(en[key]),
      });
      expect(fr[key].trim().length).toBeGreaterThan(0);
    }
  });

  test("placeholders are filled, unknown ones kept", () => {
    expect(translate("en", "accept.done", { space: "Team" })).toBe(
      "You are now a member of Team.",
    );
    expect(translate("fr", "accept.done")).toContain("{space}");
  });

  test("the system's language is used when the app speaks it", () => {
    expect(detectLanguage(["fr-FR", "en-US"])).toBe("fr");
    expect(detectLanguage(["de-DE", "en-GB"])).toBe("en");
    expect(detectLanguage(["de-DE"])).toBe("en");
    expect(detectLanguage([])).toBe("en");
  });
});
