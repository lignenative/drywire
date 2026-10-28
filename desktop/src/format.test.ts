import { expect, test } from "bun:test";
import { formatMs, hostOf, shortKey } from "./format";

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
