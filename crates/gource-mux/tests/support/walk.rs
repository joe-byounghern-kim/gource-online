// SPDX-License-Identifier: GPL-3.0-or-later

//! An independent ISO BMFF reader for the tests (REC-61 a): walks the box tree,
//! checks that every box fits its parent exactly, and rebuilds the sample
//! table of a flat or fragmented file from what is on disk.

#![allow(dead_code)]

pub type Kind = [u8; 4];

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Kind,
    pub offset: usize,
    pub size: usize,
    pub header: usize,
    pub children: Vec<Node>,
}

impl Node {
    pub fn body<'a>(&self, file: &'a [u8]) -> &'a [u8] {
        &file[self.offset + self.header..self.offset + self.size]
    }

    pub fn child(&self, kind: &Kind) -> Option<&Node> {
        self.children.iter().find(|c| &c.kind == kind)
    }

    pub fn all(&self, kind: &Kind) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(move |c| &c.kind == kind)
    }

    /// Follows a path of child kinds.
    pub fn path(&self, path: &[&Kind]) -> Option<&Node> {
        path.iter().try_fold(self, |n, k| n.child(k))
    }

    pub fn kinds(&self) -> Vec<String> {
        self.children.iter().map(|c| name(&c.kind)).collect()
    }
}

pub fn name(kind: &Kind) -> String {
    kind.iter()
        .map(|&b| if b.is_ascii_graphic() { b as char } else { '?' })
        .collect()
}

/// Bytes between a container's header and its first child.
fn child_skip(kind: &Kind) -> Option<usize> {
    Some(match kind {
        b"moov" | b"trak" | b"mdia" | b"minf" | b"dinf" | b"stbl" | b"mvex" | b"moof" | b"traf"
        | b"mfra" | b"udta" | b"edts" | b"ilst" | b"----" | b"\xa9cmt" | b"\xa9too" => 0,
        b"meta" => 4,
        b"dref" | b"stsd" => 8,
        b"avc1" | b"avc3" => 78,
        _ => return None,
    })
}

fn read_header(data: &[u8], at: usize, end: usize) -> Result<(Kind, usize, usize), String> {
    if end - at < 8 {
        return Err(format!("box header at {at} truncated"));
    }
    let size32 = be32(&data[at..]);
    let kind: Kind = data[at + 4..at + 8].try_into().unwrap();
    let (size, header) = match size32 {
        0 => (end - at, 8),
        1 => {
            if end - at < 16 {
                return Err(format!("largesize header at {at} truncated"));
            }
            (be64(&data[at + 8..]) as usize, 16)
        }
        n => (n as usize, 8),
    };
    if size < header {
        return Err(format!("{} at {at}: size {size} below header", name(&kind)));
    }
    Ok((kind, size, header))
}

fn walk_range(data: &[u8], mut at: usize, end: usize) -> Result<Vec<Node>, String> {
    let mut nodes = Vec::new();
    while at < end {
        let (kind, size, header) = read_header(data, at, end)?;
        if at + size > end {
            return Err(format!(
                "{} at {at}: size {size} overruns its parent (ends {end})",
                name(&kind)
            ));
        }
        let children = match child_skip(&kind) {
            Some(skip) => {
                if header + skip > size {
                    return Err(format!("{} at {at} too small for its fields", name(&kind)));
                }
                walk_range(data, at + header + skip, at + size)?
            }
            None => Vec::new(),
        };
        nodes.push(Node {
            kind,
            offset: at,
            size,
            header,
            children,
        });
        at += size;
    }
    Ok(nodes)
}

/// Walks a whole file; every box must be complete and nest exactly.
pub fn walk(data: &[u8]) -> Result<Vec<Node>, String> {
    walk_range(data, 0, data.len())
}

/// Walks the complete top-level boxes of a possibly truncated file and stops
/// at the first incomplete one. Complete boxes must still be valid inside.
pub fn walk_prefix(data: &[u8]) -> Result<Vec<Node>, String> {
    let mut nodes = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let Ok((_, size, _)) = read_header(data, at, data.len()) else {
            break;
        };
        if at + size > data.len() {
            break;
        }
        nodes.extend(walk_range(data, at, at + size)?);
        at += size;
    }
    Ok(nodes)
}

pub fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
pub fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b[..4].try_into().unwrap())
}
pub fn be64(b: &[u8]) -> u64 {
    u64::from_be_bytes(b[..8].try_into().unwrap())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub offset: u64,
    pub size: u32,
    pub duration: u32,
    pub decode_time: u64,
    pub key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    pub sequence: u32,
    pub moof_offset: u64,
    pub base_time: u64,
    pub samples: usize,
}

#[derive(Debug, Default)]
pub struct Movie {
    pub top: Vec<String>,
    pub brand: String,
    pub header_version: u8,
    pub mvhd_timescale: u32,
    pub mvhd_duration: u64,
    pub tkhd_duration: u64,
    pub mdhd_timescale: u32,
    pub mdhd_duration: u64,
    pub mehd_duration: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub entry_width: u16,
    pub entry_height: u16,
    pub avcc: Vec<u8>,
    pub colr: Option<(u16, u16, u16, bool)>,
    pub pasp: Option<(u32, u32)>,
    pub co64: bool,
    pub samples: Vec<Sample>,
    pub fragments: Vec<Fragment>,
    pub tfra: Vec<(u64, u64)>,
    pub mfro_size: Option<u32>,
    pub tags: Vec<(String, Vec<u8>)>,
}

fn full_version(body: &[u8]) -> u8 {
    body[0]
}

/// Timescale and duration of `mvhd` or `mdhd`, version 0 or 1.
fn timescale_duration(body: &[u8]) -> (u32, u64) {
    if full_version(body) == 1 {
        (be32(&body[20..]), be64(&body[24..]))
    } else {
        (be32(&body[12..]), u64::from(be32(&body[16..])))
    }
}

/// Reads the movie. With `lenient`, a truncated tail is ignored and only
/// complete `moof`+`mdat` pairs count.
pub fn read_movie(file: &[u8], lenient: bool) -> Result<Movie, String> {
    let top = if lenient {
        walk_prefix(file)?
    } else {
        walk(file)?
    };
    let mut m = Movie {
        top: top.iter().map(|n| name(&n.kind)).collect(),
        ..Movie::default()
    };
    let ftyp = top
        .first()
        .filter(|n| &n.kind == b"ftyp")
        .ok_or("ftyp is not first")?;
    m.brand = name(ftyp.body(file)[..4].try_into().unwrap());
    let moov = top.iter().find(|n| &n.kind == b"moov").ok_or("no moov")?;
    if top.iter().filter(|n| &n.kind == b"moov").count() != 1 {
        return Err("more than one moov".into());
    }

    let mvhd = moov.child(b"mvhd").ok_or("no mvhd")?.body(file);
    m.header_version = full_version(mvhd);
    (m.mvhd_timescale, m.mvhd_duration) = timescale_duration(mvhd);

    let trak = moov.child(b"trak").ok_or("no trak")?;
    let tkhd = trak.child(b"tkhd").ok_or("no tkhd")?.body(file);
    if be32(tkhd) & 0xffffff != 3 {
        return Err("tkhd flags are not 3".into());
    }
    m.tkhd_duration = if full_version(tkhd) == 1 {
        be64(&tkhd[28..])
    } else {
        u64::from(be32(&tkhd[20..]))
    };
    m.width = be32(&tkhd[tkhd.len() - 8..]) >> 16;
    m.height = be32(&tkhd[tkhd.len() - 4..]) >> 16;

    let mdia = trak.child(b"mdia").ok_or("no mdia")?;
    let mdhd = mdia.child(b"mdhd").ok_or("no mdhd")?.body(file);
    (m.mdhd_timescale, m.mdhd_duration) = timescale_duration(mdhd);
    let hdlr = mdia.child(b"hdlr").ok_or("no hdlr")?.body(file);
    if &hdlr[8..12] != b"vide" {
        return Err("handler is not vide".into());
    }
    let minf = mdia.child(b"minf").ok_or("no minf")?;
    minf.child(b"vmhd").ok_or("no vmhd")?;
    minf.path(&[b"dinf", b"dref", b"url "])
        .ok_or("no dref url")?;
    let stbl = minf.child(b"stbl").ok_or("no stbl")?;
    let stsd = stbl.child(b"stsd").ok_or("no stsd")?;
    if be32(&stsd.body(file)[4..]) != 1 || stsd.children.len() != 1 {
        return Err("stsd must hold one entry".into());
    }
    let avc1 = stsd.child(b"avc1").ok_or("no avc1")?;
    let entry = &file[avc1.offset + 8..];
    m.entry_width = be16(&entry[24..]);
    m.entry_height = be16(&entry[26..]);
    m.avcc = avc1.child(b"avcC").ok_or("no avcC")?.body(file).to_vec();
    if let Some(colr) = avc1.child(b"colr") {
        let b = colr.body(file);
        if &b[..4] != b"nclx" || b.len() != 11 {
            return Err("colr is not nclx".into());
        }
        m.colr = Some((
            be16(&b[4..]),
            be16(&b[6..]),
            be16(&b[8..]),
            b[10] & 0x80 != 0,
        ));
    }
    if let Some(pasp) = avc1.child(b"pasp") {
        let b = pasp.body(file);
        m.pasp = Some((be32(b), be32(&b[4..])));
    }

    if let Some(udta) = moov.child(b"udta") {
        let meta = udta.child(b"meta").ok_or("udta without meta")?;
        let hdlr = meta.child(b"hdlr").ok_or("meta without hdlr")?.body(file);
        if &hdlr[8..12] != b"mdir" {
            return Err("meta handler is not mdir".into());
        }
        for item in &meta.child(b"ilst").ok_or("meta without ilst")?.children {
            let data = item
                .child(b"data")
                .ok_or("ilst item without data")?
                .body(file);
            if be32(data) != 1 {
                return Err("data is not UTF-8".into());
            }
            let mut key = name(&item.kind);
            if &item.kind == b"----" {
                let mean = item.child(b"mean").ok_or("---- without mean")?.body(file);
                let nm = item.child(b"name").ok_or("---- without name")?.body(file);
                key = format!(
                    "----:{}:{}",
                    String::from_utf8_lossy(&mean[4..]),
                    String::from_utf8_lossy(&nm[4..])
                );
            }
            m.tags.push((key, data[8..].to_vec()));
        }
    }

    let stts = table(stbl, b"stts", file, 8)?;
    let stsz_body = stbl.child(b"stsz").ok_or("no stsz")?.body(file);
    let stsc = table(stbl, b"stsc", file, 12)?;
    let (chunk_offsets, co64) = match (stbl.child(b"stco"), stbl.child(b"co64")) {
        (Some(_), None) => (table(stbl, b"stco", file, 4)?, false),
        (None, Some(_)) => (table(stbl, b"co64", file, 8)?, true),
        _ => return Err("need exactly one of stco and co64".into()),
    };
    m.co64 = co64;
    let sample_count = be32(&stsz_body[8..]) as usize;

    if let Some(mvex) = moov.child(b"mvex") {
        if sample_count != 0 || !stts.is_empty() || !stsc.is_empty() || !chunk_offsets.is_empty() {
            return Err("fragmented file with non-empty sample tables".into());
        }
        if let Some(mehd) = mvex.child(b"mehd") {
            m.mehd_duration = Some(be64(&mehd.body(file)[4..]));
        }
        let trex = mvex.child(b"trex").ok_or("mvex without trex")?.body(file);
        if be32(&trex[4..]) != 1 {
            return Err("trex is not for track 1".into());
        }
        read_fragments(file, &top, &mut m, lenient)?;
    } else {
        read_flat(
            file,
            stbl,
            &stts,
            stsz_body,
            &stsc,
            &chunk_offsets,
            &top,
            &mut m,
        )?;
    }
    Ok(m)
}

/// Rows of a full-box table with a 32-bit count, `width` bytes each.
fn table<'a>(
    stbl: &Node,
    kind: &Kind,
    file: &'a [u8],
    width: usize,
) -> Result<Vec<&'a [u8]>, String> {
    let body = stbl
        .child(kind)
        .ok_or_else(|| format!("no {}", name(kind)))?
        .body(file);
    let n = be32(&body[4..]) as usize;
    if body.len() != 8 + n * width {
        return Err(format!("{} length does not match its count", name(kind)));
    }
    Ok((0..n)
        .map(|i| &body[8 + i * width..8 + (i + 1) * width])
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn read_flat(
    file: &[u8],
    stbl: &Node,
    stts: &[&[u8]],
    stsz: &[u8],
    stsc: &[&[u8]],
    chunks: &[&[u8]],
    top: &[Node],
    m: &mut Movie,
) -> Result<(), String> {
    let fixed = be32(&stsz[4..]);
    let n = be32(&stsz[8..]) as usize;
    let sizes: Vec<u32> = if fixed != 0 {
        vec![fixed; n]
    } else {
        if stsz.len() != 12 + 4 * n {
            return Err("stsz length does not match its count".into());
        }
        (0..n).map(|i| be32(&stsz[12 + 4 * i..])).collect()
    };
    let mut durations = Vec::with_capacity(n);
    for row in stts {
        for _ in 0..be32(row) {
            durations.push(be32(&row[4..]));
        }
    }
    if durations.len() != n {
        return Err(format!("stts covers {} samples, stsz {n}", durations.len()));
    }
    // Without stss every sample is a sync sample.
    let mut key = vec![stbl.child(b"stss").is_none(); n];
    if stbl.child(b"stss").is_some() {
        let mut prev = 0;
        for row in table(stbl, b"stss", file, 4)? {
            let k = be32(row) as usize;
            if k <= prev || k > n {
                return Err("stss not increasing or out of range".into());
            }
            key[k - 1] = true;
            prev = k;
        }
    }
    let offsets: Vec<u64> = chunks
        .iter()
        .map(|r| {
            if r.len() == 8 {
                be64(r)
            } else {
                u64::from(be32(r))
            }
        })
        .collect();
    // stsc: runs of chunks with the same samples-per-chunk.
    let mut per_chunk = Vec::with_capacity(offsets.len());
    for (i, row) in stsc.iter().enumerate() {
        let first = be32(row) as usize;
        let spc = be32(&row[4..]);
        let next = stsc
            .get(i + 1)
            .map_or(offsets.len() + 1, |r| be32(r) as usize);
        if first == 0 || next <= first {
            return Err("stsc first_chunk not increasing".into());
        }
        for _ in first..next {
            per_chunk.push(spc);
        }
    }
    if per_chunk.len() != offsets.len() {
        return Err("stsc does not cover every chunk".into());
    }
    let mut s = 0;
    let mut time = 0u64;
    for (c, &spc) in per_chunk.iter().enumerate() {
        let mut at = offsets[c];
        for _ in 0..spc {
            if s >= n {
                return Err("chunks hold more samples than stsz".into());
            }
            m.samples.push(Sample {
                offset: at,
                size: sizes[s],
                duration: durations[s],
                decode_time: time,
                key: key[s],
            });
            at += u64::from(sizes[s]);
            time += u64::from(durations[s]);
            s += 1;
        }
    }
    if s != n {
        return Err("chunks hold fewer samples than stsz".into());
    }
    // Every sample must lie inside an mdat payload.
    let mdats: Vec<(u64, u64)> = top
        .iter()
        .filter(|b| &b.kind == b"mdat")
        .map(|b| ((b.offset + b.header) as u64, (b.offset + b.size) as u64))
        .collect();
    for smp in &m.samples {
        let end = smp.offset + u64::from(smp.size);
        if !mdats.iter().any(|&(a, b)| smp.offset >= a && end <= b) {
            return Err(format!("sample at {} outside every mdat", smp.offset));
        }
    }
    Ok(())
}

fn read_fragments(file: &[u8], top: &[Node], m: &mut Movie, lenient: bool) -> Result<(), String> {
    let mut time = 0u64;
    let mut i = 0;
    while i < top.len() {
        let node = &top[i];
        match &node.kind {
            b"moof" => {
                let Some(mdat) = top.get(i + 1).filter(|n| &n.kind == b"mdat") else {
                    if lenient {
                        break;
                    }
                    return Err("moof not followed by mdat".into());
                };
                let seq = be32(&node.child(b"mfhd").ok_or("no mfhd")?.body(file)[4..]);
                let traf = node.child(b"traf").ok_or("no traf")?;
                let tfhd = traf.child(b"tfhd").ok_or("no tfhd")?.body(file);
                if be32(tfhd) & 0xffffff != 0x02_0000 || be32(&tfhd[4..]) != 1 {
                    return Err("tfhd is not default-base-is-moof for track 1".into());
                }
                let tfdt = traf.child(b"tfdt").ok_or("no tfdt")?.body(file);
                let base = if full_version(tfdt) == 1 {
                    be64(&tfdt[4..])
                } else {
                    u64::from(be32(&tfdt[4..]))
                };
                if base != time {
                    return Err(format!("tfdt {base} does not continue at {time}"));
                }
                let trun = traf.child(b"trun").ok_or("no trun")?.body(file);
                if be32(trun) & 0xffffff != 0x701 {
                    return Err("unexpected trun flags".into());
                }
                let count = be32(&trun[4..]) as usize;
                if trun.len() != 12 + 12 * count {
                    return Err("trun length does not match its count".into());
                }
                let data_offset = be32(&trun[8..]) as u64;
                let mut at = node.offset as u64 + data_offset;
                if at != (mdat.offset + mdat.header) as u64 {
                    return Err("trun data_offset does not point at the mdat payload".into());
                }
                let first = m.samples.len();
                for k in 0..count {
                    let e = &trun[12 + 12 * k..];
                    let (duration, size, flags) = (be32(e), be32(&e[4..]), be32(&e[8..]));
                    m.samples.push(Sample {
                        offset: at,
                        size,
                        duration,
                        decode_time: time,
                        key: flags & 0x0001_0000 == 0,
                    });
                    at += u64::from(size);
                    time += u64::from(duration);
                }
                if at != (mdat.offset + mdat.size) as u64 {
                    return Err("samples do not fill the mdat exactly".into());
                }
                m.fragments.push(Fragment {
                    sequence: seq,
                    moof_offset: node.offset as u64,
                    base_time: base,
                    samples: m.samples.len() - first,
                });
                i += 2;
                continue;
            }
            b"mfra" => {
                let tfra = node.child(b"tfra").ok_or("no tfra")?.body(file);
                if full_version(tfra) != 1 || be32(&tfra[8..]) != 0 {
                    return Err("tfra is not version 1 with 1-byte numbers".into());
                }
                let n = be32(&tfra[12..]) as usize;
                if tfra.len() != 16 + 19 * n {
                    return Err("tfra length does not match its count".into());
                }
                for k in 0..n {
                    let e = &tfra[16 + 19 * k..];
                    m.tfra.push((be64(e), be64(&e[8..])));
                }
                let mfro = node.child(b"mfro").ok_or("no mfro")?.body(file);
                m.mfro_size = Some(be32(&mfro[4..]));
            }
            b"mdat" if !lenient => return Err("mdat without moof".into()),
            _ => {}
        }
        i += 1;
    }
    Ok(())
}
