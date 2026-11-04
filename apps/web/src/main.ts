// A first web client, to prove the path end to end (ADR 0038 to 0040):
// it joins a room with an invitation link, binds its session over
// WebTransport, sends a second of a 440 Hz tone as encrypted Opus frames,
// and decodes what comes back from the other members, typically the
// command-line client's echo bot (`make web`). It is a developer's page:
// the real interface reuses the desktop app's components, drawn on the
// design canvas first.

import { decoder, encoder, SAMPLE_RATE, supported } from "./codec";
import { ControlPlane, type JoinAnswer } from "./control";
import { loadIdentity, sign } from "./identity";
import { HELLO_ACK, PONG, Protocol, unhex } from "./protocol";
import { RelayLink } from "./relay";

const FRAME_MS = 10;
const FRAME = (SAMPLE_RATE * FRAME_MS) / 1000;
const TONE_HZ = 440;
const TONE_FRAMES = 100; // one second

export type Result = {
  frameMs: number;
  maxDatagramSize: number;
  pingMs: number[];
  sent: number;
  received: number;
  decodedSamples: number;
  toneHz: number;
  firstEchoMs: number | null;
  dropped: Record<string, number>;
};

const log = (line: string) => {
  const out = document.querySelector("#log")!;
  out.textContent += `${line}\n`;
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The frequency of a signal, from how often it crosses zero. */
function frequency(pcm: Float32Array): number {
  let crossings = 0;
  for (let i = 1; i < pcm.length; i++) if (pcm[i - 1] < 0 !== pcm[i] < 0) crossings++;
  return (crossings / 2) * (SAMPLE_RATE / pcm.length);
}

async function run(link: string, name: string): Promise<Result> {
  const settings = { frameMs: FRAME_MS, bitrate: 64_000 } as const;
  if (!(await supported(settings))) throw new Error("this browser cannot encode Opus with WebCodecs");

  const protocol = await Protocol.load(await (await fetch("/drywire.wasm")).arrayBuffer());
  const invitation = protocol.invitation(link);
  const identity = await loadIdentity();
  log(`instance ${invitation.instance}`);

  // Through Vite's proxy in development: see control.ts.
  const control = new ControlPlane("", invitation.instance, identity, protocol);
  await control.login();
  const view = await control.redeem(invitation.code, name);
  const answer: JoinAnswer = await control.join(view.rooms[0].id);
  if (!answer.web_relay) throw new Error("this instance serves no browser (DRYWIRE_WEB_ADDR)");
  log(`joined ${view.space.name} / ${answer.room.name} as session ${answer.session}`);

  protocol.start({
    session: answer.session,
    secret: unhex(answer.session_secret),
    spaceKey: invitation.spaceKey,
    roomId: unhex(answer.room.id),
    publicKey: identity.publicKey,
    salt: crypto.getRandomValues(new Uint8Array(32)),
  });
  for (const s of answer.sessions) {
    if (s.session !== answer.session) protocol.addPeer(s.session, unhex(s.public_key));
  }

  const relay = await RelayLink.open(answer.web_relay.url, answer.web_relay.certificate_hashes);
  log(`WebTransport open, datagrams up to ${relay.maxDatagramSize} bytes`);

  // Everything the relay sends, as it comes.
  const result: Result = {
    frameMs: FRAME_MS,
    maxDatagramSize: relay.maxDatagramSize,
    pingMs: [],
    sent: 0,
    received: 0,
    decodedSamples: 0,
    toneHz: 0,
    firstEchoMs: null,
    dropped: {},
  };
  let helloCounter = 0n;
  let bound = false;
  let firstSent = 0;
  const heard: Float32Array[] = [];
  const opus = decoder((pcm) => heard.push(pcm));
  (async () => {
    for (;;) {
      const { value, done } = await relay.reader.read();
      if (done) return;
      const r = protocol.receive(value);
      if (r.kind === "audio") {
        if (result.received++ === 0) result.firstEchoMs = performance.now() - firstSent;
        opus.decode(
          new EncodedAudioChunk({
            type: "key",
            timestamp: (r.timestamp * 1e6) / SAMPLE_RATE,
            data: r.frame,
          }),
        );
      } else if (r.kind === "control" && r.type === HELLO_ACK) {
        bound ||= protocol.isHelloAck(value, helloCounter);
      } else if (r.kind === "control" && r.type === PONG) {
        const sentAt = new DataView(value.buffer, value.byteOffset, value.byteLength).getBigUint64(16);
        result.pingMs.push(performance.now() - Number(sentAt) / 1000);
      } else if (r.kind === "dropped") {
        result.dropped[r.reason] = (result.dropped[r.reason] ?? 0) + 1;
      }
    }
  })();

  // Bind the session: a new counter for every attempt (protocol 6.1).
  for (let attempt = 0; !bound; attempt++) {
    if (attempt === 10) throw new Error("the relay never answered the hello");
    helloCounter = BigInt(Date.now());
    relay.send(protocol.hello(helloCounter));
    await sleep(300);
  }
  log("session bound to the WebTransport session");

  for (let i = 0; i < 5; i++) {
    relay.send(protocol.ping(i, BigInt(Math.round(performance.now() * 1000))));
    await sleep(100);
  }

  // The announcement is signed once and sent again unchanged (ADR 0040).
  const announce = protocol.announce(await sign(identity, protocol.announcementMessage()));
  relay.send(announce);

  // A second of tone, encoded as it would be captured, paced in real time.
  const frames: Uint8Array[] = [];
  const opusOut = encoder(settings, (f) => frames.push(f));
  for (let i = 0; i < TONE_FRAMES; i++) {
    const pcm = new Float32Array(FRAME);
    for (let n = 0; n < FRAME; n++) {
      pcm[n] = 0.3 * Math.sin((2 * Math.PI * TONE_HZ * (i * FRAME + n)) / SAMPLE_RATE);
    }
    opusOut.encode(
      new AudioData({
        format: "f32-planar",
        sampleRate: SAMPLE_RATE,
        numberOfFrames: FRAME,
        numberOfChannels: 1,
        timestamp: (i * FRAME_MS * 1000) | 0,
        data: pcm,
      }),
    );
  }
  await opusOut.flush();
  const start = performance.now();
  for (let i = 0; i < frames.length; i++) {
    if (i % 100 === 0) relay.send(announce); // once a second, and first
    if (i === 0) firstSent = performance.now();
    relay.send(protocol.seal(frames[i], i * FRAME, i === frames.length - 1));
    result.sent++;
    const due = start + (i + 1) * FRAME_MS;
    await sleep(Math.max(0, due - performance.now()));
  }

  // Wait for the echo to finish, then look at what came back.
  await sleep(1500);
  await opus.flush().catch(() => {});
  const all = new Float32Array(heard.reduce((n, p) => n + p.length, 0));
  heard.reduce((at, p) => (all.set(p, at), at + p.length), 0);
  result.decodedSamples = all.length;
  // The tone's frequency, between the first and the last loud sample: the
  // silence around it would lower the count.
  const isLoud = (s: number) => Math.abs(s) > 0.01;
  const first = all.findIndex(isLoud);
  const last = all.findLastIndex(isLoud);
  result.toneHz = first >= 0 ? Math.round(frequency(all.subarray(first, last + 1))) : 0;
  relay.close();
  await control.leave().catch(() => {});

  // Play what came back, so a person at the page hears it too.
  if (all.length) {
    const context = new AudioContext({ sampleRate: SAMPLE_RATE });
    const buffer = context.createBuffer(1, all.length, SAMPLE_RATE);
    buffer.copyToChannel(all, 0);
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    source.start();
  }
  return result;
}

const form = document.querySelector("form")!;
const linkInput = document.querySelector<HTMLInputElement>("#link")!;
// `make web` opens the page with the invitation in the fragment, which
// never leaves the browser.
const fromFragment = () => {
  if (location.hash.length > 1) linkInput.value = `https://drywire.gg/i${location.hash}`;
};
fromFragment();
addEventListener("hashchange", fromFragment);
form.addEventListener("submit", (e) => {
  e.preventDefault();
  const name = document.querySelector<HTMLInputElement>("#name")!.value;
  run(linkInput.value, name).then(
    (result) => {
      log(JSON.stringify(result, null, 2));
      (window as unknown as { drywireResult: Result }).drywireResult = result;
    },
    (err) => log(`failed: ${err.message ?? err}`),
  );
});
