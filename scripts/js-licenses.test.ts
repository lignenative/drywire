import { expect, test } from "bun:test";
import { allowed, declared } from "./js-licenses";

test("SPDX expressions", () => {
  expect(allowed("MIT")).toBe(true);
  expect(allowed("Apache-2.0 OR MIT")).toBe(true);
  expect(allowed("(MIT OR GPL-3.0-only)")).toBe(true);
  expect(allowed("MIT AND GPL-3.0-only")).toBe(false);
  expect(allowed("(MIT AND BSD-3-Clause) OR GPL-2.0")).toBe(true);
  expect(allowed("GPL-2.0-or-later")).toBe(false);
  expect(allowed("CC-BY-4.0")).toBe(false);
  expect(allowed("Apache-2.0 WITH LLVM-exception")).toBe(false);
  expect(allowed("SEE LICENSE IN LICENSE.md")).toBe(false);
  expect(allowed("(MIT")).toBe(false);
  expect(allowed("")).toBe(false);
});

test("declared licenses, old forms included", () => {
  expect(declared({ license: "ISC" })).toBe("ISC");
  expect(declared({ license: { type: "MIT" } })).toBe("MIT");
  expect(declared({ licenses: [{ type: "MIT" }, { type: "Apache-2.0" }] })).toBe(
    "(MIT OR Apache-2.0)",
  );
  expect(declared({})).toBeUndefined();
});
