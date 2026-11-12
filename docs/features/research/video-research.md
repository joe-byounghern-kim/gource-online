<!-- Research notes gathered on 2026-10-02 (branch web-mvp, HEAD 8a3ecec) for the feature specs in docs/features/. Working notes: claims are marked verified or inferred; file:line anchors describe that commit. -->

# External research: recording a WebGL2 canvas to a video file in Chrome, with no third-party JS

Status labels: **[V]** means a source I fetched states it, though WebFetch returns summaries from a small model, so treat exact wording as paraphrase. **[S]** means it appeared only in a search-result snippet. **[I]** means my inference or prior knowledge, not checked this session.

## 1. WebCodecs VideoEncoder

- **Availability.** WebCodecs shipped in Chrome 94 [V] (https://developer.chrome.com/blog/new-in-chrome-94). It needs a secure context and is also exposed in Dedicated Workers [V] (https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder).
- **Codec strings.**
  - H.264 uses `avc1.PPCCLL` or `avc3.PPCCLL`, as in RFC 6381. Examples: `avc1.42001f` (Baseline 3.1, used in https://devtails.xyz/adam/how-to-save-html-canvas-to-mp4-using-web-codecs-api), `avc1.640028` (High 4.0), `avc1.640033` (High 5.1) [V] (https://www.w3.org/TR/webcodecs-avc-codec-registration/).
  - VP9 uses `vp09.*`, AV1 uses `av01.*`, HEVC uses `hev1.*` or `hvc1.*` [V] (https://www.w3.org/TR/webcodecs-codec-registry/).
- **AVC `format`.** The registration defines two modes [V] (https://www.w3.org/TR/webcodecs-avc-codec-registration/).
  - `avc` is the default. Parameter sets (SPS/PPS) are kept out of the bitstream, and `decoderConfig.description` carries an `AVCDecoderConfigurationRecord`. This is the form MP4 wants, since that record becomes the `avcC` box.
  - `annexb` repeats SPS/PPS in the stream and gives no `description`. It suits live or mid-stream entry.
  - Set it with `avc: { format: "avc" }` in the config.
- **VP9 and AV1 chunks.**
  - VP9 chunks are raw frames, and `description` is unused [V] (https://www.w3.org/TR/webcodecs-vp9-codec-registration/).
  - AV1 chunks are in the low-overhead bitstream format, and `description` is unused [V] (https://www.w3.org/TR/webcodecs-av1-codec-registration/).
  - A muxer must therefore build `vpcC` or `av1C` itself from the codec string and the first keyframe. [I] For AV1 that means parsing the sequence header OBU.
- **Config fields.** [V] (https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder/configure)
  - `codec`, `width`, `height`, `displayWidth`, `displayHeight`.
  - `bitrate` and `framerate`.
  - `hardwareAcceleration`: `no-preference`, `prefer-hardware` or `prefer-software`.
  - `bitrateMode`: `constant`, `variable` (default) or `quantizer`.
  - `latencyMode`: `quality` (default) or `realtime`.
  - `alpha` and `scalabilityMode`.
- **Checking support.** Call `VideoEncoder.isConfigSupported(config)` first [V] (https://developer.chrome.com/docs/web-platform/best-practices/webcodecs). Check the exact config you will use, including `hardwareAcceleration`. [I] Support depends on the platform encoder at that resolution.
- **Keyframes.** `encode(frame, {keyFrame: true})` forces one [V]. There is no interval config field in what I fetched. Force a keyframe every N frames yourself, for example every 2 s of output time, so seeking works.
- **Frames from a canvas.** `new VideoFrame(canvas, {timestamp, duration})`. A VideoFrame is a CanvasImageSource [V] (Chrome best-practices page above). The sample code computes `timestamp = frameNumber * 1e6 / fps` in microseconds [V] (devtails link above). [I] Timestamps come from the simulation clock, not wall time. Pass `duration` explicitly. Compute each timestamp as `round(n*1e6/fps)` so rounding error does not accumulate.
- **WebGL read-back.** [I, not verified this session] The frame must be built right after drawing, in the same task, or the context needs `preserveDrawingBuffer: true`. Otherwise the frame can come out blank.
- **Backpressure.** `encodeQueueSize` is the number of pending encodes, and the `dequeue` event fires when it decreases [V] (https://www.w3.org/TR/webcodecs/). The Chrome guide drops frames when the queue is above 2, which suits live capture [V]. Offline export must not drop. Await a dequeue promise whenever `encodeQueueSize` exceeds a threshold of about 4 to 8 [I].
- **Frame lifetime.** Call `frame.close()` after every `encode()` [V].
- **Flush.** `await encoder.flush()` before finalizing the container [V] (devtails link above).
- **Colour.** See section 6.

## 2. Containers

**MP4 (ISOBMFF)**
- **Structure.** A non-fragmented file is `ftyp`, then `moov`, then `mdat`. `moov` contains `mvhd` and a `trak` with `tkhd`, `mdia`, `mdhd`, `hdlr`, `minf` and `stbl`. `stbl` holds `stsd` (with `avc1` containing `avcC`), `stts`, `stss` (sync samples), `stsc`, `stsz`, `stco` or `co64`, and optionally `ctts`. Browsers emit no B-frames in the default encoder setup [I], so `ctts` can usually be skipped. If a chunk's decode and presentation order differ, `ctts` is needed.
- **Streaming writes.** Fragmented MP4 is `ftyp`, `moov` (empty sample tables), then repeated `moof` plus `mdat`. It can be written with no seeking and survives truncation. Chrome's own MediaRecorder MP4 output is fragmented and has `moov` duration fields of 0, which breaks seekbars in Windows 11 Media Player [V] (https://blog.addpipe.com/duration-in-mp4-files-produced-by-chrome-safari/, observed on Chrome 147).
- **Preferred output.** Write fragmented MP4 as it streams to disk. Where possible, finish with a rewrite that puts `moov` first with the correct duration. With the File System Access API's seekable `write({type:'seek'})`, `moov` can be patched in place [I]. Otherwise reserve space for `moov` or use `co64` and a final patch.
- **Size.** [I] A hand-written H.264-only MP4 muxer is roughly 400 to 700 lines of TypeScript or Rust. The former mp4-muxer was described as about 20 KB per muxer [S] (https://musicvisualizerpro.com/blog/scaling-webcodecs-video-encoding-on-apache-servers/ via search summary). That library is now deprecated in favour of Mediabunny [V] (https://github.com/Vanilagy/mp4-muxer). Mediabunny supports MP4 (fast-start and fragmented), WebM and MKV [V] (https://mediabunny.dev/guide/introduction). It is third-party, so it is useful only as a reference implementation to read.
- **Compatibility of H.264 MP4.**
  - YouTube recommends High profile, 4:2:0, closed GOP [V, via search] (https://support.google.com/youtube/answer/1722171).
  - [I] H.264 High in MP4 plays in QuickTime, Windows Media Player and Movies & TV, and is accepted by Slack, Discord and X.
  - [I] Use `avc1.64xxxx`, with the level chosen from the resolution.
- **VP9 and AV1 in MP4.**
  - Windows Media Player does not play VP9-in-MP4 [S] (https://support.microsoft.com/en-us/topic/file-types-supported-by-windows-media-player-32d9998e-dc8f-af54-7ba1-e996f74375d9).
  - AV1 on Windows needs the AV1 Video Extension [S].
  - QuickTime/AVFoundation plays AV1 only with a hardware decoder, such as M3/M4 [S] (https://issues.chromium.org/issues/404291878).
  - Conclusion: H.264 MP4 is the compatibility baseline. VP9 or AV1 are optional "smaller file" choices, best in WebM.

**WebM/Matroska**
- **Elements.** EBML header, then `Segment` containing `Info`, `Tracks` (`CodecID` `V_VP9`, plus `CodecPrivate` for H.264/MKV), `Cluster`s of `SimpleBlock`s, and `Cues` [V, via search] (https://www.matroska.org/technical/notes.html).
- **Unknown sizes.** Segment and Cluster sizes may be "unknown" for streaming [S]. Players then lack duration and seeking unless you patch `Info`/`Duration` and `Cues` at the end.
- **Size.** [I] This is simpler than MP4. A VP9 WebM muxer is roughly 250 to 400 lines.
- **Fit.** WebM is the easiest fallback for Chrome-only sharing.

## 3. MediaRecorder + captureStream (fallback)

- **MP4 support.** Chrome 126 added MediaRecorder support for muxing audio and video into MP4 [V] (https://developer.chrome.com/release-notes/126). It covers H.264 video and AAC audio [V] (https://groups.google.com/a/chromium.org/g/blink-dev/c/p1OMVj1FrMI). Tracking bug: https://issues.chromium.org/issues/40127044. Per search results, it works only when the platform has an OS encoder [S], so use `MediaRecorder.isTypeSupported()`.
- **Output.** It is fragmented with zero duration (see section 2).
- **Frame pacing.** `captureStream()` captures a frame each time the canvas changes. `captureStream(0)` plus `track.requestFrame()` gives manual pacing [V] (https://developer.mozilla.org/en-US/docs/Web/API/HTMLCanvasElement/captureStream).
- **Real-time limit.** [I] MediaRecorder timestamps frames by wall clock, so fast or slow rendering warps playback speed. It gives variable frame rate and can drop frames under load. Use `requestFrame` pacing plus real-time delays only if WebCodecs is unavailable.
- **Verdict.** MediaRecorder is real-time only, so it cannot support "record at 4x" with deterministic output. Use it only as a feature-detected fallback.

## 4. Writing large outputs

- **File System Access.** `showSaveFilePicker()` needs a secure context and user activation, and returns a `FileSystemFileHandle` [V] (https://developer.mozilla.org/en-US/docs/Web/API/Window/showSaveFilePicker). Its availability is limited, not Baseline. [I] Chrome 86+. Open the file with `createWritable()` and stream chunks.
- **Picking the file.** Call the picker at the moment the user clicks Record, not at the end. After minutes of encoding the user activation is gone.
- **OPFS.** Chrome 86+, Firefox 111+, Safari 15.2+. Use `createWritable()` on the main thread and `createSyncAccessHandle()` in workers. Quota applies and it is cleared with site data. Files cannot be moved to user-visible storage, only copied through `showSaveFilePicker`, which is main-thread only [V] (https://web.dev/articles/origin-private-file-system). OPFS is a good staging area for a fallback path. Then expose the result as a `File`/Blob (backed by disk) for an anchor `download`.
- **Blob limits.**
  - In-memory blob storage is capped at 2 GB on x64 desktop [S].
  - `URL.createObjectURL` can fail on very large blobs, for example a 700 MB case [S] (https://issues.chromium.org/issues/375385420).
  - Revoke object URLs when done.
- **Sizing.** At 1080p and 8 Mbps, one minute is about 60 MB, so in-memory Blobs are fine for short clips. Stream to disk for long or 4K ones.

## 5. Hidden tabs and throttling

- **rAF.** Chrome does not call `requestAnimationFrame` in background tabs [V] (https://developer.chrome.com/blog/background_tabs).
- **Timer throttling.**
  - Since Chrome 57, background-tab timers get a budget that regenerates at 0.01 s per second after 10 s hidden [V] (same link).
  - Chrome 88+ applies intensive throttling, limiting a chain of timers to one wake-up per minute after the page has been hidden for more than 5 minutes [S] (https://groups.google.com/a/chromium.org/g/scheduler-dev/c/BVQbTtNU8Kw).
  - Audible audio and WebRTC are exempt [V/S].
- **Workers.** Timers in a dedicated Worker are reported as not throttled [S] (https://medium.com/@adithyaviswam/overcoming-browser-throttling-of-setinterval-executions-45387853a826). I did not verify throttling of worker rAF or message-driven loops.
- **OffscreenCanvas.** It is widely available since March 2023 [V] (https://developer.mozilla.org/en-US/docs/Web/API/OffscreenCanvas). `canvas.transferControlToOffscreen()` then `postMessage({canvas}, [canvas])` runs WebGL in a worker, and rAF can be called there [V].
- **Design consequence.** [I] An offline export should not use rAF at all. Loop `step(dt)`, render, encode, in the worker or on the main thread, driven by `await` on encoder backpressure rather than wall-clock timers. A background tab then still renders and encodes (a throttled `setTimeout` between steps would stall it, so avoid timers). Make the engine render into an OffscreenCanvas. Keep the live view on its own canvas.
- **Checks.** Assert behaviour in a hidden tab with a Playwright run. Chrome's `--disable-background-timer-throttling` flag is only for tests [V].

## 6. Resolution and colour

- **H.264 levels.** Max frame size is 8160 macroblocks at 4.0, 8192 at 4.1 and 36,864 at 5.1 [S] (https://blog.webex.com/engineering/modern-video-conferencing-systems-h264-sdp/). Macroblocks are 16x16 pixels.
  - 1920x1080 is 8160 macroblocks and fits Level 4.0.
  - 4096x2160 is 34,560 and needs 5.1 (`avc1.640033`).
- **Even dimensions.** 4:2:0 needs even width and height [I], so round down to even.
- **Hardware encoder limits (Chrome).**
  - Windows: before 130.0.6703.0 it was hardcoded to 1920x1088 at 30 fps. From 131.0.6759.0 it queries MediaFoundation, up to 7680x4320 [S] (https://chromium.googlesource.com/chromium/src/+/676fadac93a1fa1dbb8ebdda23b0405803a3435a%5E!/).
  - macOS: 4096x2304 [S].
  - Linux VA-API: 4096x4096 at 30 fps for H.264 [S].
  - Always check `isConfigSupported` and offer a smaller size if it fails.
- **Textures and drawing buffers.** I did not verify MAX_TEXTURE_SIZE or drawing-buffer limits. Query `gl.MAX_RENDERBUFFER_SIZE` and `gl.MAX_VIEWPORT_DIMS` [I]. To export at a size different from the screen, render to an OffscreenCanvas of that size. The aspect ratio must match what the camera framed.
- **Colour.**
  - A W3C issue reports canvas-sourced VP9 frames looking darker, especially in green, even with BT.709 metadata set on the VideoFrame and the WebM headers. The cause is unresolved, and VLC and Windows Movies & TV ignore the metadata [V] (https://github.com/w3c/webcodecs/issues/598).
  - Another report says BT.601 and BT.709 get confused when the encoder doesn't tag the colour space explicitly [S].
  - Recommendation [I]: build frames with `colorSpace: {primaries:'bt709', transfer:'bt709', matrix:'bt709', fullRange:false}` and write matching `colr`/VUI tags in the container. Compare an encoded frame against the source by pixel-diffing in a Playwright test. Gource's flat colours and bloom make a mismatch easy to see.

## 7. Audio

- No video-codec constraint requires audio [I]. I did not verify any platform's rule on silent tracks.
- A search summary claimed audio is required for YouTube and X. That is unverified and probably wrong, so do not rely on it.
- Plan for a silent-track option only if a specific target rejects videos with no audio.

## 8. Server-side alternative

I did not research this and cite no sources. Inference only [I]:

- **Headless Chrome.** Run the same WebCodecs export page and read the file back, for example through a download or the File System Access API under automation. GPU-less machines fall back to software WebGL (SwiftShader), which is slow.
- **Native route.** `gource-cli` has the engine and frame batches. A native render using `wgpu` or OpenGL would pipe raw frames to `ffmpeg` (H.264 `-pix_fmt yuv420p`, `-colorspace bt709`). That adds a second renderer to keep in parity, which is a large cost.
- **Practical choice.** For cloud renders, use headless Chrome running the browser export path, so there is one render path.

## Recommendations

1. Build the primary path on WebCodecs with H.264 `avc` format plus a hand-written fragmented-MP4 muxer, writing to a file chosen with `showSaveFilePicker` (Chrome 86+, WebCodecs 94+). Keep a VP9/WebM option as a second muxer. Skip AV1 in MP4 for now.
2. Drive export deterministically with `step(dt)`, one frame per `1/fps` of simulation time. Speed is `days_per_second` or the time scale, not wall time. Take timestamps from the frame counter.
3. Respect backpressure by awaiting `dequeue` when `encodeQueueSize` is above a threshold. Call `close()` on every frame and `flush()` before the final `moov`.
4. Don't use rAF or timers in the export loop. Render to a dedicated OffscreenCanvas, possibly in a worker, so hidden tabs still work.
5. Gate every option with `isConfigSupported`. Fall back to a smaller size, then to VP9/WebM, then to MediaRecorder with an honest "real-time only" warning.
6. Tag colour as BT.709 limited range and add a pixel-diff regression test.

**Not verified:** Chrome's MediaRecorder `video/mp4` platform coverage beyond the Chrome 126 release note; the exact Chrome version for `showSaveFilePicker` (stated as 86 from memory); worker rAF throttling; WebGL size limits; platform audio requirements.