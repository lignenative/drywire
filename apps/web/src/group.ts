// The spaces' MLS groups in this browser (ADRs 0052 and 0055), as
// drywire-control's SpaceGroup does natively: each round catches up with
// the commits, lets in the waiting devices of members, and takes out the
// devices no longer a member's; a browser not in the group joins from its
// welcome, or asks to join; the owner's device makes the group. OpenMLS
// runs in WebAssembly, loaded the first time a group is needed.

import type { StagedCommit, WebDevice, WebGroup } from "drywire-mls-wasm";
import { type ControlPlane, StaleEpoch } from "./control";
import type { Identity } from "./identity";
import { hex, unhex } from "./protocol";

const STATE = "drywire.web.groups";

type Module = typeof import("drywire-mls-wasm");
let loading: Promise<Module> | null = null;

/** Hands in the module already loaded, as the tests do (Bun does not
    know Vite's alias). */
export function useModule(m: Module): void {
  loading = Promise.resolve(m);
}

/** The OpenMLS module, loaded once. */
function load(): Promise<Module> {
  loading ??= import("drywire-mls-wasm").then(async (m) => {
    await m.default();
    return m;
  });
  return loading;
}

function concat(parts: Uint8Array[]): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

export type GroupState = { ready: true; epoch: number } | { ready: false };

export class Groups {
  private device: WebDevice | null = null;
  private groups = new Map<string, WebGroup>();
  private asked = new Set<string>();
  /** One round at a time: two would stage commits over each other. */
  private queue: Promise<unknown> = Promise.resolve();

  constructor(private readonly identity: Identity) {}

  private async open(): Promise<{ m: Module; device: WebDevice }> {
    const m = await load();
    if (!this.device) {
      const saved = (() => {
        try {
          return localStorage.getItem(STATE);
        } catch {
          return null;
        }
      })();
      if (saved) this.device = m.WebDevice.load(unhex(saved));
      else if (this.identity.seed) this.device = new m.WebDevice(this.identity.seed);
      else throw Object.assign(new Error("this browser's key is from before accounts"), { code: "old_key" });
    }
    return { m, device: this.device };
  }

  private save(): void {
    try {
      if (this.device) localStorage.setItem(STATE, hex(this.device.save()));
    } catch {
      // Full or blocked storage: the next start joins the groups again.
    }
  }

  /** One round with the delivery service for a space. */
  sync(control: ControlPlane, spaceId: string, owner: boolean): Promise<GroupState> {
    const round = this.queue.catch(() => {}).then(() => this.round(control, spaceId, owner));
    this.queue = round;
    return round;
  }

  private async round(control: ControlPlane, spaceId: string, owner: boolean): Promise<GroupState> {
    const { m, device } = await this.open();
    try {
      const space = await control.space(spaceId);
      const memberKeys = (space.members ?? []).flatMap((x) => x.devices ?? []);
      const members = concat(memberKeys.map(unhex));
      const isMember = (d: Uint8Array) => memberKeys.includes(hex(d));
      const id = unhex(spaceId);
      let group = this.groups.get(spaceId) ?? m.WebGroup.load(device, id) ?? null;
      const me = hex(this.identity.publicKey);

      if (!group) {
        const welcome = await control.welcome(spaceId, me);
        if (welcome) {
          try {
            group = m.WebGroup.join(device, welcome, members);
          } catch {
            group = null;
          }
        }
        if (!group && owner && (await control.commits(spaceId, 0)).length === 0) {
          const made = m.WebGroup.create(device, id);
          const first = made.renew(device);
          try {
            await control.postCommit(spaceId, 0, first.commit, new Uint8Array(), new Uint8Array());
            made.confirm(device);
            group = made;
          } catch (e) {
            made.forget(device);
            if (!(e instanceof StaleEpoch)) throw e;
          }
        }
        if (!group) {
          if (!this.asked.has(spaceId)) {
            await control.postKeyPackage(spaceId, me, device.keyPackage());
            this.asked.add(spaceId);
          }
          return { ready: false };
        }
        this.groups.set(spaceId, group);
      }

      for (;;) {
        // Catch up, 64 commits at a time.
        for (;;) {
          const commits = await control.commits(spaceId, group.epoch());
          if (commits.length === 0) break;
          for (const c of commits) {
            try {
              group.apply(device, c.commit, members);
            } catch {
              // Removed, or a state gone wrong: ask again as a new leaf.
              return this.restart(control, spaceId, owner, group, device);
            }
          }
        }
        if (!group.isActive()) return this.restart(control, spaceId, owner, group, device);
        let staged: StagedCommit | undefined = group.prune(device, members);
        if (!staged) {
          const waiting = (await control.keyPackages(spaceId)).filter((p) => isMember(p.device));
          if (waiting.length > 0) {
            try {
              staged = group.add(
                device,
                concat(waiting.map((p) => p.device)),
                concat(waiting.map((p) => p.keyPackage)),
                Uint32Array.from(waiting.map((p) => p.keyPackage.length)),
                members,
              );
            } catch {
              staged = undefined;
            }
          }
        }
        if (!staged) return { ready: true, epoch: group.epoch() };
        try {
          await control.postCommit(spaceId, group.epoch(), staged.commit, staged.welcome, staged.welcomed);
          group.confirm(device);
        } catch (e) {
          group.abandon(device);
          if (!(e instanceof StaleEpoch)) throw e;
        }
      }
    } finally {
      this.save();
    }
  }

  private restart(control: ControlPlane, spaceId: string, owner: boolean, group: WebGroup, device: WebDevice) {
    this.groups.delete(spaceId);
    this.asked.delete(spaceId);
    group.forget(device);
    return this.round(control, spaceId, owner);
  }

  /** The key of a room in the space's current epoch, once in the group. */
  async roomKey(spaceId: string, roomId: string): Promise<Uint8Array<ArrayBuffer> | null> {
    const group = this.groups.get(spaceId);
    if (!group) return null;
    const { device } = await this.open();
    return group.roomKey(device, unhex(roomId));
  }
}
