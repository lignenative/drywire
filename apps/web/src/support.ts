// What the web client needs from the browser, and what this one lacks.
// The names are shown as they are, in both languages: they are the
// standards to look for.
import { bestSettings, type EncoderSettings } from "./codec";

export interface Support {
  missing: string[];
  /** The Opus frames this browser encodes, when it can. */
  codec: EncoderSettings | null;
}

async function ed25519(): Promise<boolean> {
  try {
    await crypto.subtle.generateKey({ name: "Ed25519" }, false, ["sign", "verify"]);
    return true;
  } catch {
    return false;
  }
}

export async function checkSupport(): Promise<Support> {
  const missing: string[] = [];
  if (typeof WebTransport === "undefined") missing.push("WebTransport");
  const codec = await bestSettings();
  if (!codec) missing.push("Opus (WebCodecs)");
  if (typeof AudioWorkletNode === "undefined") missing.push("AudioWorklet");
  if (typeof WebAssembly === "undefined") missing.push("WebAssembly");
  if (!crypto?.subtle || !(await ed25519())) missing.push("Ed25519 (WebCrypto)");
  if (!navigator.mediaDevices?.getUserMedia) missing.push("getUserMedia");
  return { missing, codec };
}
