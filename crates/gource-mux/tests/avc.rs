// SPDX-License-Identifier: GPL-3.0-or-later

//! `avcC` validation and SPS decoding.

mod support;

use gource_mux::MuxError;
use gource_mux::avc::{AvcConfig, SpsInfo};
use support::h264::{self, BitWriter, Geometry};
use support::walk;

/// A High-profile SPS as a hardware encoder would write it, optionally with
/// scaling lists, POC type 0 or 1, and interlaced coding.
fn high_sps(
    width: u32,
    height: u32,
    scaling: bool,
    poc_type: u32,
    frame_mbs_only: bool,
) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.bits(100, 8);
    w.bits(0, 8);
    w.bits(42, 8);
    w.ue(0);
    w.ue(1); // chroma_format_idc 4:2:0
    w.ue(0); // bit_depth_luma_minus8
    w.ue(0);
    w.bit(false);
    w.bit(scaling);
    if scaling {
        for i in 0..8 {
            let present = i % 3 != 1;
            w.bit(present);
            if present {
                for k in 0..if i < 6 { 16 } else { 64 } {
                    w.se(if k == 3 { -8 } else { (k % 5) - 2 });
                }
            }
        }
    }
    w.ue(4);
    w.ue(poc_type);
    match poc_type {
        0 => w.ue(6),
        1 => {
            w.bit(false);
            w.se(-3);
            w.se(2);
            w.ue(3);
            for v in [1, -1, 7] {
                w.se(v);
            }
        }
        _ => {}
    }
    w.ue(4);
    w.bit(false);
    let mbs_w = width.div_ceil(16);
    let field = if frame_mbs_only { 1 } else { 2 };
    let map_h = height.div_ceil(16 * field);
    w.ue(mbs_w - 1);
    w.ue(map_h - 1);
    w.bit(frame_mbs_only);
    if !frame_mbs_only {
        w.bit(false);
    }
    w.bit(true);
    let crop_r = (mbs_w * 16 - width) / 2;
    let crop_b = (map_h * 16 * field - height) / (2 * field);
    w.bit(true);
    w.ue(0);
    w.ue(crop_r);
    w.ue(0);
    w.ue(crop_b);
    w.bit(true); // VUI follows; the parser stops before it
    w.bits(0xdead_beef, 32);
    h264::nal(3, 7, &w.finish())
}

#[test]
fn generated_baseline_sps_decodes() {
    for (w, h) in [
        (64, 36),
        (1920, 1080),
        (16, 16),
        (1280, 720),
        (3840, 2160),
        (1080, 1920),
    ] {
        let sps = SpsInfo::parse(&h264::sps(Geometry {
            width: w,
            height: h,
        }))
        .unwrap();
        assert_eq!((sps.width, sps.height), (w, h));
        assert_eq!(
            (sps.profile_idc, sps.level_idc, sps.chroma_format_idc),
            (66, 30, 1)
        );
        assert_eq!(sps.bit_depth_luma, 8);
    }
}

#[test]
fn high_profile_sps_with_scaling_lists_and_poc_types() {
    for scaling in [false, true] {
        for poc in [0, 1, 2] {
            for (w, h) in [(1920, 1080), (2560, 1440), (1080, 1080), (720, 1280)] {
                let sps = SpsInfo::parse(&high_sps(w, h, scaling, poc, true)).unwrap();
                assert_eq!(
                    (sps.width, sps.height),
                    (w, h),
                    "scaling {scaling} poc {poc}"
                );
                assert_eq!((sps.profile_idc, sps.level_idc), (100, 42));
            }
        }
    }
    // Interlaced coding doubles the vertical crop unit.
    let sps = SpsInfo::parse(&high_sps(1920, 1080, false, 0, false)).unwrap();
    assert_eq!((sps.width, sps.height), (1920, 1080));
}

#[test]
fn emulation_prevention_bytes_are_removed() {
    let mut escaped = 0;
    // Large Exp-Golomb codes put runs of 16 zero bits into the RBSP.
    for mbs in (1..=512u32).chain(32760..=32800).chain(65530..=65536) {
        let width = mbs * 16;
        let nal = high_sps(width, width, false, 2, true);
        // An inserted byte always shows as 00 00 03.
        if nal.windows(3).any(|w| w == [0, 0, 3]) {
            escaped += 1;
        }
        let sps = SpsInfo::parse(&nal).unwrap();
        assert_eq!((sps.width, sps.height), (width, width));
    }
    assert!(escaped > 0, "no width exercised emulation prevention");
}

#[test]
fn malformed_sps_is_none() {
    let full = high_sps(1920, 1080, true, 1, true);
    for len in 1..12 {
        assert_eq!(SpsInfo::parse(&full[..len]), None, "len {len}");
    }
    assert_eq!(SpsInfo::parse(&[0x68, 0xff]), None, "not NAL type 7");
    assert_eq!(SpsInfo::parse(&[]), None);
}

#[test]
fn avcc_fields_and_codec_string() {
    let g = Geometry {
        width: 64,
        height: 36,
    };
    let record = h264::avcc(&h264::sps(g), &h264::pps());
    let avc = AvcConfig::parse(&record).unwrap();
    assert_eq!(avc.record(), &record[..]);
    assert_eq!(
        (avc.profile_idc, avc.profile_compat, avc.level_idc),
        (66, 0xc0, 30)
    );
    assert_eq!(avc.nal_length_size, 4);
    assert_eq!(avc.codec_string(), "avc1.42c01e");
    assert_eq!(avc.sps.map(|s| (s.width, s.height)), Some((64, 36)));
}

#[test]
fn avcc_with_undecodable_sps_is_accepted_without_size() {
    // A truncated SPS body: the record is structurally valid, so muxing can
    // go ahead with the configured size.
    let sps = [0x67, 100, 0, 40];
    let record = h264::avcc(&sps, &h264::pps());
    let avc = AvcConfig::parse(&record).unwrap();
    assert_eq!(avc.sps, None);
    assert_eq!(avc.codec_string(), "avc1.640028");
}

#[test]
fn invalid_avcc_is_rejected() {
    let g = Geometry {
        width: 64,
        height: 36,
    };
    let good = h264::avcc(&h264::sps(g), &h264::pps());
    let bad = |bytes: &[u8]| matches!(AvcConfig::parse(bytes), Err(MuxError::InvalidAvcc(_)));
    assert!(bad(&[]));
    assert!(bad(&good[..6]));
    assert!(bad(&good[..good.len() - 1]), "truncated PPS");
    let mut v = good.clone();
    v[0] = 2;
    assert!(bad(&v), "version");
    let mut v = good.clone();
    v[4] = 0xfe;
    assert!(bad(&v), "3-byte NAL lengths");
    let mut v = good.clone();
    v[5] = 0xe0;
    assert!(bad(&v), "no SPS");
    let mut v = good.clone();
    v[8] = 0x68;
    assert!(bad(&v), "SPS slot holds a PPS");
    let sps_len = usize::from(u16::from_be_bytes([good[6], good[7]]));
    let mut v = good[..8 + sps_len].to_vec();
    v.push(0);
    assert!(bad(&v), "no PPS");
}

/// The real-encoder vector: x264 High profile, 64x36 (coded 64x48 with a
/// crop), 12 frames at 30 fps, GOP 6, no B-frames. Generated with
/// `ffmpeg -f lavfi -i testsrc=size=64x36:rate=30 -frames:v 12 -c:v libx264
/// -profile:v high -preset veryfast -g 6 -keyint_min 6 -sc_threshold 0 -bf 0
/// -pix_fmt yuv420p -movflags +faststart -map_metadata -1 -fflags +bitexact
/// -flags:v +bitexact x264-high-64x36.mp4` (ffmpeg 7.1).
#[test]
fn x264_record_decodes() {
    let file = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/x264-high-64x36.mp4"
    ))
    .unwrap();
    let movie = walk::read_movie(&file, false).unwrap();
    let avc = AvcConfig::parse(&movie.avcc).unwrap();
    assert_eq!(avc.profile_idc, 100);
    assert!(avc.codec_string().starts_with("avc1.64"));
    let sps = avc.sps.unwrap();
    assert_eq!((sps.width, sps.height, sps.chroma_format_idc), (64, 36, 1));
}
