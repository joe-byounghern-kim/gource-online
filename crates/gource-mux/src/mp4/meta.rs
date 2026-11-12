// SPDX-License-Identifier: GPL-3.0-or-later

//! Embedded recipe (REC-71): `moov/udta/meta` with an iTunes-style `ilst`
//! (handler `mdir`), which `ffprobe` and most players read as tags.

use super::boxes::{Put, close, open, open_full};

/// Largest recipe JSON that is embedded; a longer one is left out and
/// [`super::Mp4Writer::recipe_dropped`] is set, so the caller can fall back to
/// the link only (REC-71).
pub const RECIPE_MAX_BYTES: usize = 16 * 1024;

/// The `©too` value is `"<TOOL_PREFIX> <build>"`.
pub const TOOL_PREFIX: &str = "gource-web";

/// Tags to embed. Empty fields are left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    /// `©cmt`: one line of human text with the reproduce link.
    pub comment: String,
    /// The web build id; `©too` becomes `gource-web <build>` (`gource-web`
    /// alone when empty).
    pub build: String,
    /// Recipe JSON (UTF-8), the freeform item `----` with mean
    /// `org.gource-web` and name `recipe`.
    pub recipe: Vec<u8>,
}

/// Encodes a `udta` box. Returns the bytes and whether the recipe was dropped
/// for exceeding [`RECIPE_MAX_BYTES`].
pub(super) fn encode_udta(meta: &Metadata) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let udta = open(&mut buf, b"udta");
    let meta_box = open_full(&mut buf, b"meta", 0, 0);
    let hdlr = open_full(&mut buf, b"hdlr", 0, 0);
    buf.u32(0);
    buf.bytes(b"mdir");
    buf.bytes(b"appl");
    buf.u64(0);
    buf.u8(0); // empty name
    close(&mut buf, hdlr);

    let ilst = open(&mut buf, b"ilst");
    if !meta.comment.is_empty() {
        text_item(&mut buf, b"\xa9cmt", meta.comment.as_bytes());
    }
    let tool = if meta.build.is_empty() {
        TOOL_PREFIX.to_owned()
    } else {
        format!("{TOOL_PREFIX} {}", meta.build)
    };
    text_item(&mut buf, b"\xa9too", tool.as_bytes());
    let dropped = meta.recipe.len() > RECIPE_MAX_BYTES;
    if !meta.recipe.is_empty() && !dropped {
        let item = open(&mut buf, b"----");
        let mean = open_full(&mut buf, b"mean", 0, 0);
        buf.bytes(b"org.gource-web");
        close(&mut buf, mean);
        let name = open_full(&mut buf, b"name", 0, 0);
        buf.bytes(b"recipe");
        close(&mut buf, name);
        data_box(&mut buf, &meta.recipe);
        close(&mut buf, item);
    }
    close(&mut buf, ilst);
    close(&mut buf, meta_box);
    close(&mut buf, udta);
    (buf, dropped)
}

fn text_item(buf: &mut Vec<u8>, kind: &[u8; 4], text: &[u8]) {
    let item = open(buf, kind);
    data_box(buf, text);
    close(buf, item);
}

/// A `data` box holding UTF-8 text (well-known type 1, locale 0).
fn data_box(buf: &mut Vec<u8>, text: &[u8]) {
    let data = open(buf, b"data");
    buf.u32(1);
    buf.u32(0);
    buf.bytes(text);
    close(buf, data);
}
