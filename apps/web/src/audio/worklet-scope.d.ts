// The AudioWorkletGlobalScope, which TypeScript's DOM library leaves out:
// only what capture.worklet.ts and playback.worklet.ts use. These names
// exist in the worklets only, never on the page's main thread.

declare const sampleRate: number;
declare const currentFrame: number;

declare class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}

interface AudioWorkletProcessorImpl extends AudioWorkletProcessor {
  process(inputs: Float32Array[][], outputs: Float32Array[][], parameters: Record<string, Float32Array>): boolean;
}

declare function registerProcessor(
  name: string,
  processor: new (options: AudioWorkletNodeOptions) => AudioWorkletProcessorImpl,
): void;
