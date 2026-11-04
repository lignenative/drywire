// The relay, from a browser: a WebTransport session whose datagrams carry
// the same packets a native client sends over UDP (ADR 0038), to a
// certificate pinned by the hashes the join answer gave (ADR 0039).

import { unhex } from "./protocol";

export class RelayLink {
  private constructor(
    private readonly transport: WebTransport,
    private readonly writer: WritableStreamDefaultWriter<Uint8Array>,
    readonly reader: ReadableStreamDefaultReader<Uint8Array>,
  ) {}

  static async open(url: string, certificateHashes: string[]): Promise<RelayLink> {
    const transport = new WebTransport(url, {
      // Lowercase: some engines compare the name literally (ADR 0039).
      serverCertificateHashes: certificateHashes.map((h) => ({
        algorithm: "sha-256",
        value: unhex(h),
      })),
    });
    await transport.ready;
    // Chrome only has `writable`, Safari only `createWritable()`, Firefox
    // both: the standard way first.
    const datagrams = transport.datagrams as WebTransportDatagramDuplexStream & {
      createWritable?: () => WritableStream<Uint8Array>;
    };
    const writable = datagrams.createWritable?.() ?? datagrams.writable;
    return new RelayLink(transport, writable.getWriter(), datagrams.readable.getReader());
  }

  /** The largest packet a datagram can carry on this connection. */
  get maxDatagramSize(): number {
    return this.transport.datagrams.maxDatagramSize;
  }

  /** Sends one packet. Like UDP, it may be lost; it is never retried. */
  send(packet: Uint8Array): void {
    // Not awaited: a datagram that cannot go out at once is dropped by
    // the browser, which is what voice wants.
    this.writer.write(packet).catch(() => {});
  }

  close(): void {
    this.transport.close();
  }
}
