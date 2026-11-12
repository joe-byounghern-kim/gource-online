# gource-mux

An MP4 writer for H.264, with no dependencies and no I/O. The browser encodes the frames with WebCodecs and this
crate puts them in a file the usual players open and seek.

Nothing here comes from Gource. Gource writes raw PPM frames (`--output-ppm-stream`) for an external encoder such
as ffmpeg; a browser has no such pipe, so the recorder muxes in the page.

| File | What |
|---|---|
| `mp4/` | The ISO BMFF boxes, the streaming writer, the embedded recipe and in-place edits for seekable sinks |
| `avc.rs` | The parts of H.264 the container needs: the decoder configuration record and the sequence parameter set |

The design and its requirements are in `docs/features/recording.md`.

    cargo test -p gource-mux

GPL-3.0-or-later. See `LICENSING.md`.
