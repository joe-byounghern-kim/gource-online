// SPDX-License-Identifier: GPL-3.0-or-later

//! The parts of H.264 the container needs: the `AVCDecoderConfigurationRecord`
//! (ISO/IEC 14496-15 5.3.3.1, the `avcC` payload WebCodecs hands out as
//! `decoderConfig.description`) and enough of the sequence parameter set
//! (H.264 7.3.2.1.1) to read the profile, level and cropped picture size.

use crate::MuxError;

/// A validated `AVCDecoderConfigurationRecord`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvcConfig {
    record: Vec<u8>,
    /// `AVCProfileIndication` (66 Baseline, 77 Main, 100 High).
    pub profile_idc: u8,
    /// `profile_compatibility` (the constraint flags byte).
    pub profile_compat: u8,
    /// `AVCLevelIndication` (`level * 10`, 31 for 3.1).
    pub level_idc: u8,
    /// Bytes in each NAL unit length prefix of a sample (1, 2 or 4).
    pub nal_length_size: u8,
    /// The first SPS, decoded; `None` when it uses syntax this parser does not
    /// cover (the picture size then comes from the config alone).
    pub sps: Option<SpsInfo>,
}

impl AvcConfig {
    /// Parses and validates a record. The bytes are kept verbatim for `avcC`.
    pub fn parse(record: &[u8]) -> Result<Self, MuxError> {
        let bad = MuxError::InvalidAvcc;
        if record.len() < 7 {
            return Err(bad("shorter than 7 bytes"));
        }
        if record[0] != 1 {
            return Err(bad("configurationVersion is not 1"));
        }
        let nal_length_size = (record[4] & 0b11) + 1;
        if nal_length_size == 3 {
            return Err(bad("lengthSizeMinusOne is 2"));
        }
        let mut at = 5;
        let sps_count = record[at] & 0x1f;
        at += 1;
        if sps_count == 0 {
            return Err(bad("no SPS"));
        }
        let mut first_sps = None;
        for _ in 0..sps_count {
            let nal = parameter_set(record, &mut at)?;
            if nal.first().map(|h| h & 0x1f) != Some(7) {
                return Err(bad("SPS entry is not NAL type 7"));
            }
            first_sps.get_or_insert(nal);
        }
        let pps_count = *record.get(at).ok_or(bad("truncated before PPS count"))?;
        at += 1;
        if pps_count == 0 {
            return Err(bad("no PPS"));
        }
        for _ in 0..pps_count {
            let nal = parameter_set(record, &mut at)?;
            if nal.first().map(|h| h & 0x1f) != Some(8) {
                return Err(bad("PPS entry is not NAL type 8"));
            }
        }
        // High-profile records may carry a chroma/bit-depth extension after
        // the PPS list; it is copied verbatim and not needed here.
        Ok(AvcConfig {
            record: record.to_vec(),
            profile_idc: record[1],
            profile_compat: record[2],
            level_idc: record[3],
            nal_length_size,
            sps: first_sps.and_then(SpsInfo::parse),
        })
    }

    /// The record bytes, as passed in.
    pub fn record(&self) -> &[u8] {
        &self.record
    }

    /// The RFC 6381 codec string (`avc1.640028`).
    pub fn codec_string(&self) -> String {
        format!(
            "avc1.{:02x}{:02x}{:02x}",
            self.profile_idc, self.profile_compat, self.level_idc
        )
    }
}

fn parameter_set<'a>(record: &'a [u8], at: &mut usize) -> Result<&'a [u8], MuxError> {
    let len_bytes = record
        .get(*at..*at + 2)
        .ok_or(MuxError::InvalidAvcc("truncated parameter set length"))?;
    let len = usize::from(u16::from_be_bytes([len_bytes[0], len_bytes[1]]));
    *at += 2;
    let nal = record
        .get(*at..*at + len)
        .ok_or(MuxError::InvalidAvcc("truncated parameter set"))?;
    *at += len;
    Ok(nal)
}

/// Fields of a sequence parameter set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpsInfo {
    /// `profile_idc`.
    pub profile_idc: u8,
    /// `level_idc`.
    pub level_idc: u8,
    /// `chroma_format_idc` (1 is 4:2:0).
    pub chroma_format_idc: u32,
    /// `bit_depth_luma_minus8 + 8`.
    pub bit_depth_luma: u32,
    /// Picture width after the cropping window.
    pub width: u32,
    /// Picture height after the cropping window.
    pub height: u32,
}

/// Profiles whose SPS carries `chroma_format_idc` and the bit depths.
const HIGH_PROFILES: [u8; 13] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135];

impl SpsInfo {
    /// Decodes an SPS NAL unit (header byte included, emulation prevention
    /// bytes still in). `None` for anything malformed or out of range.
    pub fn parse(nal: &[u8]) -> Option<SpsInfo> {
        if nal.first()? & 0x1f != 7 {
            return None;
        }
        let rbsp = unescape(&nal[1..]);
        let mut r = BitReader::new(&rbsp);
        let profile_idc = r.bits(8)? as u8;
        r.bits(8)?; // constraint_set flags, reserved_zero_2bits
        let level_idc = r.bits(8)? as u8;
        r.ue()?; // seq_parameter_set_id
        let mut chroma_format_idc = 1;
        let mut bit_depth_luma = 8;
        let mut separate_colour_plane = false;
        if HIGH_PROFILES.contains(&profile_idc) {
            chroma_format_idc = r.ue()?;
            if chroma_format_idc > 3 {
                return None;
            }
            if chroma_format_idc == 3 {
                separate_colour_plane = r.flag()?;
            }
            bit_depth_luma = r.ue()?.checked_add(8)?;
            r.ue()?; // bit_depth_chroma_minus8
            r.flag()?; // qpprime_y_zero_transform_bypass_flag
            if r.flag()? {
                // seq_scaling_matrix_present_flag
                let lists = if chroma_format_idc == 3 { 12 } else { 8 };
                for i in 0..lists {
                    if r.flag()? {
                        skip_scaling_list(&mut r, if i < 6 { 16 } else { 64 })?;
                    }
                }
            }
        }
        r.ue()?; // log2_max_frame_num_minus4
        match r.ue()? {
            0 => {
                r.ue()?; // log2_max_pic_order_cnt_lsb_minus4
            }
            1 => {
                r.flag()?; // delta_pic_order_always_zero_flag
                r.se()?; // offset_for_non_ref_pic
                r.se()?; // offset_for_top_to_bottom_field
                let cycle = r.ue()?;
                if cycle > 255 {
                    return None;
                }
                for _ in 0..cycle {
                    r.se()?;
                }
            }
            2 => {}
            _ => return None,
        }
        r.ue()?; // max_num_ref_frames
        r.flag()?; // gaps_in_frame_num_value_allowed_flag
        let width_mbs = r.ue()?.checked_add(1)?;
        let height_map_units = r.ue()?.checked_add(1)?;
        let frame_mbs_only = r.flag()?;
        if !frame_mbs_only {
            r.flag()?; // mb_adaptive_frame_field_flag
        }
        r.flag()?; // direct_8x8_inference_flag
        let (mut crop_l, mut crop_r, mut crop_t, mut crop_b) = (0, 0, 0, 0);
        if r.flag()? {
            crop_l = r.ue()?;
            crop_r = r.ue()?;
            crop_t = r.ue()?;
            crop_b = r.ue()?;
        }
        // H.264 7.4.2.1.1: crop units depend on ChromaArrayType.
        let chroma_array_type = if separate_colour_plane {
            0
        } else {
            chroma_format_idc
        };
        let (sub_w, sub_h) = match chroma_array_type {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        let field_factor = if frame_mbs_only { 1 } else { 2 };
        let crop_x = sub_w * crop_l.checked_add(crop_r)?;
        let crop_y = sub_h * field_factor * crop_t.checked_add(crop_b)?;
        let width = width_mbs.checked_mul(16)?.checked_sub(crop_x)?;
        let height = height_map_units
            .checked_mul(16 * field_factor)?
            .checked_sub(crop_y)?;
        if width == 0 || height == 0 {
            return None;
        }
        Some(SpsInfo {
            profile_idc,
            level_idc,
            chroma_format_idc,
            bit_depth_luma,
            width,
            height,
        })
    }
}

fn skip_scaling_list(r: &mut BitReader<'_>, size: u32) -> Option<()> {
    let (mut last, mut next) = (8i64, 8i64);
    for _ in 0..size {
        if next != 0 {
            let delta = i64::from(r.se()?);
            next = (last + delta + 256).rem_euclid(256);
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

/// Removes emulation prevention bytes (`00 00 03` becomes `00 00`).
fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &b in data {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, bit: 0 }
    }

    fn flag(&mut self) -> Option<bool> {
        let byte = *self.data.get(self.bit / 8)?;
        let set = byte & (0x80 >> (self.bit % 8)) != 0;
        self.bit += 1;
        Some(set)
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u32::from(self.flag()?);
        }
        Some(v)
    }

    /// Unsigned Exp-Golomb, at most 32 bits of value.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while !self.flag()? {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let rest = self.bits(zeros)?;
        ((1u64 << zeros) - 1 + u64::from(rest)).try_into().ok()
    }

    fn se(&mut self) -> Option<i32> {
        let k = i64::from(self.ue()?);
        let v = if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) };
        v.try_into().ok()
    }
}
