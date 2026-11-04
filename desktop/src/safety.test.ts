import { expect, test } from "bun:test";
import { safetyCode, verificationOf } from "./safety";

// Computed with Python's hashlib from ADR 0053's definition.
test("the safety code matches the ADR's definition, whatever the order", async () => {
  const a = "01".repeat(32);
  const b = "02".repeat(32);
  expect(await safetyCode(a, b)).toBe("55313 29356 05757 51644");
  expect(await safetyCode(b, a)).toBe("55313 29356 05757 51644");
  expect(
    await safetyCode(
      "568299f6a6e2ab6954e9f4c2841fe2ea3a5755bdedfc93f28362adfcbde078e1",
      "3a1dc3f7ed817e8ef21cc6d6bce6dfb60570ef8433e7578b672c6ac97a21c4a1",
    ),
  ).toBe("93145 58739 10552 32654");
});

test("a verified name seen with another key is a new key", () => {
  const verified = { aa: "sam" };
  expect(verificationOf(verified, "aa", "sam")).toBe("verified");
  expect(verificationOf(verified, "bb", "sam")).toBe("newKey");
  expect(verificationOf(verified, "bb", "kim")).toBe("none");
});
