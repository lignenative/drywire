// The web client's vaults against drywire-proto's (core/drywire-proto/
// src/vault.rs): same codes, same derivations, same sealed bytes.
import { expect, test } from "bun:test";
import { BadCode, formatCode, open, parseCode, seal, vaultId, vaultKeyBytes, WrongCode } from "./vault";

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const counting = Uint8Array.from({ length: 16 }, (_, i) => i);

test("codes read back as written, and match the Rust vectors", async () => {
  expect(formatCode(counting)).toBe("000G-40R4-0M30-E209-185G-R38E-1W");
  expect(formatCode(new Uint8Array(16).fill(0xff))).toBe("ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZW");
  expect(parseCode("000G-40R4-0M30-E209-185G-R38E-1W")).toEqual(counting);
  // As people type it: any case, spaces, O for 0, I or L for 1.
  expect(parseCode("ooog 40r4 om30 e209 185g r38e 1w")).toEqual(counting);
  expect(hex(await vaultId(counting))).toBe("2079b980fe62fb5d556bcd04252b5272df06a4cb49b62856f121d1a7a2c94561");
  expect(hex(await vaultKeyBytes(counting))).toBe("bf3d34d37b1b420ef4cb57c95e29cd69de59c0d859aadfea71c28b5e797e5831");
});

test("anything else is not a code", () => {
  for (const bad of ["", "000G-40R4", "UUUU-UUUU-UUUU-UUUU-UUUU-UUUU-UU", "ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZ", "ÉZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-Z0"]) {
    expect(() => parseCode(bad)).toThrow(BadCode);
  }
});

test("sealing matches the Rust bytes, and only the code opens them", async () => {
  const secret = new Uint8Array(16).fill(1);
  const plaintext = new TextEncoder().encode('{"name":"alex"}');
  const sealed = await seal(secret, plaintext, new Uint8Array(12).fill(2));
  expect(hex(sealed)).toBe("01020202020202020202020202c4b32882c24761db6379d20a7cd413887ba4b217e29138955482efa7bde3b5");
  expect(await open(secret, sealed)).toEqual(plaintext);
  expect(open(new Uint8Array(16).fill(3), sealed)).rejects.toThrow(WrongCode);
  const damaged = sealed.slice();
  damaged[damaged.length - 1] ^= 1;
  expect(open(secret, damaged)).rejects.toThrow(WrongCode);
});
