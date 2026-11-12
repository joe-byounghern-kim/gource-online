// SPDX-License-Identifier: GPL-3.0-or-later

//! T-18: box-walk tests of the MP4 writer (REC-55 to REC-61, REC-71).

mod support;

use gource_mux::MuxError;
use gource_mux::mp4::{
    Colour, Layout, Metadata, Mp4Config, Mp4Writer, Patch, Placement, RECIPE_MAX_BYTES,
    apply_patches, encode_patches,
};
use support::h264::{self, Geometry};
use support::walk::{self, Movie, be32, be64};
use support::{Rng, pts_us};

const TINY: Geometry = Geometry {
    width: 64,
    height: 36,
};

fn tiny_avcc() -> Vec<u8> {
    h264::avcc(&h264::sps(TINY), &h264::pps())
}

fn config(layout: Layout, fps: u32) -> Mp4Config {
    Mp4Config {
        layout,
        ..Mp4Config::new(TINY.width, TINY.height, fps)
    }
}

const LAYOUTS: [Layout; 3] = [
    Layout::Fragmented,
    Layout::Faststart {
        max_samples: 0,
        max_sync_samples: 0,
    },
    Layout::FaststartDeferred,
];

/// Faststart layouts sized for `n` samples.
fn sized(layout: Layout, n: u64) -> Layout {
    match layout {
        Layout::Faststart { .. } => Layout::Faststart {
            max_samples: n,
            max_sync_samples: n,
        },
        other => other,
    }
}

struct Input<'a> {
    data: &'a [u8],
    pts: i64,
    key: bool,
}

struct Muxed {
    /// The finished file: patches applied, deferred tail inserted.
    file: Vec<u8>,
    /// The bytes as streamed, before patches.
    streamed: Vec<u8>,
    patches: Vec<Patch>,
    placement: Placement,
    writer: Mp4Writer,
}

/// Muxes like the web controller does: drains after every call, keeps parts,
/// applies patches at the end.
fn mux(
    cfg: Mp4Config,
    meta: Option<&Metadata>,
    inputs: &[Input<'_>],
    last_duration_us: u64,
) -> Muxed {
    let mut w = Mp4Writer::new(cfg, &tiny_avcc(), meta).unwrap();
    let mut streamed = Vec::new();
    w.drain_output(&mut streamed);
    for (i, s) in inputs.iter().enumerate() {
        let dur = if i + 1 == inputs.len() {
            last_duration_us
        } else {
            0
        };
        w.push(s.data, s.pts, dur, s.key).unwrap();
        w.drain_output(&mut streamed);
    }
    w.finish().unwrap();
    let mut tail = Vec::new();
    w.drain_output(&mut tail);
    let placement = w.placement();
    let mut file = streamed.clone();
    match placement {
        Placement::Append => file.extend_from_slice(&tail),
        Placement::InsertAt { offset, len } => {
            let at = offset as usize;
            file.splice(at..at, tail[..len].iter().copied());
            file.extend_from_slice(&tail[len..]);
        }
    }
    streamed.extend_from_slice(&tail);
    let patches = w.patches().to_vec();
    apply_patches(&patches, &mut file);
    assert_eq!(w.bytes_written(), file.len() as u64);
    Muxed {
        file,
        streamed,
        patches,
        placement,
        writer: w,
    }
}

/// Checks the walked samples against the input, byte for byte.
fn assert_samples(movie: &Movie, file: &[u8], inputs: &[Input<'_>], timescale: u64) {
    assert_eq!(movie.samples.len(), inputs.len());
    let origin = inputs.first().map_or(0, |s| s.pts);
    for (i, (got, want)) in movie.samples.iter().zip(inputs).enumerate() {
        assert_eq!(got.size as usize, want.data.len(), "size of sample {i}");
        assert_eq!(got.key, want.key, "key flag of sample {i}");
        let at = got.offset as usize;
        assert!(
            &file[at..at + want.data.len()] == want.data,
            "bytes of sample {i}"
        );
        let rel = (want.pts - origin) as u64;
        let ticks = (rel * timescale + 500_000) / 1_000_000;
        assert_eq!(got.decode_time, ticks, "decode time of sample {i}");
    }
}

fn h264_inputs(stream: &h264::Stream, fps: u64) -> Vec<Input<'_>> {
    stream
        .frames
        .iter()
        .enumerate()
        .map(|(i, (data, key))| Input {
            data,
            pts: pts_us(i as u64, fps),
            key: *key,
        })
        .collect()
}

#[test]
fn fragmented_structure() {
    let stream = h264::stream(TINY, 20, 6);
    let inputs = h264_inputs(&stream, 30);
    let m = mux(config(Layout::Fragmented, 30), None, &inputs, 0);
    let movie = walk::read_movie(&m.file, false).unwrap();

    let mut top = vec!["ftyp", "moov"];
    for _ in 0..4 {
        top.extend(["moof", "mdat"]);
    }
    top.push("mfra");
    assert_eq!(movie.top, top);
    assert_eq!(movie.brand, "iso5");
    assert_eq!(movie.header_version, 1);
    assert_eq!(
        (movie.mvhd_timescale, movie.mdhd_timescale),
        (90_000, 90_000)
    );
    assert_eq!((movie.width, movie.height), (64, 36));
    assert_eq!((movie.entry_width, movie.entry_height), (64, 36));
    assert_eq!(movie.avcc, stream.avcc, "avcC is copied verbatim");
    assert_eq!(
        movie.colr,
        Some((1, 1, 1, false)),
        "BT.709 limited (REC-47)"
    );
    assert_eq!(movie.pasp, Some((1, 1)));
    assert!(movie.tags.is_empty());
    assert_samples(&movie, &m.file, &inputs, 90_000);
    assert!(movie.samples.iter().all(|s| s.duration == 3000));

    // One fragment per GOP, numbered from 1, each starting with its keyframe.
    let seqs: Vec<u32> = movie.fragments.iter().map(|f| f.sequence).collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
    let lens: Vec<usize> = movie.fragments.iter().map(|f| f.samples).collect();
    assert_eq!(lens, [6, 6, 6, 2]);
    let mut first = 0;
    for f in &movie.fragments {
        assert!(movie.samples[first].key);
        assert!(
            movie.samples[first + 1..first + f.samples]
                .iter()
                .all(|s| !s.key)
        );
        first += f.samples;
    }
    // mfra indexes every fragment.
    let index: Vec<(u64, u64)> = movie
        .fragments
        .iter()
        .map(|f| (f.base_time, f.moof_offset))
        .collect();
    assert_eq!(movie.tfra, index);
    let mfro = movie.mfro_size.unwrap() as usize;
    assert_eq!(
        &m.file[m.file.len() - mfro + 4..m.file.len() - mfro + 8],
        b"mfra"
    );

    // Durations: zero as streamed, exact after the patches (REC-58).
    // `mdhd` covers only the samples in `moov` (none), so it stays zero:
    // AVFoundation adds the fragments to it and would double the length.
    let total = 20 * 3000;
    assert_eq!(movie.mvhd_duration, total);
    assert_eq!(movie.tkhd_duration, total);
    assert_eq!(movie.mdhd_duration, 0);
    assert_eq!(movie.mehd_duration, Some(total));
    let raw = walk::read_movie(&m.streamed, false).unwrap();
    assert_eq!(
        (raw.mvhd_duration, raw.tkhd_duration, raw.mdhd_duration),
        (0, 0, 0)
    );
    assert_eq!(raw.mehd_duration, Some(0));
    assert_eq!(m.writer.frames(), 20);
    assert_eq!(m.writer.duration_us(), 666_667);
    assert_eq!(m.placement, Placement::Append);
}

#[test]
fn patches_wire_format() {
    let stream = h264::stream(TINY, 7, 3);
    let inputs = h264_inputs(&stream, 25);
    let m = mux(config(Layout::Fragmented, 25), None, &inputs, 0);
    assert_eq!(m.patches.len(), 3);
    let mut wire = Vec::new();
    encode_patches(&m.patches, &mut wire);
    assert_eq!(wire.len(), 3 * (8 + 4 + 8));
    let mut decoded = Vec::new();
    let mut at = 0;
    while at < wire.len() {
        let offset = u64::from_le_bytes(wire[at..at + 8].try_into().unwrap());
        let len = u32::from_le_bytes(wire[at + 8..at + 12].try_into().unwrap()) as usize;
        decoded.push(Patch {
            offset,
            bytes: wire[at + 12..at + 12 + len].to_vec(),
        });
        at += 12 + len;
    }
    assert_eq!(decoded, m.patches);
    for p in &m.patches {
        assert_eq!(be64(&p.bytes), 7 * 3600);
        // Every patch lands inside the header, which is written first.
        assert!(p.offset < 2000);
    }
}

/// REC-61 (b): synthetic samples, every layout, keyframe patterns, 1 to
/// 100,000 samples, key intervals 1 to 300, sizes 0 to 1 MB with at most
/// 64 MiB per case.
#[test]
fn sample_tables_match_input() {
    const CAP: u64 = 64 << 20;
    let mut pool = vec![0u8; 1 << 20];
    let mut rng = Rng(0x5eed);
    for b in pool.iter_mut() {
        *b = rng.next() as u8;
    }
    let mut cases = 0;
    for (n, interval) in [
        (1, 1),
        (2, 1),
        (3, 2),
        (7, 300),
        (60, 7),
        (301, 30),
        (1000, 120),
        (4000, 0), // random keyframes
        (100_000, 300),
        (100_000, 1),
    ] {
        for max_size in [0u64, 9, 4096, 1 << 20] {
            let max_size = max_size.min(CAP / n);
            let mut inputs = Vec::with_capacity(n as usize);
            for i in 0..n {
                let size = if max_size == 0 {
                    0
                } else {
                    rng.below(max_size + 1) as usize
                };
                let start = rng.below((pool.len() - size) as u64 + 1) as usize;
                let key = match interval {
                    0 => i == 0 || rng.below(9) == 0,
                    k => i % k == 0,
                };
                inputs.push(Input {
                    data: &pool[start..start + size],
                    pts: pts_us(i, 30) + 1_000_000,
                    key,
                });
            }
            for layout in LAYOUTS {
                let m = mux(config(sized(layout, n), 30), None, &inputs, 0);
                let movie = walk::read_movie(&m.file, false)
                    .unwrap_or_else(|e| panic!("{layout:?} n {n} interval {interval}: {e}"));
                assert_samples(&movie, &m.file, &inputs, 90_000);
                assert!(movie.samples.iter().all(|s| s.duration == 3000));
                assert_eq!(movie.mvhd_duration, n * 3000);
                let mdhd = if layout == Layout::Fragmented {
                    0
                } else {
                    n * 3000
                };
                assert_eq!(movie.mdhd_duration, mdhd);
                if layout == Layout::Fragmented {
                    let keys = inputs.iter().filter(|s| s.key).count();
                    assert_eq!(movie.fragments.len(), keys);
                    assert!(
                        movie
                            .fragments
                            .iter()
                            .enumerate()
                            .all(|(i, f)| f.sequence == i as u32 + 1)
                    );
                } else if layout == Layout::FaststartDeferred {
                    assert_eq!(movie.top, ["ftyp", "moov", "mdat"]);
                } else {
                    assert_eq!(movie.top, ["ftyp", "moov", "free", "mdat"]);
                }
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 10 * 4 * 3);
}

#[test]
fn fragments_split_by_sample_count() {
    let stream = h264::stream(TINY, 25, 10);
    let inputs = h264_inputs(&stream, 30);
    let cfg = Mp4Config {
        max_fragment_samples: 4,
        ..config(Layout::Fragmented, 30)
    };
    let m = mux(cfg, None, &inputs, 0);
    let movie = walk::read_movie(&m.file, false).unwrap();
    let lens: Vec<usize> = movie.fragments.iter().map(|f| f.samples).collect();
    assert_eq!(lens, [4, 4, 2, 4, 4, 2, 4, 1]);
    assert_samples(&movie, &m.file, &inputs, 90_000);
    // Only fragments that start with a keyframe are random access points.
    let starts: Vec<u64> = movie.tfra.iter().map(|&(t, _)| t / 3000).collect();
    assert_eq!(starts, [0, 10, 20]);
}

/// Exact frame durations at 24, 25, 30 and 60 fps over an hour: no drift.
#[test]
fn durations_exact_over_long_runs() {
    let payload = [0u8, 0, 0, 1, 0x65];
    for fps in [24u64, 25, 30, 60] {
        let n = fps * 3600;
        let nominal = 90_000 / fps as u32;
        let inputs: Vec<Input<'_>> = (0..n)
            .map(|i| Input {
                data: &payload,
                pts: pts_us(i, fps),
                key: i % (2 * fps) == 0,
            })
            .collect();
        let last_duration = (pts_us(n, fps) - pts_us(n - 1, fps)) as u64;
        for layout in [Layout::Fragmented, sized(LAYOUTS[1], n)] {
            let m = mux(config(layout, fps as u32), None, &inputs, last_duration);
            let movie = walk::read_movie(&m.file, false).unwrap();
            assert_eq!(movie.samples.len() as u64, n);
            for (i, s) in movie.samples.iter().enumerate() {
                assert_eq!(s.duration, nominal, "{fps} fps sample {i}");
                assert_eq!(s.decode_time, i as u64 * u64::from(nominal));
            }
            assert_eq!(movie.mvhd_duration, n * u64::from(nominal));
            assert_eq!(m.writer.duration_us(), 3_600_000_000);
            if !matches!(layout, Layout::Fragmented) {
                // One stts run for the whole hour.
                let stts = find(&m.file, b"stts");
                assert_eq!(be32(&m.file[stts + 12..]), 1, "{fps} fps stts entries");
            }
        }
    }
}

/// Offset of the first box of `kind` (its size field).
fn find(file: &[u8], kind: &[u8; 4]) -> usize {
    file.windows(4).position(|w| w == kind).unwrap() - 4
}

/// REC-61 (d): cutting a fragmented file anywhere leaves a file that walks as
/// valid up to the last complete fragment.
#[test]
fn truncated_file_keeps_complete_fragments() {
    let mut rng = Rng(7);
    let payloads: Vec<Vec<u8>> = (0..24).map(|i| vec![rng.next() as u8; 5 + i % 7]).collect();
    let inputs: Vec<Input<'_>> = payloads
        .iter()
        .enumerate()
        .map(|(i, data)| Input {
            data,
            pts: pts_us(i as u64, 60),
            key: i % 5 == 0,
        })
        .collect();
    let m = mux(config(Layout::Fragmented, 60), None, &inputs, 0);
    let full = walk::read_movie(&m.file, false).unwrap();
    let top = walk::walk(&m.file).unwrap();
    let moov_end = top[1].offset + top[1].size;
    let mfra_at = top.last().unwrap().offset;
    // Each fragment ends where the next one (or mfra) starts.
    let ends: Vec<usize> = full
        .fragments
        .iter()
        .skip(1)
        .map(|f| f.moof_offset as usize)
        .chain([mfra_at])
        .collect();
    for cut in moov_end..m.file.len() {
        let movie =
            walk::read_movie(&m.file[..cut], true).unwrap_or_else(|e| panic!("cut {cut}: {e}"));
        let complete: usize = full
            .fragments
            .iter()
            .zip(&ends)
            .filter(|(_, end)| **end <= cut)
            .map(|(f, _)| f.samples)
            .sum();
        assert_eq!(movie.samples.len(), complete, "cut {cut}");
        assert_samples(&movie, &m.file, &inputs[..complete], 90_000);
    }
}

#[test]
fn faststart_puts_moov_first() {
    let stream = h264::stream(TINY, 90, 30);
    let inputs = h264_inputs(&stream, 30);
    let layout = Layout::Faststart {
        max_samples: 100,
        max_sync_samples: 4,
    };
    let m = mux(config(layout, 30), None, &inputs, 0);
    let movie = walk::read_movie(&m.file, false).unwrap();
    assert_eq!(movie.top, ["ftyp", "moov", "free", "mdat"]);
    assert_eq!(movie.brand, "isom");
    assert!(!m.writer.moov_at_end());
    assert_samples(&movie, &m.file, &inputs, 90_000);
    assert_eq!(movie.mvhd_duration, 90 * 3000);
    assert_eq!(movie.tkhd_duration, 90 * 3000);
    assert_eq!(movie.mehd_duration, None);
    assert!(!movie.co64);
    // One chunk per second of samples.
    assert_eq!(be32(&m.file[find(&m.file, b"stco") + 12..]), 3);
    // As streamed: the reserve is a free box and the mdat header claims no
    // payload until patched.
    let mdat_at = m.patches[1].offset as usize - 8;
    let raw = walk::walk(&m.streamed[..mdat_at + 16]).unwrap();
    let kinds: Vec<String> = raw.iter().map(|n| walk::name(&n.kind)).collect();
    assert_eq!(kinds, ["ftyp", "free", "mdat"].map(String::from));
    // Two patches: moov into the reserve, the mdat size.
    assert_eq!(m.patches.len(), 2);
    assert_eq!(&m.patches[0].bytes[4..8], b"moov");
    assert_eq!(m.placement, Placement::Append);
}

#[test]
fn faststart_outgrowing_the_reserve_appends_moov() {
    let payload = [0u8, 0, 0, 1, 0x65];
    let n = 20_000;
    let inputs: Vec<Input<'_>> = (0..n)
        .map(|i| Input {
            data: &payload,
            pts: pts_us(i, 60),
            key: true,
        })
        .collect();
    let layout = Layout::Faststart {
        max_samples: 10,
        max_sync_samples: 1,
    };
    let m = mux(config(layout, 60), None, &inputs, 0);
    assert!(m.writer.moov_at_end());
    let movie = walk::read_movie(&m.file, false).unwrap();
    assert_eq!(movie.top, ["ftyp", "free", "mdat", "moov"]);
    assert_samples(&movie, &m.file, &inputs, 90_000);
    assert_eq!(m.patches.len(), 1, "only the mdat size");
}

#[test]
fn deferred_layout_inserts_moov_after_ftyp() {
    let stream = h264::stream(TINY, 40, 12);
    let inputs = h264_inputs(&stream, 24);
    let m = mux(config(Layout::FaststartDeferred, 24), None, &inputs, 0);
    let movie = walk::read_movie(&m.file, false).unwrap();
    assert_eq!(movie.top, ["ftyp", "moov", "mdat"]);
    assert!(m.patches.is_empty());
    assert_samples(&movie, &m.file, &inputs, 90_000);
    assert!(movie.samples.iter().all(|s| s.duration == 3750));
    let ftyp_len = be32(&m.file) as u64;
    match m.placement {
        Placement::InsertAt { offset, len } => {
            assert_eq!(offset, ftyp_len);
            assert_eq!(len as u64, movie_len(&m.file, 1) + 8);
        }
        other => panic!("{other:?}"),
    }

    // Without draining before finish, the writer splices moov itself and the
    // output is the same file in order.
    let mut w = Mp4Writer::new(config(Layout::FaststartDeferred, 24), &tiny_avcc(), None).unwrap();
    for s in &inputs {
        w.push(s.data, s.pts, 0, s.key).unwrap();
    }
    w.finish().unwrap();
    assert_eq!(w.placement(), Placement::Append);
    assert_eq!(w.pending_output(), &m.file[..]);
}

/// Size of the `index`-th top-level box.
fn movie_len(file: &[u8], index: usize) -> u64 {
    walk::walk(file).unwrap()[index].size as u64
}

/// More than 4 GiB of sample data switches `stco` to `co64` (and `mdat` to a
/// 64-bit size). The data is never held: the output is drained into one
/// reused buffer and counted, like a sink that discards it.
#[test]
fn co64_beyond_4_gib() {
    let sample = vec![0x5au8; 1 << 20];
    let n = 4200u64; // 4200 MiB
    for layout in [
        Layout::Faststart {
            max_samples: n,
            max_sync_samples: n / 60 + 1,
        },
        Layout::FaststartDeferred,
    ] {
        let mut w = Mp4Writer::new(config(layout, 30), &tiny_avcc(), None).unwrap();
        let mut head = Vec::new();
        w.drain_output(&mut head);
        let mut scratch = Vec::with_capacity(2 << 20);
        let mut streamed = head.len() as u64;
        for i in 0..n {
            w.push(&sample, pts_us(i, 30), 0, i % 60 == 0).unwrap();
            w.drain_output(&mut scratch);
            streamed += scratch.len() as u64;
            scratch.clear();
        }
        w.finish().unwrap();
        let mut tail = Vec::new();
        w.drain_output(&mut tail);
        streamed += tail.len() as u64;
        assert_eq!(w.bytes_written(), streamed);

        let (moov, data_start, mdat_size) = match layout {
            Layout::FaststartDeferred => {
                let moov_len = be32(&tail) as usize;
                assert_eq!(
                    &tail[moov_len..moov_len + 8],
                    &[0, 0, 0, 1, b'm', b'd', b'a', b't']
                );
                let mdat_size = be64(&tail[moov_len + 8..]);
                (
                    tail[..moov_len].to_vec(),
                    (head.len() + tail.len()) as u64,
                    mdat_size,
                )
            }
            _ => {
                assert!(tail.is_empty());
                let p = w.patches();
                assert_eq!(p.len(), 2);
                let moov_len = be32(&p[0].bytes) as usize;
                let mdat_at = p[1].offset - 8;
                (
                    p[0].bytes[..moov_len].to_vec(),
                    mdat_at + 16,
                    be64(&p[1].bytes),
                )
            }
        };
        assert_eq!(mdat_size, 16 + n * (1 << 20));
        assert!(walk::walk(&moov).is_ok());
        assert!(!moov.windows(4).any(|k| k == b"stco"));
        let co64 = find(&moov, b"co64");
        let count = be32(&moov[co64 + 12..]) as u64;
        assert_eq!(count, n.div_ceil(30));
        for c in 0..count {
            let off = be64(&moov[co64 + 16 + 8 * c as usize..]);
            assert_eq!(off, data_start + c * 30 * (1 << 20));
        }
        assert!(be64(&moov[co64 + 16 + 8 * (count as usize - 1)..]) > u64::from(u32::MAX));
        assert_eq!(be32(&moov[find(&moov, b"stsz") + 16..]), n as u32);
    }
}

#[test]
fn embedded_recipe_tags() {
    let recipe = br#"{"v":1,"app":"gource-web","options":{"seconds-per-day":"2"}}"#.to_vec();
    let meta = Metadata {
        comment: "Made with gource-web: https://example.org/?repo=github.com/o/r".into(),
        build: "abc1234".into(),
        recipe: recipe.clone(),
    };
    let stream = h264::stream(TINY, 3, 3);
    let inputs = h264_inputs(&stream, 30);
    for layout in LAYOUTS {
        let m = mux(config(sized(layout, 3), 30), Some(&meta), &inputs, 0);
        assert!(!m.writer.recipe_dropped());
        let movie = walk::read_movie(&m.file, false).unwrap();
        let tags: Vec<(String, Vec<u8>)> = vec![
            ("?cmt".into(), meta.comment.clone().into_bytes()),
            ("?too".into(), b"gource-web abc1234".to_vec()),
            ("----:org.gource-web:recipe".into(), recipe.clone()),
        ];
        assert_eq!(movie.tags, tags, "{layout:?}");
        assert_samples(&movie, &m.file, &inputs, 90_000);
    }

    // Over the cap: the recipe is left out and flagged; the rest stays.
    let big = Metadata {
        comment: String::new(),
        build: String::new(),
        recipe: vec![b'x'; RECIPE_MAX_BYTES + 1],
    };
    let m = mux(config(Layout::Fragmented, 30), Some(&big), &inputs, 0);
    assert!(m.writer.recipe_dropped());
    let movie = walk::read_movie(&m.file, false).unwrap();
    assert_eq!(
        movie.tags,
        vec![("?too".to_string(), b"gource-web".to_vec())]
    );
    let at_cap = Metadata {
        recipe: vec![b'x'; RECIPE_MAX_BYTES],
        ..big
    };
    let m = mux(config(Layout::Fragmented, 30), Some(&at_cap), &inputs, 0);
    assert!(!m.writer.recipe_dropped());
}

#[test]
fn output_is_deterministic_and_independent_of_draining() {
    let stream = h264::stream(TINY, 50, 7);
    let inputs = h264_inputs(&stream, 30);
    for layout in [Layout::Fragmented, sized(LAYOUTS[1], 50)] {
        let a = mux(config(layout, 30), None, &inputs, 0);
        let b = mux(config(layout, 30), None, &inputs, 0);
        assert_eq!(a.file, b.file);
        // Draining through a small fixed buffer gives the same stream.
        let mut w = Mp4Writer::new(config(layout, 30), &tiny_avcc(), None).unwrap();
        let mut got = Vec::new();
        let mut dest = [0u8; 97];
        let mut drain = |w: &mut Mp4Writer, got: &mut Vec<u8>| loop {
            let n = w.take_output(&mut dest);
            if n == 0 {
                break;
            }
            got.extend_from_slice(&dest[..n]);
        };
        drain(&mut w, &mut got);
        for s in &inputs {
            let pending = w.push(s.data, s.pts, 0, s.key).unwrap();
            assert_eq!(pending, w.pending_output().len());
            drain(&mut w, &mut got);
        }
        w.finish().unwrap();
        drain(&mut w, &mut got);
        assert_eq!(got, a.streamed);
    }
}

#[test]
fn fragmented_push_returns_zero_until_a_gop_closes() {
    let stream = h264::stream(TINY, 13, 6);
    let mut w = Mp4Writer::new(config(Layout::Fragmented, 30), &stream.avcc, None).unwrap();
    let header = w.pending_output().len();
    assert!(header > 0);
    let mut header_bytes = Vec::new();
    w.drain_output(&mut header_bytes);
    let pending: Vec<usize> = stream
        .frames
        .iter()
        .enumerate()
        .map(|(i, (d, k))| {
            let n = w.push(d, pts_us(i as u64, 30), 0, *k).unwrap();
            let mut sink = Vec::new();
            w.drain_output(&mut sink);
            n
        })
        .collect();
    let closed: Vec<usize> = pending
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(closed, [6, 12]);
}

#[test]
fn errors_are_reported_and_leave_the_writer_usable() {
    let d = [0u8, 0, 0, 1, 0x65];
    let mut w = Mp4Writer::new(config(Layout::Fragmented, 30), &tiny_avcc(), None).unwrap();
    assert_eq!(w.push(&d, 0, 0, false), Err(MuxError::FirstSampleNotKey));
    w.push(&d, 1000, 0, true).unwrap();
    w.push(&d, 34_333, 0, false).unwrap();
    let reorder = w.push(&d, 20_000, 0, false).unwrap_err();
    assert_eq!(
        reorder,
        MuxError::NonMonotonic {
            previous_us: 34_333,
            pts_us: 20_000
        }
    );
    assert_eq!(reorder.code(), "record_mux_reorder");
    assert!(matches!(
        w.push(&d, 34_333, 0, false),
        Err(MuxError::NonMonotonic { .. })
    ));
    // 1 us later rounds to the same 90 kHz tick.
    assert!(matches!(
        w.push(&d, 34_334, 0, false),
        Err(MuxError::NonMonotonic { .. })
    ));
    assert!(matches!(
        w.push(&d, i64::MIN, 0, false),
        Err(MuxError::NonMonotonic { .. })
    ));
    // A gap beyond 32 bits of ticks.
    let gap = w.push(&d, 34_333 + 50_000_000_000, 0, false).unwrap_err();
    assert_eq!(gap, MuxError::DurationTooLarge);
    assert_eq!(gap.code(), "record_mux_failed");
    w.push(&d, 67_667, 0, false).unwrap();
    w.finish().unwrap();
    assert_eq!(w.frames(), 3);
    assert_eq!(w.push(&d, 100_000, 0, true), Err(MuxError::Finished));
    assert_eq!(w.finish(), Err(MuxError::Finished));
    let mut file = Vec::new();
    w.drain_output(&mut file);
    apply_patches(w.patches(), &mut file);
    let movie = walk::read_movie(&file, false).unwrap();
    let durations: Vec<u32> = movie.samples.iter().map(|s| s.duration).collect();
    assert_eq!(durations, [3000, 3000, 3000]);
}

#[test]
fn last_sample_duration() {
    let d = [0u8, 0, 0, 1, 0x65];
    // Given with the chunk: used for the last sample.
    let mut w = Mp4Writer::new(config(Layout::Fragmented, 60), &tiny_avcc(), None).unwrap();
    w.push(&d, 0, 16_667, true).unwrap();
    w.push(&d, 16_667, 40_000, false).unwrap();
    assert_eq!(
        w.duration_ticks(),
        1500 + 1500,
        "open sample counts as 1/fps"
    );
    w.finish().unwrap();
    assert_eq!(w.duration_ticks(), 1500 + 3600);
    // Empty run: a valid file with no samples.
    for layout in LAYOUTS {
        let mut w = Mp4Writer::new(config(sized(layout, 0), 30), &tiny_avcc(), None).unwrap();
        w.finish().unwrap();
        let mut file = Vec::new();
        w.drain_output(&mut file);
        apply_patches(w.patches(), &mut file);
        let movie = walk::read_movie(&file, false).unwrap();
        assert!(movie.samples.is_empty());
        assert_eq!(w.duration_us(), 0);
    }
}

#[test]
fn config_is_validated() {
    let avcc = tiny_avcc();
    let try_new = |cfg: Mp4Config| Mp4Writer::new(cfg, &avcc, None).err();
    assert!(matches!(
        try_new(Mp4Config::new(0, 36, 30)),
        Some(MuxError::InvalidConfig(_))
    ));
    assert!(matches!(
        try_new(Mp4Config::new(64, 70_000, 30)),
        Some(MuxError::InvalidConfig(_))
    ));
    assert!(matches!(
        try_new(Mp4Config::new(64, 36, 0)),
        Some(MuxError::InvalidConfig(_))
    ));
    let odd = Mp4Config {
        timescale: 1000,
        ..Mp4Config::new(64, 36, 24)
    };
    assert!(matches!(try_new(odd), Some(MuxError::InvalidConfig(_))));
    assert_eq!(
        try_new(Mp4Config::new(1920, 1080, 30)),
        Some(MuxError::SizeMismatch {
            config: (1920, 1080),
            sps: (64, 36)
        })
    );
    assert!(matches!(
        Mp4Writer::new(Mp4Config::new(64, 36, 30), &[], None).err(),
        Some(MuxError::InvalidAvcc(_))
    ));
    let huge = Mp4Config {
        layout: Layout::Faststart {
            max_samples: 1 << 31,
            max_sync_samples: 1 << 31,
        },
        ..Mp4Config::new(64, 36, 30)
    };
    assert!(matches!(try_new(huge), Some(MuxError::InvalidConfig(_))));
    // Other code points pass straight through to colr.
    let full = Mp4Config {
        colour: Colour {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: true,
        },
        ..Mp4Config::new(64, 36, 30)
    };
    let mut w = Mp4Writer::new(full, &avcc, None).unwrap();
    w.push(&[], 0, 0, true).unwrap();
    w.finish().unwrap();
    let mut file = Vec::new();
    w.drain_output(&mut file);
    assert_eq!(
        walk::read_movie(&file, false).unwrap().colr,
        Some((9, 16, 9, true))
    );
}

/// The real-encoder vector (see `tests/avc.rs`): x264 samples remuxed in
/// every layout keep their bytes, keyframes and order.
#[test]
fn x264_vector_remuxes() {
    let src = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/x264-high-64x36.mp4"
    ))
    .unwrap();
    let movie = walk::read_movie(&src, false).unwrap();
    assert_eq!(movie.samples.len(), 12);
    let inputs: Vec<Input<'_>> = movie
        .samples
        .iter()
        .enumerate()
        .map(|(i, s)| Input {
            data: &src[s.offset as usize..s.offset as usize + s.size as usize],
            pts: pts_us(i as u64, 30),
            key: s.key,
        })
        .collect();
    assert_eq!(inputs.iter().filter(|s| s.key).count(), 2);
    for layout in LAYOUTS {
        let cfg = Mp4Config {
            layout: sized(layout, 12),
            ..Mp4Config::new(64, 36, 30)
        };
        let mut w = Mp4Writer::new(cfg, &movie.avcc, None).unwrap();
        let mut file = Vec::new();
        w.drain_output(&mut file);
        for s in &inputs {
            w.push(s.data, s.pts, 0, s.key).unwrap();
            w.drain_output(&mut file);
        }
        w.finish().unwrap();
        let mut tail = Vec::new();
        w.drain_output(&mut tail);
        if let Placement::InsertAt { offset, len } = w.placement() {
            file.splice(offset as usize..offset as usize, tail.drain(..len));
        }
        file.extend_from_slice(&tail);
        apply_patches(w.patches(), &mut file);
        let out = walk::read_movie(&file, false).unwrap();
        assert_eq!(out.avcc, movie.avcc);
        assert_samples(&out, &file, &inputs, 90_000);
    }
}

/// REC-61 (e): with `GOURCE_MUX_FFPROBE=1`, `ffprobe` (on `PATH`) must agree
/// on codec, size, frame count and duration, decoding every frame of the tiny
/// real H.264 stream in each layout. The files are left in the target's tmp
/// dir for manual playback checks.
#[test]
fn ffprobe_agrees() {
    if std::env::var("GOURCE_MUX_FFPROBE").as_deref() != Ok("1") {
        eprintln!("skipped: set GOURCE_MUX_FFPROBE=1 to run ffprobe");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let g = Geometry {
        width: 320,
        height: 180,
    };
    let stream = h264::stream(g, 150, 60);
    let inputs = h264_inputs(&stream, 60);
    for (name, layout) in [
        ("fragmented", Layout::Fragmented),
        ("faststart", sized(LAYOUTS[1], 150)),
        ("deferred", Layout::FaststartDeferred),
    ] {
        let cfg = Mp4Config {
            layout,
            ..Mp4Config::new(g.width, g.height, 60)
        };
        let mut w = Mp4Writer::new(
            cfg,
            &stream.avcc,
            Some(&Metadata {
                comment: "ffprobe test".into(),
                build: "test".into(),
                recipe: br#"{"v":1}"#.to_vec(),
            }),
        )
        .unwrap();
        let mut file = Vec::new();
        w.drain_output(&mut file);
        for s in &inputs {
            w.push(s.data, s.pts, 0, s.key).unwrap();
            w.drain_output(&mut file);
        }
        w.finish().unwrap();
        let mut tail = Vec::new();
        w.drain_output(&mut tail);
        if let Placement::InsertAt { offset, len } = w.placement() {
            file.splice(offset as usize..offset as usize, tail.drain(..len));
        }
        file.extend_from_slice(&tail);
        apply_patches(w.patches(), &mut file);
        let path = dir.join(format!("gource-mux-{name}.mp4"));
        std::fs::write(&path, &file).unwrap();
        let out = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-show_entries",
                "stream=codec_name,width,height,nb_read_frames,duration:format_tags=comment,encoder",
                "-of",
                "default=nw=1",
            ])
            .arg(&path)
            .output()
            .expect("ffprobe on PATH");
        let text = String::from_utf8_lossy(&out.stdout);
        let errors = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && errors.trim().is_empty(),
            "{name}: {errors}"
        );
        for want in [
            "codec_name=h264",
            "width=320",
            "height=180",
            "nb_read_frames=150",
            "duration=2.500000",
            "TAG:comment=ffprobe test",
            "TAG:encoder=gource-web test",
        ] {
            assert!(
                text.lines().any(|l| l == want),
                "{name}: missing {want} in\n{text}"
            );
        }
    }
}
