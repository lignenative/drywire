// The module drywire-mls-wasm becomes once wasm-bindgen made its glue
// (scripts/mls-wasm.sh writes it to src/mls-wasm/, which git ignores):
// what the page uses of it.
declare module "drywire-mls-wasm" {
  export default function init(options?: { module_or_path: BufferSource }): Promise<unknown>;
  export class WebDevice {
    constructor(seed: Uint8Array<ArrayBuffer>);
    static load(state: Uint8Array<ArrayBuffer>): WebDevice;
    save(): Uint8Array<ArrayBuffer>;
    publicKey(): Uint8Array<ArrayBuffer>;
    keyPackage(): Uint8Array<ArrayBuffer>;
    free(): void;
  }
  export class StagedCommit {
    readonly commit: Uint8Array<ArrayBuffer>;
    readonly welcome: Uint8Array<ArrayBuffer>;
    readonly welcomed: Uint8Array<ArrayBuffer>;
    free(): void;
  }
  export class WebGroup {
    static create(device: WebDevice, space: Uint8Array<ArrayBuffer>): WebGroup;
    static load(device: WebDevice, space: Uint8Array<ArrayBuffer>): WebGroup | undefined;
    static join(device: WebDevice, welcome: Uint8Array<ArrayBuffer>, members: Uint8Array<ArrayBuffer>): WebGroup;
    epoch(): number;
    isActive(): boolean;
    apply(device: WebDevice, commit: Uint8Array<ArrayBuffer>, members: Uint8Array<ArrayBuffer>): void;
    prune(device: WebDevice, members: Uint8Array<ArrayBuffer>): StagedCommit | undefined;
    add(device: WebDevice, devices: Uint8Array<ArrayBuffer>, packages: Uint8Array<ArrayBuffer>, lengths: Uint32Array<ArrayBuffer>, members: Uint8Array<ArrayBuffer>): StagedCommit;
    renew(device: WebDevice): StagedCommit;
    confirm(device: WebDevice): void;
    abandon(device: WebDevice): void;
    forget(device: WebDevice): void;
    roomKey(device: WebDevice, room: Uint8Array<ArrayBuffer>): Uint8Array<ArrayBuffer>;
    free(): void;
  }
}
