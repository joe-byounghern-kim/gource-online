// SPDX-License-Identifier: GPL-3.0-or-later

//! In-place edits for seekable sinks (REC-58, REC-59).

/// Overwrite `bytes` at absolute file offset `offset`. Patches never change
/// the file length and never overlap each other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    /// Absolute byte offset in the output file.
    pub offset: u64,
    /// Replacement bytes.
    pub bytes: Vec<u8>,
}

impl Patch {
    /// Applies the patch to a complete in-memory copy of the file.
    ///
    /// # Panics
    /// When the patch reaches past the end of `file`.
    pub fn apply(&self, file: &mut [u8]) {
        let at = usize::try_from(self.offset).expect("patch offset beyond address space");
        file[at..at + self.bytes.len()].copy_from_slice(&self.bytes);
    }
}

/// Applies every patch in order.
pub fn apply_patches(patches: &[Patch], file: &mut [u8]) {
    for p in patches {
        p.apply(file);
    }
}

/// The wire form handed to TypeScript (`take_patches()`): for each patch,
/// `offset` as u64 little-endian, `len` as u32 little-endian, then the bytes.
/// Appends to `out`.
pub fn encode_patches(patches: &[Patch], out: &mut Vec<u8>) {
    for p in patches {
        out.extend_from_slice(&p.offset.to_le_bytes());
        let len = u32::try_from(p.bytes.len()).expect("patch longer than 4 GiB");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&p.bytes);
    }
}
