// SPDX-License-Identifier: GPL-3.0-or-later

//! [`Mp4Writer`]: the streaming H.264 MP4 muxer.

use super::boxes::{
    self, DurationFields, FLAGS_NON_SYNC, FLAGS_SYNC, Put, Tables, Track, close, mdat_header,
    mdat_header_large, open, open_full, write_ftyp, write_moov,
};
use super::meta::{Metadata, encode_udta};
use super::{Layout, Mp4Config, Patch, Placement};
use crate::MuxError;
use crate::avc::AvcConfig;

/// Room for `stts` runs beyond the regular one or two (irregular sample
/// durations) and anything else unforeseen in the faststart reserve.
const RESERVE_SLACK: u64 = 64 * 1024;

/// Streams H.264 access units into an MP4 file.
///
/// Every produced byte goes to one output buffer, in file order: the header
/// (queued by [`Mp4Writer::new`]), fragments or sample data (queued by
/// [`Mp4Writer::push`]) and the tail (queued by [`Mp4Writer::finish`]). The
/// caller drains it with [`Mp4Writer::take_output`] or
/// [`Mp4Writer::drain_output`] whenever a call returns a non-zero length; the
/// buffer keeps its capacity, so a steady run allocates nothing per frame.
/// After `finish`, [`Mp4Writer::patches`] lists in-place edits for a seekable
/// sink and [`Mp4Writer::placement`] says where the tail goes.
///
/// A rejected call leaves the writer unchanged, so the caller can still
/// `finish` and keep what was muxed ("Stop and save").
pub struct Mp4Writer {
    config: Mp4Config,
    avc: AvcConfig,
    udta: Vec<u8>,
    recipe_dropped: bool,
    out: Vec<u8>,
    /// Bytes already taken out of `out`: the file offset of `out[0]`.
    drained: u64,
    origin_us: i64,
    last: Option<Last>,
    frames: u64,
    /// Sum of the finished sample durations, in ticks.
    total_ticks: u64,
    finished: bool,
    patches: Vec<Patch>,
    placement: Placement,
    moov_at_end: bool,
    body: Body,
}

/// The newest sample, whose duration is known only when the next one comes.
#[derive(Clone, Copy)]
struct Last {
    pts_us: i64,
    ticks: u64,
    duration_us: u64,
}

enum Body {
    Fragmented(Fragments),
    Flat(Flat),
}

struct Fragments {
    durations: DurationFields,
    sequence: u32,
    samples: Vec<FragSample>,
    data: Vec<u8>,
    start_ticks: u64,
    starts_with_key: bool,
    /// `tfra` entries: (decode time, absolute `moof` offset).
    index: Vec<(u64, u64)>,
}

struct FragSample {
    size: u32,
    duration: u32,
    flags: u32,
}

struct Flat {
    sizes: Vec<u32>,
    stts: Vec<(u32, u32)>,
    sync: Vec<u32>,
    /// Chunk offsets relative to the first payload byte.
    chunk_offsets: Vec<u64>,
    chunk_samples: u32,
    payload: u64,
    mode: FlatMode,
}

enum FlatMode {
    Reserve {
        free_at: u64,
        free_len: u64,
        mdat_at: u64,
    },
    Deferred {
        header_len: u64,
    },
}

impl Mp4Writer {
    /// Validates the config and the `AVCDecoderConfigurationRecord` (WebCodecs
    /// `decoderConfig.description`), and queues the file header: `ftyp` and
    /// `moov` (fragmented), `ftyp`, the `free` reserve and the `mdat` header
    /// (faststart), or `ftyp` alone (deferred). `metadata` adds
    /// `moov/udta/meta` (REC-71).
    pub fn new(
        config: Mp4Config,
        avcc: &[u8],
        metadata: Option<&Metadata>,
    ) -> Result<Self, MuxError> {
        let bad = MuxError::InvalidConfig;
        if !(1..=0xffff).contains(&config.width) || !(1..=0xffff).contains(&config.height) {
            return Err(bad("width and height must be 1 to 65535"));
        }
        if config.fps == 0 || config.timescale == 0 || !config.timescale.is_multiple_of(config.fps)
        {
            return Err(bad("timescale must be a non-zero multiple of fps"));
        }
        let avc = AvcConfig::parse(avcc)?;
        if let Some(sps) = avc.sps
            && (sps.width, sps.height) != (config.width, config.height)
        {
            return Err(MuxError::SizeMismatch {
                config: (config.width, config.height),
                sps: (sps.width, sps.height),
            });
        }
        let (udta, recipe_dropped) = metadata.map_or((Vec::new(), false), encode_udta);

        let mut out = Vec::new();
        let body = match config.layout {
            Layout::Fragmented => {
                write_ftyp(&mut out, true);
                let track = Track {
                    config: &config,
                    avcc,
                    udta: &udta,
                };
                let durations = write_moov(&mut out, &track, None, 0);
                Body::Fragmented(Fragments {
                    durations,
                    sequence: 1,
                    samples: Vec::new(),
                    data: Vec::new(),
                    start_ticks: 0,
                    starts_with_key: false,
                    index: Vec::new(),
                })
            }
            Layout::Faststart {
                max_samples,
                max_sync_samples,
            } => {
                write_ftyp(&mut out, false);
                let track = Track {
                    config: &config,
                    avcc,
                    udta: &udta,
                };
                let free_len = reserve_len(&track, max_samples, max_sync_samples);
                let free_size = u32::try_from(free_len)
                    .map_err(|_| bad("faststart reserve larger than 4 GiB"))?;
                let free_at = out.len() as u64;
                out.u32(free_size);
                out.bytes(b"free");
                out.resize(out.len() + free_len as usize - 8, 0);
                let mdat_at = out.len() as u64;
                mdat_header_large(&mut out, 0);
                Body::Flat(Flat::new(
                    &config,
                    FlatMode::Reserve {
                        free_at,
                        free_len,
                        mdat_at,
                    },
                ))
            }
            Layout::FaststartDeferred => {
                write_ftyp(&mut out, false);
                let header_len = out.len() as u64;
                Body::Flat(Flat::new(&config, FlatMode::Deferred { header_len }))
            }
        };
        Ok(Mp4Writer {
            config,
            avc,
            udta,
            recipe_dropped,
            out,
            drained: 0,
            origin_us: 0,
            last: None,
            frames: 0,
            total_ticks: 0,
            finished: false,
            patches: Vec::new(),
            placement: Placement::Append,
            moov_at_end: false,
            body,
        })
    }

    /// Adds one encoded chunk: `data` is the access unit in AVCC form
    /// (length-prefixed NAL units, passed through unchanged, REC-57),
    /// `pts_us` its presentation time, `duration_us` its duration (0 when
    /// unknown; only the last sample's is used, the others follow from the
    /// next timestamp) and `is_key` the chunk type. Returns the length of the
    /// pending output: in the fragmented layout 0 until a keyframe closes the
    /// previous GOP's fragment.
    ///
    /// Errors: the first sample is not a keyframe, the timestamp does not
    /// increase (no B-frame reordering in this writer, REC-53), a size or
    /// gap overflows 32 bits, or the writer is finished.
    pub fn push(
        &mut self,
        data: &[u8],
        pts_us: i64,
        duration_us: u64,
        is_key: bool,
    ) -> Result<usize, MuxError> {
        if self.finished {
            return Err(MuxError::Finished);
        }
        let size =
            u32::try_from(data.len()).map_err(|_| MuxError::SampleTooLarge(data.len() as u64))?;
        let (origin, ticks, delta) = match self.last {
            None if !is_key => return Err(MuxError::FirstSampleNotKey),
            None => (pts_us, 0, None),
            Some(last) => {
                let reject = MuxError::NonMonotonic {
                    previous_us: last.pts_us,
                    pts_us,
                };
                let rel = pts_us
                    .checked_sub(self.origin_us)
                    .and_then(|d| u64::try_from(d).ok())
                    .ok_or(reject.clone())?;
                let ticks = self.ticks(rel);
                if pts_us <= last.pts_us || ticks <= last.ticks {
                    return Err(reject);
                }
                let delta =
                    u32::try_from(ticks - last.ticks).map_err(|_| MuxError::DurationTooLarge)?;
                (self.origin_us, ticks, Some(delta))
            }
        };
        if let Some(delta) = delta {
            self.end_sample(delta);
        }
        self.origin_us = origin;
        let at = self.drained + self.out.len() as u64;
        match &mut self.body {
            Body::Fragmented(f) => {
                let limit = self.config.max_fragment_samples as usize;
                if !f.samples.is_empty() && (is_key || (limit > 0 && f.samples.len() >= limit)) {
                    f.close(&mut self.out, at);
                }
                if f.samples.is_empty() {
                    f.start_ticks = ticks;
                    f.starts_with_key = is_key;
                }
                f.samples.push(FragSample {
                    size,
                    duration: 0,
                    flags: if is_key { FLAGS_SYNC } else { FLAGS_NON_SYNC },
                });
                f.data.extend_from_slice(data);
            }
            Body::Flat(flat) => {
                if flat.sizes.len().is_multiple_of(flat.chunk_samples as usize) {
                    flat.chunk_offsets.push(flat.payload);
                }
                flat.sizes.push(size);
                if is_key {
                    flat.sync.push(boxes::count(flat.sizes.len()));
                }
                flat.payload += u64::from(size);
                self.out.extend_from_slice(data);
            }
        }
        self.last = Some(Last {
            pts_us,
            ticks,
            duration_us,
        });
        self.frames += 1;
        Ok(self.out.len())
    }

    /// Ends the file: the last sample gets its duration (the one passed with
    /// it, else `1/fps`), the last fragment and `mfra` (fragmented) or `moov`
    /// are queued, and the patches are computed. Returns the length of the
    /// pending output. Call after `encoder.flush()`.
    pub fn finish(&mut self) -> Result<usize, MuxError> {
        if self.finished {
            return Err(MuxError::Finished);
        }
        if let Some(last) = self.last {
            let nominal = u64::from(self.config.timescale / self.config.fps);
            let delta = if last.duration_us > 0 {
                let rel = (last.pts_us - self.origin_us) as u64;
                self.ticks(rel.saturating_add(last.duration_us))
                    .saturating_sub(last.ticks)
                    .max(1)
            } else {
                nominal
            };
            self.end_sample(u32::try_from(delta).map_err(|_| MuxError::DurationTooLarge)?);
        }
        self.finished = true;
        let at = self.drained + self.out.len() as u64;
        let total = self.total_ticks;
        let track = Track {
            config: &self.config,
            avcc: self.avc.record(),
            udta: &self.udta,
        };
        match &mut self.body {
            Body::Fragmented(f) => {
                if !f.samples.is_empty() {
                    f.close(&mut self.out, at);
                }
                f.write_mfra(&mut self.out);
                // Not `mdhd`: in a fragmented file it covers only the samples
                // in `moov` (none). AVFoundation (QuickTime, Safari) adds the
                // fragments to it, so a patched `mdhd` doubles the length.
                let d = &f.durations;
                for offset in [d.mvhd, d.tkhd, d.mehd] {
                    self.patches.push(Patch {
                        offset: offset as u64,
                        bytes: total.to_be_bytes().to_vec(),
                    });
                }
            }
            Body::Flat(flat) => {
                let mut moov = Vec::new();
                match flat.mode {
                    FlatMode::Reserve {
                        free_at,
                        free_len,
                        mdat_at,
                    } => {
                        write_moov(&mut moov, &track, Some(&flat.tables(mdat_at + 16)), total);
                        let len = moov.len() as u64;
                        if len == free_len || len + 8 <= free_len {
                            if len < free_len {
                                moov.u32((free_len - len) as u32);
                                moov.bytes(b"free");
                            }
                            self.patches.push(Patch {
                                offset: free_at,
                                bytes: moov,
                            });
                        } else {
                            // The run outgrew the reserve: moov goes last and
                            // the reserve stays a free box.
                            self.out.extend_from_slice(&moov);
                            self.moov_at_end = true;
                        }
                        self.patches.push(Patch {
                            offset: mdat_at + 8,
                            bytes: (flat.payload + 16).to_be_bytes().to_vec(),
                        });
                    }
                    FlatMode::Deferred { header_len } => {
                        // moov's size does not depend on the offset values,
                        // only on stco versus co64; measure, then write.
                        let large = flat.payload + 8 > u64::from(u32::MAX);
                        let hdr = if large { 16 } else { 8 };
                        write_moov(
                            &mut moov,
                            &track,
                            Some(&flat.tables(header_len + hdr)),
                            total,
                        );
                        let base = header_len + moov.len() as u64 + hdr;
                        moov.clear();
                        write_moov(&mut moov, &track, Some(&flat.tables(base)), total);
                        if base != header_len + moov.len() as u64 + hdr {
                            // Switched from stco to co64: one more pass.
                            let base = header_len + moov.len() as u64 + hdr;
                            moov.clear();
                            write_moov(&mut moov, &track, Some(&flat.tables(base)), total);
                        }
                        if large {
                            mdat_header_large(&mut moov, flat.payload);
                        } else {
                            mdat_header(&mut moov, flat.payload);
                        }
                        // Splice at the insertion point while it is still in
                        // the buffer; otherwise the caller inserts.
                        if let Some(pos) = header_len.checked_sub(self.drained) {
                            let pos = pos as usize;
                            self.out.splice(pos..pos, moov);
                        } else {
                            let len = moov.len();
                            self.out.splice(0..0, moov);
                            self.placement = Placement::InsertAt {
                                offset: header_len,
                                len,
                            };
                        }
                    }
                }
            }
        }
        Ok(self.out.len())
    }

    fn end_sample(&mut self, delta: u32) {
        self.total_ticks += u64::from(delta);
        match &mut self.body {
            Body::Fragmented(f) => {
                if let Some(s) = f.samples.last_mut() {
                    s.duration = delta;
                }
            }
            Body::Flat(flat) => match flat.stts.last_mut() {
                Some((n, d)) if *d == delta => *n += 1,
                _ => flat.stts.push((1, delta)),
            },
        }
    }

    fn ticks(&self, rel_us: u64) -> u64 {
        let t = (u128::from(rel_us) * u128::from(self.config.timescale) + 500_000) / 1_000_000;
        u64::try_from(t).unwrap_or(u64::MAX)
    }

    /// The pending output, in file order.
    pub fn pending_output(&self) -> &[u8] {
        &self.out
    }

    /// Moves up to `dest.len()` pending bytes into `dest`; returns how many.
    /// No allocation (the wasm `take_output(dest)`).
    pub fn take_output(&mut self, dest: &mut [u8]) -> usize {
        let n = dest.len().min(self.out.len());
        dest[..n].copy_from_slice(&self.out[..n]);
        self.out.drain(..n);
        self.drained += n as u64;
        n
    }

    /// Appends all pending bytes to `dest`.
    pub fn drain_output(&mut self, dest: &mut Vec<u8>) {
        dest.extend_from_slice(&self.out);
        self.drained += self.out.len() as u64;
        self.out.clear();
    }

    /// In-place edits for a seekable sink, valid after `finish`: the
    /// `mvhd`, `tkhd` and `mehd` durations (fragmented, REC-58) or `moov`
    /// into the reserve and the `mdat` size (faststart, REC-59). Empty for
    /// the deferred layout.
    pub fn patches(&self) -> &[Patch] {
        &self.patches
    }

    /// Where the bytes queued by `finish` go.
    pub fn placement(&self) -> Placement {
        self.placement
    }

    /// Faststart only: the run outgrew the reserve, so `moov` was appended at
    /// the end (the file is valid and seekable, but not faststart).
    pub fn moov_at_end(&self) -> bool {
        self.moov_at_end
    }

    /// The recipe exceeded [`super::RECIPE_MAX_BYTES`] and was not embedded.
    pub fn recipe_dropped(&self) -> bool {
        self.recipe_dropped
    }

    /// The parsed `avcC`.
    pub fn avc(&self) -> &AvcConfig {
        &self.avc
    }

    /// The configuration.
    pub fn config(&self) -> &Mp4Config {
        &self.config
    }

    /// File length so far (drained plus pending bytes).
    pub fn bytes_written(&self) -> u64 {
        self.drained + self.out.len() as u64
    }

    /// Samples accepted.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Duration in ticks of the timescale: the finished samples plus, before
    /// `finish`, `1/fps` for the newest one.
    pub fn duration_ticks(&self) -> u64 {
        let open = if self.finished || self.last.is_none() {
            0
        } else {
            u64::from(self.config.timescale / self.config.fps)
        };
        self.total_ticks + open
    }

    /// [`Mp4Writer::duration_ticks`] in microseconds, rounded.
    pub fn duration_us(&self) -> u64 {
        let ts = u128::from(self.config.timescale);
        let us = (u128::from(self.duration_ticks()) * 1_000_000 + ts / 2) / ts;
        u64::try_from(us).unwrap_or(u64::MAX)
    }
}

impl Fragments {
    /// Queues `moof` and `mdat` for the buffered samples; `at` is the file
    /// offset where `moof` lands.
    fn close(&mut self, out: &mut Vec<u8>, at: u64) {
        let moof = open(out, b"moof");
        let mfhd = open_full(out, b"mfhd", 0, 0);
        out.u32(self.sequence);
        close(out, mfhd);
        let traf = open(out, b"traf");
        let tfhd = open_full(out, b"tfhd", 0, 0x02_0000); // default-base-is-moof
        out.u32(1);
        close(out, tfhd);
        let tfdt = open_full(out, b"tfdt", 1, 0);
        out.u64(self.start_ticks);
        close(out, tfdt);
        // data-offset, sample-duration, sample-size, sample-flags present
        let trun = open_full(out, b"trun", 0, 0x00_0701);
        out.u32(boxes::count(self.samples.len()));
        let data_offset_at = out.len();
        out.u32(0);
        for s in &self.samples {
            out.u32(s.duration);
            out.u32(s.size);
            out.u32(s.flags);
        }
        close(out, trun);
        close(out, traf);
        close(out, moof);
        let payload = self.data.len() as u64;
        let header = if payload + 8 > u64::from(u32::MAX) {
            16
        } else {
            8
        };
        let data_offset = u32::try_from(out.len() - moof + header).expect("moof over 2 GiB");
        out[data_offset_at..data_offset_at + 4].copy_from_slice(&data_offset.to_be_bytes());
        if header == 16 {
            mdat_header_large(out, payload);
        } else {
            mdat_header(out, payload);
        }
        out.extend_from_slice(&self.data);
        if self.starts_with_key {
            self.index.push((self.start_ticks, at));
        }
        self.sequence += 1;
        self.samples.clear();
        self.data.clear();
    }

    /// Queues `mfra`: a `tfra` with one entry per fragment that starts with a
    /// keyframe, and `mfro`.
    fn write_mfra(&self, out: &mut Vec<u8>) {
        let mfra = open(out, b"mfra");
        let tfra = open_full(out, b"tfra", 1, 0);
        out.u32(1); // track_ID
        out.u32(0); // traf, trun and sample numbers are 1 byte each
        out.u32(boxes::count(self.index.len()));
        for &(time, offset) in &self.index {
            out.u64(time);
            out.u64(offset);
            out.bytes(&[1, 1, 1]);
        }
        close(out, tfra);
        let mfro = open_full(out, b"mfro", 0, 0);
        let size = u32::try_from(out.len() + 4 - mfra).expect("mfra over 4 GiB");
        out.u32(size);
        close(out, mfro);
        close(out, mfra);
    }
}

impl Flat {
    fn new(config: &Mp4Config, mode: FlatMode) -> Self {
        Flat {
            sizes: Vec::new(),
            stts: Vec::new(),
            sync: Vec::new(),
            chunk_offsets: Vec::new(),
            chunk_samples: config.fps,
            payload: 0,
            mode,
        }
    }

    fn tables(&self, base: u64) -> Tables<'_> {
        Tables {
            sizes: &self.sizes,
            stts: &self.stts,
            sync: &self.sync,
            chunk_samples: self.chunk_samples,
            chunk_offsets: &self.chunk_offsets,
            base,
        }
    }
}

/// Bytes to reserve (as one `free` box) for a flat `moov` of at most
/// `samples` samples, `syncs` of them keyframes: the empty-table `moov` plus
/// 4 bytes per `stsz` and `stss` entry, 8 per `co64` chunk offset, two `stsc`
/// entries, and [`RESERVE_SLACK`] for irregular `stts` runs. This replaces
/// the spec's estimate `4 * N + 64 KiB`, which leaves out keyframes and
/// chunk offsets.
fn reserve_len(track: &Track<'_>, samples: u64, syncs: u64) -> u64 {
    let empty = Tables {
        sizes: &[],
        stts: &[],
        sync: &[],
        chunk_samples: track.config.fps,
        chunk_offsets: &[],
        base: 0,
    };
    let mut moov = Vec::new();
    write_moov(&mut moov, track, Some(&empty), 0);
    let chunks = samples.div_ceil(u64::from(track.config.fps));
    moov.len() as u64 + 4 * samples + 4 * syncs.min(samples) + 8 * chunks + 24 + RESERVE_SLACK + 8
}
