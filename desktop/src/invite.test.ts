import { expect, test } from "bun:test";
import { invitationProblem } from "./invite";

test("an invitation link starts like the ones the core accepts", () => {
  expect(invitationProblem("https://drywire.gg/i#AQIDBAUG", "alex")).toBeNull();
  expect(invitationProblem("  drywire://i#AQID-_  ", "alex")).toBeNull();
  expect(invitationProblem("https://example.org/i#AQID", "alex")).toBe("invalid_link");
  expect(invitationProblem("https://drywire.gg/i#", "alex")).toBe("invalid_link");
});

test("a name is required, and the link is reported first", () => {
  expect(invitationProblem("https://drywire.gg/i#AQID", "   ")).toBe("name_missing");
  expect(invitationProblem("nope", "")).toBe("invalid_link");
  expect(invitationProblem("https://drywire.gg/i#AQID", "x".repeat(65))).toBe("name_missing");
});
