// SPDX-License-Identifier: GPL-3.0-or-later

// Output sinks (recording.md section 5.8, REC-63 to REC-65):
//
// - FileSink: a File System Access writable from `showSaveFilePicker`,
//   chosen in the click handler (REC-15). Writes are queued, not awaited per
//   frame; the export loop awaits `drain` only when the queued bytes pass a
//   threshold. At the end the muxer's patches are written with positioned
//   writes (seekable sink), then `close()`.
// - MemorySink: the parts kept in memory and joined into one Blob at the
//   end, capped at BLOB_CAP (REC-63). The `faststart-deferred` layout's
//   `moov` is spliced in after `ftyp` (Placement InsertAt).
//
// Failures are classified by DOMException name, never by message
// (REC-65): QuotaExceededError is `record_disk_full`, anything else
// `record_sink_failed`.

export interface SinkError {
  code: 'record_disk_full' | 'record_sink_failed';
  name: string;
  text: string;
}

export interface Placement {
  kind: 'append' | 'insert_at';
  offset?: number;
  len?: number;
}

export interface SinkResult {
  bytes: number;
  /** Memory sink: the finished file. */
  blob?: Blob;
}

export interface Sink {
  readonly kind: 'file' | 'memory';
  /** File name (as picked, or suggested). */
  readonly name: string;
  readonly seekable: boolean;
  /** Bytes handed to `write` so far (the size of the file so far). */
  readonly written: number;
  /** Queued bytes not yet written. */
  readonly pending: number;
  /** The memory cap was reached; the recording must stop and save. */
  readonly full: boolean;
  /** Largest file this sink takes (the in-memory cap), or Infinity. */
  readonly limit: number;
  readonly error: SinkError | null;
  /** Queues a part; the sink owns it afterwards. */
  write(part: Uint8Array): void;
  /** Resolves once at most `limit` bytes are queued; rejects with the sink's error. */
  drain(limit: number): Promise<void>;
  /** Writes the tail, applies the patches and closes. */
  finish(tail: Uint8Array, placement: Placement, patches: Uint8Array): Promise<SinkResult>;
  /** Discard: abort the writable (the picked file stays as created) or drop the parts. */
  abort(): Promise<void>;
  /** After an error: try to close once so what was written is kept (REC-65). */
  tryClose(): Promise<boolean>;
}

export function sinkError(e: unknown): SinkError {
  const name = e instanceof DOMException || (e instanceof Error && e.name) ? (e as Error).name : 'Error';
  const text = e instanceof Error ? e.message : String(e);
  return {
    code: name === 'QuotaExceededError' ? 'record_disk_full' : 'record_sink_failed',
    name,
    text,
  };
}

/** `take_patches()` wire form: repeated (offset u64 LE, length u32 LE, bytes). */
export function decodePatches(wire: Uint8Array): Array<{ offset: number; bytes: Uint8Array }> {
  const out: Array<{ offset: number; bytes: Uint8Array }> = [];
  const view = new DataView(wire.buffer, wire.byteOffset, wire.byteLength);
  let p = 0;
  while (p + 12 <= wire.byteLength) {
    const offset = view.getUint32(p, true) + view.getUint32(p + 4, true) * 2 ** 32;
    const len = view.getUint32(p + 8, true);
    p += 12;
    out.push({ offset, bytes: wire.subarray(p, p + len) });
    p += len;
  }
  return out;
}

/** Minimal shape of `FileSystemWritableFileStream` (not in TypeScript's DOM lib everywhere). */
export interface WritableLike {
  write(data: Uint8Array | { type: 'write'; position: number; data: Uint8Array }): Promise<void>;
  close(): Promise<void>;
  abort(): Promise<void>;
}

export interface FileHandleLike {
  readonly name: string;
  createWritable(): Promise<WritableLike>;
}

export class FileSink implements Sink {
  readonly kind = 'file';
  readonly seekable = true;
  readonly full = false;
  readonly limit = Infinity;
  written = 0;
  pending = 0;
  error: SinkError | null = null;
  private chain: Promise<void> = Promise.resolve();
  private waiter: (() => void) | null = null;
  private closed = false;

  private constructor(
    readonly name: string,
    private readonly writable: WritableLike,
  ) {}

  static async open(handle: FileHandleLike): Promise<FileSink> {
    const writable = await handle.createWritable();
    return new FileSink(handle.name, writable);
  }

  write(part: Uint8Array): void {
    if (this.error || part.byteLength === 0) return;
    const n = part.byteLength;
    this.written += n;
    this.pending += n;
    this.chain = this.chain.then(async () => {
      if (this.error) return;
      try {
        await this.writable.write(part);
      } catch (e) {
        this.error = sinkError(e);
      } finally {
        this.pending -= n;
        this.wake();
      }
    });
  }

  private wake(): void {
    const w = this.waiter;
    this.waiter = null;
    w?.();
  }

  async drain(limit: number): Promise<void> {
    while (this.pending > limit && !this.error) {
      await new Promise<void>((resolve) => (this.waiter = resolve));
    }
    if (this.error) throw this.error;
  }

  async finish(tail: Uint8Array, _placement: Placement, patches: Uint8Array): Promise<SinkResult> {
    this.write(tail);
    await this.chain;
    if (this.error) throw this.error;
    try {
      for (const p of decodePatches(patches)) {
        await this.writable.write({ type: 'write', position: p.offset, data: p.bytes });
      }
      this.closed = true;
      await this.writable.close();
    } catch (e) {
      this.error = sinkError(e);
      throw this.error;
    }
    return { bytes: this.written };
  }

  async abort(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.error ??= { code: 'record_sink_failed', name: 'AbortError', text: 'discarded' };
    this.wake();
    try {
      await this.writable.abort();
    } catch {
      // Nothing more can be done with it.
    }
  }

  async tryClose(): Promise<boolean> {
    if (this.closed) return false;
    this.closed = true;
    try {
      await this.chain;
      await this.writable.close();
      return true;
    } catch {
      return false;
    }
  }
}

export class MemorySink implements Sink {
  readonly kind = 'memory';
  readonly seekable = false;
  readonly pending = 0;
  written = 0;
  error: SinkError | null = null;
  private parts: Uint8Array[] = [];

  constructor(
    readonly name: string,
    readonly limit: number,
  ) {}

  get full(): boolean {
    return this.written >= this.limit;
  }

  write(part: Uint8Array): void {
    if (part.byteLength === 0) return;
    this.parts.push(part);
    this.written += part.byteLength;
  }

  drain(): Promise<void> {
    return Promise.resolve();
  }

  async finish(tail: Uint8Array, placement: Placement, patches: Uint8Array): Promise<SinkResult> {
    for (const p of decodePatches(patches)) this.patch(p.offset, p.bytes);
    if (placement.kind === 'insert_at' && placement.offset !== undefined && placement.len !== undefined) {
      this.insert(placement.offset, tail.subarray(0, placement.len));
      this.write(tail.subarray(placement.len));
    } else {
      this.write(tail);
    }
    const blob = new Blob(this.parts as BlobPart[], { type: 'video/mp4' });
    this.parts = [];
    return { bytes: blob.size, blob };
  }

  /** Overwrites bytes at `offset` across the parts (the parts are ours). */
  private patch(offset: number, bytes: Uint8Array): void {
    let start = 0;
    let done = 0;
    for (const part of this.parts) {
      const end = start + part.byteLength;
      while (done < bytes.byteLength && offset + done >= start && offset + done < end) {
        part[offset + done - start] = bytes[done]!;
        done++;
      }
      if (done === bytes.byteLength) return;
      start = end;
    }
  }

  /** Inserts `bytes` at `offset`, splitting the part it falls in. */
  private insert(offset: number, bytes: Uint8Array): void {
    let start = 0;
    for (let i = 0; i <= this.parts.length; i++) {
      if (start === offset) {
        this.parts.splice(i, 0, bytes);
        this.written += bytes.byteLength;
        return;
      }
      const part = this.parts[i];
      if (!part) break;
      const end = start + part.byteLength;
      if (offset > start && offset < end) {
        const cut = offset - start;
        this.parts.splice(i, 1, part.subarray(0, cut), bytes, part.subarray(cut));
        this.written += bytes.byteLength;
        return;
      }
      start = end;
    }
    this.write(bytes);
  }

  async abort(): Promise<void> {
    this.parts = [];
  }

  async tryClose(): Promise<boolean> {
    return false;
  }
}
