import { expect, test } from "bun:test";
import { formatMs, hostOf, linkQuality, meterPercent, shortKey, volumePercent } from "./format";

test("milliseconds keep one decimal only when small", () => {
  expect(formatMs(2.345)).toBe("2.3 ms");
  expect(formatMs(28.6)).toBe("29 ms");
  expect(formatMs(Number.NaN)).toBe("–");
});

test("an instance is shown by its host", () => {
  expect(hostOf("https://voice.example.org")).toBe("voice.example.org");
  expect(hostOf("http://127.0.0.1:10050")).toBe("127.0.0.1:10050");
  expect(hostOf("not a url")).toBe("not a url");
});

test("long keys are cut in the middle", () => {
  expect(shortKey("a".repeat(64))).toBe("aaaaaaaa…aaaaaaaa");
  expect(shortKey("abc")).toBe("abc");
});

test("the meter spans the floor to full scale", () => {
  expect(meterPercent(-90)).toBe(0);
  expect(meterPercent(-45)).toBe(50);
  expect(meterPercent(0)).toBe(100);
  expect(meterPercent(6)).toBe(100);
  expect(meterPercent(-Infinity)).toBe(0);
});

test("volumes are whole percentages", () => {
  expect(volumePercent(1)).toBe(100);
  expect(volumePercent(1.234)).toBe(123);
});

test("link quality", () => {
  expect(linkQuality(14, 0)).toBe("good");
  expect(linkQuality(80, 0)).toBe("fair");
  expect(linkQuality(20, 3)).toBe("fair");
  expect(linkQuality(null, 0)).toBe("poor");
  expect(linkQuality(20, 10)).toBe("poor");
});
