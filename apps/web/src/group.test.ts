// The browser's group loop (group.ts) with the real OpenMLS module, and a
// delivery service played in memory as the server orders it (protocol
// section 16): two browsers end up with the same room key, and a member
// who leaves is taken out. Skipped where scripts/mls-wasm.sh has not built
// the module (it needs wasm-bindgen).
import { expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import type { ControlPlane, SpaceAnswer } from "./control";
import { StaleEpoch } from "./control";
import type { Identity } from "./identity";
import { hex } from "./protocol";

const glue = new URL("./mls-wasm/drywire_mls_wasm.js", import.meta.url);
const built = existsSync(glue);

// Bun has no localStorage: a map stands in for it.
const stored = new Map<string, string>();
(globalThis as { localStorage?: unknown }).localStorage ??= {
  getItem: (k: string) => stored.get(k) ?? null,
  setItem: (k: string, v: string) => void stored.set(k, v),
  removeItem: (k: string) => void stored.delete(k),
  clear: () => stored.clear(),
};

const SPACE = "05".repeat(16);
const ROOM = "06".repeat(16);

/** The server's side, in memory. */
class Service {
  commits: Uint8Array[] = [];
  packages = new Map<string, Uint8Array>();
  welcomes = new Map<string, { epoch: number; welcome: Uint8Array }>();
  members = new Set<string>();

  control(): ControlPlane {
    return {
      space: async (): Promise<SpaceAnswer> => ({
        space: { id: SPACE, name: "Team" },
        role: "member",
        rooms: [{ id: ROOM, name: "general" }],
        members: [...this.members].map((d) => ({ devices: [d] })),
      }),
      welcome: async (_s: string, device: string) => this.welcomes.get(device)?.welcome ?? null,
      commits: async (_s: string, after: number) =>
        this.commits.slice(after).map((commit, i) => ({ epoch: after + i + 1, commit })),
      postCommit: async (_s: string, epoch: number, commit: Uint8Array, welcome: Uint8Array, welcomed: Uint8Array) => {
        if (epoch !== this.commits.length) throw new StaleEpoch();
        this.commits.push(commit);
        for (let i = 0; i + 32 <= welcomed.length; i += 32) {
          const device = hex(welcomed.subarray(i, i + 32));
          this.welcomes.set(device, { epoch: this.commits.length, welcome });
          this.packages.delete(device);
        }
        return this.commits.length;
      },
      postKeyPackage: async (_s: string, device: string, keyPackage: Uint8Array) => {
        this.packages.set(device, keyPackage);
      },
      keyPackages: async () =>
        [...this.packages].map(([device, keyPackage]) => ({
          device: Uint8Array.from(Buffer.from(device, "hex")),
          keyPackage,
        })),
    } as unknown as ControlPlane;
  }
}

test.skipIf(!built)("two browsers share their room key through the group", async () => {
  const m = (await import(glue.href)) as typeof import("drywire-mls-wasm");
  await m.default({ module_or_path: readFileSync(new URL("./mls-wasm/drywire_mls_wasm_bg.wasm", import.meta.url)) });
  const { Groups, useModule } = await import("./group");
  useModule(m);
  // An identity with its seed, as signing in by e-mail leaves it; its
  // public key is the one the device signs with.
  const identity = (seed: number): Identity => {
    const s = new Uint8Array(32).fill(seed);
    return { seed: s, publicKey: new m.WebDevice(s).publicKey() } as unknown as Identity;
  };
  const service = new Service();
  const control = service.control();
  const [alexId, samId] = [identity(1), identity(2)];
  service.members.add(hex(alexId.publicKey));
  service.members.add(hex(samId.publicKey));
  localStorage.clear();

  // Each browser keeps its own state: two Groups, one storage key, so
  // they run one after the other with the storage cleared in between.
  const alex = new Groups(alexId);
  expect(await alex.sync(control, SPACE, true)).toEqual({ ready: true, epoch: 1 });
  localStorage.clear();
  const sam = new Groups(samId);
  expect(await sam.sync(control, SPACE, false)).toEqual({ ready: false });
  expect(await alex.sync(control, SPACE, true)).toEqual({ ready: true, epoch: 2 });
  expect(await sam.sync(control, SPACE, false)).toEqual({ ready: true, epoch: 2 });
  const key = await alex.roomKey(SPACE, ROOM);
  expect(key).not.toBeNull();
  expect(await sam.roomKey(SPACE, ROOM)).toEqual(key);

  // Sam leaves the space: Alex's next round takes their device out.
  service.members.delete(hex(samId.publicKey));
  expect(await alex.sync(control, SPACE, true)).toEqual({ ready: true, epoch: 3 });
  expect(await alex.roomKey(SPACE, ROOM)).not.toEqual(key);
});
