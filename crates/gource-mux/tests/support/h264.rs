// SPDX-License-Identifier: GPL-3.0-or-later

//! A tiny but real H.264 Baseline stream for the tests: SPS, PPS, IDR pictures
//! made of I_PCM macroblocks (raw samples, no transform or entropy coding),
//! and P pictures where every macroblock is skipped. It decodes with any
//! conforming decoder, so a file muxed from it can be played and probed.

#![allow(dead_code)]

#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    cur: u8,
    used: u8,
}

impl BitWriter {
    pub fn bit(&mut self, b: bool) {
        self.cur = (self.cur << 1) | u8::from(b);
        self.used += 1;
        if self.used == 8 {
            self.bytes.push(self.cur);
            self.cur = 0;
            self.used = 0;
        }
    }

    pub fn bits(&mut self, v: u64, n: u32) {
        for i in (0..n).rev() {
            self.bit((v >> i) & 1 == 1);
        }
    }

    pub fn ue(&mut self, v: u32) {
        let x = u64::from(v) + 1;
        let len = 64 - x.leading_zeros();
        self.bits(0, len - 1);
        self.bits(x, len);
    }

    pub fn se(&mut self, v: i32) {
        let k = if v > 0 { 2 * v - 1 } else { -2 * v };
        self.ue(k as u32);
    }

    pub fn align_zero(&mut self) {
        while self.used != 0 {
            self.bit(false);
        }
    }

    pub fn byte(&mut self, b: u8) {
        assert_eq!(self.used, 0);
        self.bytes.push(b);
    }

    /// `rbsp_trailing_bits()` and the finished RBSP.
    pub fn finish(mut self) -> Vec<u8> {
        self.bit(true);
        self.align_zero();
        self.bytes
    }
}

/// Inserts emulation prevention bytes.
pub fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp.len() + 8);
    let mut zeros = 0;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

pub fn nal(ref_idc: u8, kind: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut v = vec![(ref_idc << 5) | kind];
    v.extend(escape(rbsp));
    v
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
}

impl Geometry {
    pub fn mbs_w(self) -> u32 {
        self.width.div_ceil(16)
    }
    pub fn mbs_h(self) -> u32 {
        self.height.div_ceil(16)
    }
    pub fn mbs(self) -> u32 {
        self.mbs_w() * self.mbs_h()
    }
}

/// Baseline SPS: POC type 2, one reference frame, 4-bit frame_num, cropping
/// to the exact size (even sizes only).
pub fn sps(g: Geometry) -> Vec<u8> {
    assert!(g.width.is_multiple_of(2) && g.height.is_multiple_of(2));
    let mut w = BitWriter::default();
    w.bits(66, 8); // profile_idc Baseline
    w.bits(0b1100_0000, 8); // constraint_set0/1
    w.bits(30, 8); // level 3.0
    w.ue(0); // sps id
    w.ue(0); // log2_max_frame_num_minus4
    w.ue(2); // pic_order_cnt_type
    w.ue(1); // max_num_ref_frames
    w.bit(false); // gaps
    w.ue(g.mbs_w() - 1);
    w.ue(g.mbs_h() - 1);
    w.bit(true); // frame_mbs_only
    w.bit(true); // direct_8x8_inference
    let crop_r = (g.mbs_w() * 16 - g.width) / 2;
    let crop_b = (g.mbs_h() * 16 - g.height) / 2;
    let crop = crop_r != 0 || crop_b != 0;
    w.bit(crop);
    if crop {
        w.ue(0);
        w.ue(crop_r);
        w.ue(0);
        w.ue(crop_b);
    }
    w.bit(false); // vui
    nal(3, 7, &w.finish())
}

pub fn pps() -> Vec<u8> {
    let mut w = BitWriter::default();
    w.ue(0); // pps id
    w.ue(0); // sps id
    w.bit(false); // CAVLC
    w.bit(false); // bottom_field_pic_order_in_frame_present
    w.ue(0); // one slice group
    w.ue(0); // num_ref_idx_l0_default_minus1
    w.ue(0); // l1
    w.bit(false); // weighted_pred
    w.bits(0, 2); // weighted_bipred_idc
    w.se(0); // pic_init_qp_minus26
    w.se(0); // pic_init_qs_minus26
    w.se(0); // chroma_qp_index_offset
    w.bit(true); // deblocking_filter_control_present
    w.bit(false); // constrained_intra_pred
    w.bit(false); // redundant_pic_cnt_present
    nal(3, 8, &w.finish())
}

/// `AVCDecoderConfigurationRecord` with 4-byte NAL lengths.
pub fn avcc(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut v = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe1];
    v.extend((sps.len() as u16).to_be_bytes());
    v.extend(sps);
    v.push(1);
    v.extend((pps.len() as u16).to_be_bytes());
    v.extend(pps);
    v
}

/// IDR picture of I_PCM macroblocks; `shade` sets the luma level.
pub fn idr(g: Geometry, idr_pic_id: u32, shade: u8) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.ue(0); // first_mb_in_slice
    w.ue(7); // slice_type I (all)
    w.ue(0); // pps id
    w.bits(0, 4); // frame_num
    w.ue(idr_pic_id);
    w.bit(false); // no_output_of_prior_pics
    w.bit(false); // long_term_reference
    w.se(0); // slice_qp_delta
    w.ue(1); // disable_deblocking_filter_idc
    for mb in 0..g.mbs() {
        w.ue(25); // I_PCM
        w.align_zero();
        let luma = shade.wrapping_add((mb * 13) as u8).clamp(16, 235);
        for _ in 0..256 {
            w.byte(luma);
        }
        for _ in 0..128 {
            w.byte(128);
        }
    }
    nal(3, 5, &w.finish())
}

/// P picture with every macroblock skipped (a copy of the reference).
pub fn p_skip(g: Geometry, frame_num: u32) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.ue(0);
    w.ue(5); // slice_type P (all)
    w.ue(0);
    w.bits(u64::from(frame_num % 16), 4);
    w.bit(false); // num_ref_idx_active_override
    w.bit(false); // ref_pic_list_modification_flag_l0
    w.bit(false); // adaptive_ref_pic_marking_mode
    w.se(0);
    w.ue(1);
    w.ue(g.mbs()); // mb_skip_run
    nal(2, 1, &w.finish())
}

/// Length-prefixes NAL units (4 bytes), the AVCC sample form.
pub fn sample(nals: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::new();
    for n in nals {
        v.extend((n.len() as u32).to_be_bytes());
        v.extend(*n);
    }
    v
}

pub struct Stream {
    pub avcc: Vec<u8>,
    /// (AVCC sample, keyframe)
    pub frames: Vec<(Vec<u8>, bool)>,
}

/// `count` frames with an IDR every `gop` frames.
pub fn stream(g: Geometry, count: usize, gop: usize) -> Stream {
    let avcc = avcc(&sps(g), &pps());
    let mut frames = Vec::with_capacity(count);
    let mut idrs = 0;
    let mut frame_num = 0;
    for i in 0..count {
        if i % gop == 0 {
            frames.push((
                sample(&[&idr(g, idrs % 2, 40 + (idrs * 50 % 180) as u8)]),
                true,
            ));
            idrs += 1;
            frame_num = 0;
        } else {
            frame_num += 1;
            frames.push((sample(&[&p_skip(g, frame_num)]), false));
        }
    }
    Stream { avcc, frames }
}
