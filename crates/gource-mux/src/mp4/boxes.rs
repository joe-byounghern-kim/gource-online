// SPDX-License-Identifier: GPL-3.0-or-later

//! Box serialisation: size-prefixed boxes appended to a `Vec<u8>`, and the
//! `ftyp` and `moov` trees shared by every layout (ISO/IEC 14496-12 8.2 to 8.8,
//! ISO/IEC 14496-15 5.4).

use super::{Colour, Mp4Config};

/// Big-endian appends.
pub(super) trait Put {
    fn u8(&mut self, v: u8);
    fn u16(&mut self, v: u16);
    fn u32(&mut self, v: u32);
    fn u64(&mut self, v: u64);
    fn bytes(&mut self, v: &[u8]);
}

impl Put for Vec<u8> {
    fn u8(&mut self, v: u8) {
        self.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.extend_from_slice(v);
    }
}

/// Starts a box; returns its start for [`close`].
pub(super) fn open(buf: &mut Vec<u8>, kind: &[u8; 4]) -> usize {
    let at = buf.len();
    buf.u32(0);
    buf.bytes(kind);
    at
}

/// Starts a full box (version and 24-bit flags).
pub(super) fn open_full(buf: &mut Vec<u8>, kind: &[u8; 4], version: u8, flags: u32) -> usize {
    let at = open(buf, kind);
    buf.u32((u32::from(version) << 24) | (flags & 0x00ff_ffff));
    at
}

/// Writes the size of the box started at `at`. Only `mdat` can exceed 32 bits
/// and it is written separately ([`mdat_header`]).
pub(super) fn close(buf: &mut [u8], at: usize) {
    let size = u32::try_from(buf.len() - at).expect("metadata box larger than 4 GiB");
    buf[at..at + 4].copy_from_slice(&size.to_be_bytes());
}

/// An `mdat` header for `payload` bytes: 8 bytes, or 16 with a 64-bit size.
pub(super) fn mdat_header(buf: &mut Vec<u8>, payload: u64) {
    match u32::try_from(payload + 8) {
        Ok(size) => {
            buf.u32(size);
            buf.bytes(b"mdat");
        }
        Err(_) => mdat_header_large(buf, payload),
    }
}

/// An `mdat` header with a 64-bit `largesize` (16 bytes); the size sits at
/// byte 8 of the header.
pub(super) fn mdat_header_large(buf: &mut Vec<u8>, payload: u64) {
    buf.u32(1);
    buf.bytes(b"mdat");
    buf.u64(payload + 16);
}

const UNITY_MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// `und` packed as three 5-bit letters (ISO 639-2/T, offset 0x60).
const LANGUAGE_UND: u16 = (21 << 10) | (14 << 5) | 4;

/// Sample-entry flags in `trun` (ISO/IEC 14496-12 8.8.3.1): a sync sample
/// depends on nothing (`sample_depends_on` 2); any other sample depends on
/// others and is a non-sync sample.
pub(super) const FLAGS_SYNC: u32 = 0x0200_0000;
pub(super) const FLAGS_NON_SYNC: u32 = 0x0101_0000;

pub(super) fn write_ftyp(buf: &mut Vec<u8>, fragmented: bool) {
    let at = open(buf, b"ftyp");
    if fragmented {
        buf.bytes(b"iso5");
        buf.u32(512);
        for brand in [b"iso5", b"iso6", b"mp41", b"avc1"] {
            buf.bytes(brand);
        }
    } else {
        buf.bytes(b"isom");
        buf.u32(512);
        for brand in [b"isom", b"iso2", b"avc1", b"mp41"] {
            buf.bytes(brand);
        }
    }
    close(buf, at);
}

/// The per-file constants of the `moov` tree.
pub(super) struct Track<'a> {
    pub config: &'a Mp4Config,
    /// The `AVCDecoderConfigurationRecord`, copied verbatim into `avcC`.
    pub avcc: &'a [u8],
    /// A complete `udta` box, or empty.
    pub udta: &'a [u8],
}

/// Full sample tables of a flat file.
pub(super) struct Tables<'a> {
    pub sizes: &'a [u32],
    /// `stts` runs: (sample count, sample delta).
    pub stts: &'a [(u32, u32)],
    /// 1-based numbers of the sync samples.
    pub sync: &'a [u32],
    /// Samples per chunk (the last chunk may hold fewer).
    pub chunk_samples: u32,
    /// Chunk offsets relative to `base`.
    pub chunk_offsets: &'a [u64],
    /// Absolute offset of the first `mdat` payload byte.
    pub base: u64,
}

/// Absolute positions (in the buffer passed to [`write_moov`]) of the 64-bit
/// duration fields, for the fragmented layout's patches.
pub(super) struct DurationFields {
    pub mvhd: usize,
    pub tkhd: usize,
    pub mehd: usize,
}

/// Writes `moov`. With `tables` the sample tables are complete (flat layout);
/// without, they are empty and `mvex` follows the track (fragmented layout).
pub(super) fn write_moov(
    buf: &mut Vec<u8>,
    track: &Track<'_>,
    tables: Option<&Tables<'_>>,
    duration: u64,
) -> DurationFields {
    let cfg = track.config;
    let moov = open(buf, b"moov");

    let mvhd = open_full(buf, b"mvhd", 1, 0);
    buf.u64(cfg.created);
    buf.u64(cfg.created);
    buf.u32(cfg.timescale);
    let mvhd_duration = buf.len();
    buf.u64(duration);
    buf.u32(0x0001_0000); // rate 1.0
    buf.u16(0x0100); // volume 1.0
    buf.u16(0);
    buf.u64(0);
    for v in UNITY_MATRIX {
        buf.u32(v);
    }
    buf.bytes(&[0; 24]); // pre_defined
    buf.u32(2); // next_track_ID
    close(buf, mvhd);

    let trak = open(buf, b"trak");
    let tkhd = open_full(buf, b"tkhd", 1, 3); // enabled, in movie
    buf.u64(cfg.created);
    buf.u64(cfg.created);
    buf.u32(1); // track_ID
    buf.u32(0);
    let tkhd_duration = buf.len();
    buf.u64(duration);
    buf.u64(0);
    buf.u16(0); // layer
    buf.u16(0); // alternate_group
    buf.u16(0); // volume (video)
    buf.u16(0);
    for v in UNITY_MATRIX {
        buf.u32(v);
    }
    buf.u32(cfg.width << 16);
    buf.u32(cfg.height << 16);
    close(buf, tkhd);

    let mdia = open(buf, b"mdia");
    let mdhd = open_full(buf, b"mdhd", 1, 0);
    buf.u64(cfg.created);
    buf.u64(cfg.created);
    buf.u32(cfg.timescale);
    buf.u64(duration);
    buf.u16(LANGUAGE_UND);
    buf.u16(0);
    close(buf, mdhd);

    let hdlr = open_full(buf, b"hdlr", 0, 0);
    buf.u32(0);
    buf.bytes(b"vide");
    buf.bytes(&[0; 12]);
    buf.bytes(b"VideoHandler\0");
    close(buf, hdlr);

    let minf = open(buf, b"minf");
    let vmhd = open_full(buf, b"vmhd", 0, 1);
    buf.bytes(&[0; 8]); // graphicsmode, opcolor
    close(buf, vmhd);
    let dinf = open(buf, b"dinf");
    let dref = open_full(buf, b"dref", 0, 0);
    buf.u32(1);
    let url = open_full(buf, b"url ", 0, 1); // media in this file
    close(buf, url);
    close(buf, dref);
    close(buf, dinf);
    write_stbl(buf, track, tables);
    close(buf, minf);
    close(buf, mdia);
    close(buf, trak);

    let mut mehd_duration = 0;
    if tables.is_none() {
        let mvex = open(buf, b"mvex");
        let mehd = open_full(buf, b"mehd", 1, 0);
        mehd_duration = buf.len();
        buf.u64(duration);
        close(buf, mehd);
        let trex = open_full(buf, b"trex", 0, 0);
        buf.u32(1); // track_ID
        buf.u32(1); // default_sample_description_index
        buf.u32(0);
        buf.u32(0);
        buf.u32(0);
        close(buf, trex);
        close(buf, mvex);
    }
    buf.bytes(track.udta);
    close(buf, moov);
    DurationFields {
        mvhd: mvhd_duration,
        tkhd: tkhd_duration,
        mehd: mehd_duration,
    }
}

fn write_stbl(buf: &mut Vec<u8>, track: &Track<'_>, tables: Option<&Tables<'_>>) {
    let stbl = open(buf, b"stbl");
    write_stsd(buf, track);

    let stts = open_full(buf, b"stts", 0, 0);
    let runs = tables.map_or(&[][..], |t| t.stts);
    buf.u32(count(runs.len()));
    for &(n, delta) in runs {
        buf.u32(n);
        buf.u32(delta);
    }
    close(buf, stts);

    if let Some(t) = tables {
        let stss = open_full(buf, b"stss", 0, 0);
        buf.u32(count(t.sync.len()));
        for &n in t.sync {
            buf.u32(n);
        }
        close(buf, stss);
    }

    let stsc = open_full(buf, b"stsc", 0, 0);
    match tables {
        Some(t) if !t.sizes.is_empty() => {
            let chunks = count(t.chunk_offsets.len());
            let last = count(t.sizes.len()) - (chunks - 1) * t.chunk_samples;
            // Every chunk is full except possibly the last.
            let entries: &[(u32, u32)] = if chunks == 1 || last == t.chunk_samples {
                &[(1, last)]
            } else {
                &[(1, t.chunk_samples), (chunks, last)]
            };
            buf.u32(count(entries.len()));
            for &(first_chunk, samples) in entries {
                buf.u32(first_chunk);
                buf.u32(samples);
                buf.u32(1); // sample_description_index
            }
        }
        _ => buf.u32(0),
    }
    close(buf, stsc);

    let stsz = open_full(buf, b"stsz", 0, 0);
    buf.u32(0); // sample_size: sizes follow
    let sizes = tables.map_or(&[][..], |t| t.sizes);
    buf.u32(count(sizes.len()));
    for &size in sizes {
        buf.u32(size);
    }
    close(buf, stsz);

    let (offsets, base) = tables.map_or((&[][..], 0), |t| (t.chunk_offsets, t.base));
    let wide = offsets
        .last()
        .is_some_and(|&o| base + o > u64::from(u32::MAX));
    let stco = open_full(buf, if wide { b"co64" } else { b"stco" }, 0, 0);
    buf.u32(count(offsets.len()));
    for &o in offsets {
        if wide {
            buf.u64(base + o);
        } else {
            buf.u32((base + o) as u32);
        }
    }
    close(buf, stco);
    close(buf, stbl);
}

fn write_stsd(buf: &mut Vec<u8>, track: &Track<'_>) {
    let cfg = track.config;
    let stsd = open_full(buf, b"stsd", 0, 0);
    buf.u32(1);
    let avc1 = open(buf, b"avc1");
    buf.bytes(&[0; 6]);
    buf.u16(1); // data_reference_index
    buf.bytes(&[0; 16]); // pre_defined, reserved
    buf.u16(cfg.width as u16);
    buf.u16(cfg.height as u16);
    buf.u32(0x0048_0000); // 72 dpi
    buf.u32(0x0048_0000);
    buf.u32(0);
    buf.u16(1); // frame_count
    buf.bytes(&[0; 32]); // compressorname
    buf.u16(0x0018); // depth
    buf.u16(0xffff); // pre_defined -1
    let avcc = open(buf, b"avcC");
    buf.bytes(track.avcc);
    close(buf, avcc);
    write_colr(buf, cfg.colour);
    let pasp = open(buf, b"pasp");
    buf.u32(1);
    buf.u32(1);
    close(buf, pasp);
    close(buf, avc1);
    close(buf, stsd);
}

fn write_colr(buf: &mut Vec<u8>, c: Colour) {
    let colr = open(buf, b"colr");
    buf.bytes(b"nclx");
    buf.u16(c.primaries);
    buf.u16(c.transfer);
    buf.u16(c.matrix);
    buf.u8(if c.full_range { 0x80 } else { 0 });
    close(buf, colr);
}

/// A table length as the 32-bit count field. Tables hold at most one entry
/// per sample and sample counts are bounded by memory long before 2^32.
pub(super) fn count(n: usize) -> u32 {
    u32::try_from(n).expect("table longer than 2^32 entries")
}
