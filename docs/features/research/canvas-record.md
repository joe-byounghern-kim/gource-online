<!-- Research notes gathered on 2026-10-02 for F3 recording (docs/features/recording.md). -->

## canvas-record offline MP4 techniques (dmnsgn/canvas-record, HEAD 63a4b2b, 2026-09-17; v6.0.0)

Source: https://github.com/dmnsgn/canvas-record (src/Recorder.js, src/encoders/WebCodecsEncoder.js, src/utils.js). Raw: https://raw.githubusercontent.com/dmnsgn/canvas-record/main/src/... . The muxer is **mediabunny** (https://github.com/Vanilagy/mediabunny, src/media-source.ts), not mp4-muxer. No frame loop, worker, OffscreenCanvas, or "realtime" mode exists in the library. The only realtime path is `MediaCaptureEncoder` (MediaStream).

### 1. Per-frame pipeline
- `Recorder.step()` (Recorder.js) encodes `await encoder.encode(await getFrame(encoder.frameMethod), frame)`, then does `time += 1/frameRate; frame++`. When `frame >= duration*frameRate` it calls `stop()`.
- The WebCodecs encoder's `frameMethod` is `"videoFrame"`. The frame is built straight from the canvas element with no readback:
```js
new VideoFrame(canvas, { timestamp: this.time * 1_000_000, duration: 1_000_000 / this.frameRate, visibleRect, alpha: encoder.alpha, ...frameOptions })
```
  - Timestamps are synthetic (frame index / fps in µs), so output speed is independent of wall clock.
  - Other methods exist but are slower: `bitmap` (`createImageBitmap`), `imageData` (`readPixels` or `getImageData`, CPU vertical flip, dims rounded up with `nextMultiple(x,2)`), and a default blob screenshot.
  - `visibleRect` does not crop WebGL canvases, so cropping a WebGL canvas goes through an intermediate 2D canvas (`captureCanvasRegion`).
- Keyframes and queueing: v5.5.1 (older, explicit WebCodecs code) used `keyFrame = number % groupOfPictures === 0` with the default GOP of 20 frames. HEAD instead passes `keyFrameInterval: groupOfPictures / frameRate` (seconds) to mediabunny.
- v5.5.1 backpressure was crude: `encoder.encode(frame,{keyFrame}); frame.close(); if ((n+1) % flushFrequency(10) === 0) await encoder.flush();`.
- HEAD (WebCodecsEncoder.js `encode`): `const sample = new VideoSample(frame); await this.videoSource.add(sample); sample.close();`. Backpressure now lives in mediabunny's `VideoSampleSource.add` (media-source.ts ~L633):
```ts
this.encoder.encode(videoFrame, opts); videoFrame.close();
if (this.encoder.encodeQueueSize >= 4) await new Promise(r => this.encoder.addEventListener('dequeue', r, {once:true}));
await this.lastMuxerPromise; // writer backpressure
```
  So the encoder queue depth is capped at 4, and the muxer or writer promise is also awaited.

### 2. Encoder config
- Codec default for mp4/mov is `AVC.getCodec({profile:"High", level:"5.2"})`, which gives `avc1.640034`. It is fixed at level 5.2, not chosen from the frame size. Users can override it with `encoderOptions.codec`. For webm/mkv the default is VP9 `vp09.00.10.08`. README note: AVC max frame size is 9,437,184 px, so 4K square or 8K needs another route.
- Bitrate comes from `estimateBitRate(w,h,fps,motionRank=4,mode)`:
  - Formula: `w*h*fps*motionRank*0.07*(variable ? 0.75 : 1)`.
  - Example: 1080p30 gives about 13 Mbps.
- Everything else is left at browser defaults: `bitrateMode`, `latencyMode`, `hardwareAcceleration` and the avc `format` appear only as commented hints (`latencyMode "realtime"` is noted as "faster encoding"). The AVC bitstream format is not set; mediabunny handles `avcC`.
- v5.5.1 called `VideoEncoder.isConfigSupported(config)` and threw if unsupported.
- mediabunny forces `latencyMode:'quality'` only for alpha. It also rejects or handles odd dimensions for avc/hevc (media-source.ts ~L712).

### 3. Muxing
- Muxer: mediabunny `Output` with `Mp4OutputFormat({ fastStart: writableFileStream ? false : "in-memory" })`. The target is `BufferTarget` (in-memory) or `StreamTarget(FileSystemWritableFileStream)` when using the File System Access API.
- Flow: `muxer.addVideoTrack(videoSource,{frameRate})`, then `await muxer.start()` on frame 0, then `await muxer.finalize()` and return `target.buffer`.
- `decoderConfig.description` (the avcC) never touches user code. In v5.5.1, `output: (chunk, meta) => videoSource.add(EncodedPacket.fromEncodedChunk(chunk), meta)` forwarded `meta.decoderConfig` to the muxer.
- Download (utils.js `downloadBlob`): `new Blob(parts,{type})`, `URL.createObjectURL`, an `<a download>` click, then revoke after 1 ms.
- Fragmented MP4 is not used by canvas-record. mediabunny also offers `fastStart:'fragmented'` (it is a mediabunny option, not shown in canvas-record). That mode needs no moov rewrite and can stream to disk.

### 4. Frame-step model
- The user calls `await recorder.step()` once per frame after drawing frame N (README, `example/index.js` ~L228-240). The recorder owns the time (`recorder.time`, `deltaTime`, `frame`, `frameTotal`), and the app renders deterministically from it.
- The example still uses `requestAnimationFrame(() => tick())` between steps. That caps throughput at the display refresh rate. The stats panel prints "Speedup: x(time/renderTime)".
- The library does not try to avoid rAF. Looping `render(); await step()` directly without rAF is the obvious offline speed-up, and it is what our implementation should do. Awaiting `step()` already yields to the event loop.

### 5. Speed claims and pitfalls
- README: "`WebCodecs` 5-10x faster than H264MP4Encoder (WASM x264) and 20x faster than FFmpeg (PNG to virtual FS then mux)".
- Other claims in the README:
  - The AVC 9,437,184-px limit.
  - `FFmpeg` needs SharedArrayBuffer.
  - `latencyMode: "realtime"` is hinted as faster.
- The repo does not mention `preserveDrawingBuffer`. `new VideoFrame(canvas)` is called right after drawing in the same task, so the drawing buffer is still valid and no readback is needed. If you await across a task boundary before constructing the frame, a WebGL canvas without `preserveDrawingBuffer` may be cleared.
- The repo has no Safari or Firefox notes, no colorSpace handling, and no worker or OffscreenCanvas use. Odd sizes are handled only on the `imageData` path (`nextMultiple`) and in mediabunny.
- `alpha:"keep"` works only with webm/mkv.
- In-memory output carries the usual ArrayBuffer memory limit, and the repo notes a probable ~4 GB limit for the WASM path.

### 6. Recommendations for our Rust/WASM WebGL2 fixed-dt 60 fps exporter
1. Drive frames with a plain async loop (`sim.step(1/60); render(); await encode(...)`), not rAF. Use timestamps `round(i*1e6/60)` µs and `duration = 16667`. Prefer integer-µs accumulation to avoid drift (1e6/60 is fractional; use `round(i*1_000_000/60)`).
2. Make the canvas size even (ideally a multiple of 16 to avoid padding cost) and within the H.264 level limits.
3. Create the `VideoFrame` directly from the WebGL canvas right after the draw call in the same task. This is the zero-readback path. Avoid `readPixels`, `getImageData` and `createImageBitmap`. Set `preserveDrawingBuffer:true` only if the frame is built after an await. Always call `frame.close()` immediately after `encoder.encode`.
4. Use a `VideoEncoder` directly, with no JS muxer library. Pick the codec by level from the frame size:
   - L4.0 `avc1.640028` up to 1080p30.
   - L4.2 `avc1.64002A` for 1080p60.
   - L5.1 or L5.2 `avc1.640033` or `avc1.640034` for 4K.
   - Verify with `isConfigSupported`, with a fallback list (High, then Main, then Baseline).
5. Set `bitrateMode:"variable"`, `hardwareAcceleration:"prefer-hardware"`, and `latencyMode:"realtime"` (try it, with `quality` as the fallback). Set `avc:{format:"avc"}` so chunks are AVCC and `decoderConfig.description` is present for MP4. Use a bitrate near `w*h*fps*0.07*0.75*motionRank`, or a fixed 8-20 Mbps.
6. Keyframe every 60-120 frames, via `encode(frame,{keyFrame:i%N===0})`.
7. Apply backpressure the way mediabunny does: after each `encode()`, if `encoder.encodeQueueSize >= 4` await one `dequeue` event. Do not flush every N frames. Do one `flush()` at the end. Also cap or await the muxer or writer.
8. Hand-written MP4 muxer in Rust:
   - Capture the avcC from the first chunk's `meta.decoderConfig.description`. Parse SPS for width, height, profile and level.
   - Append each chunk to `mdat` as an AVCC sample, recording per-sample size, timestamp and sync flag.
   - Output either fast-start (write `ftyp`+`moov`+`mdat` at the end, with `stts`, `stsz`, `stss`, `stsc`, `stco`/`co64` in one chunk run, and a single final buffer copy, as in mediabunny's `in-memory` mode) or fragmented `moof`/`mdat` per GOP for streaming and bounded memory.
   - Mind B-frames: use `ctts` or disable them. WebCodecs H.264 normally outputs no reordering in low-latency mode, so `dts == pts`.
   - Use a 90 kHz or 600-per-second timescale so 1/60 is exact (for example timescale 60000 with delta 1000).
9. Offer a File System Access `FileSystemWritableFileStream` target for large outputs, with fragmented MP4 or a seekable write of the moov at the end. Fall back to a Blob download otherwise.
10. For maximum throughput, run the whole export on an OffscreenCanvas in a Worker (VideoFrame, VideoEncoder and WebGL2 are all available there). This keeps the UI free. canvas-record does not do this, so it is our own improvement.
11. Add a feature check for `VideoEncoder`. Document the Firefox and Safari gaps (older Safari lacks some H.264 configs and `VideoEncoder`).
