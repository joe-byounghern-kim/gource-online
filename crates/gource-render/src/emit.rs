// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Vertex and command emission shared by every pass (`core/vbo.cpp`
//! `quadbuf::add`, `bloom.cpp` `bloombuf::add`).

use glam::{Vec2, Vec4};

use crate::{BloomVertex, DrawCommand, QuadVertex};

/// `quadbuf_default_texcoord`.
pub(crate) const DEFAULT_TEXCOORD: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

#[inline]
pub(crate) fn vertex(pos: Vec2, colour: Vec4, uv: [f32; 2]) -> QuadVertex {
    QuadVertex {
        pos: pos.to_array(),
        colour: colour.to_array(),
        uv,
    }
}

/// `quadbuf::add(textureid, pos, dims, colour, texcoord)`: corners
/// `pos`, `pos + (w, 0)`, `pos + dims`, `pos + (0, h)` with texcoords
/// `(x, y)`, `(z, y)`, `(z, w)`, `(x, w)`.
#[inline]
pub(crate) fn quad(out: &mut Vec<QuadVertex>, pos: Vec2, dims: Vec2, colour: Vec4, tc: [f32; 4]) {
    let c = colour.to_array();
    out.extend_from_slice(&[
        QuadVertex {
            pos: [pos.x, pos.y],
            colour: c,
            uv: [tc[0], tc[1]],
        },
        QuadVertex {
            pos: [pos.x + dims.x, pos.y],
            colour: c,
            uv: [tc[2], tc[1]],
        },
        QuadVertex {
            pos: [pos.x + dims.x, pos.y + dims.y],
            colour: c,
            uv: [tc[2], tc[3]],
        },
        QuadVertex {
            pos: [pos.x, pos.y + dims.y],
            colour: c,
            uv: [tc[0], tc[3]],
        },
    ]);
}

/// A quad with explicit corners in [`quad`]'s order (top-left, top-right,
/// bottom-right, bottom-left) and texcoords: for sheared sprites such as
/// the cape.
#[inline]
pub(crate) fn quad4(out: &mut Vec<QuadVertex>, corners: [Vec2; 4], colour: Vec4, tc: [f32; 4]) {
    let c = colour.to_array();
    let uv = [
        [tc[0], tc[1]],
        [tc[2], tc[1]],
        [tc[2], tc[3]],
        [tc[0], tc[3]],
    ];
    for (p, uv) in corners.into_iter().zip(uv) {
        out.push(QuadVertex {
            pos: p.to_array(),
            colour: c,
            uv,
        });
    }
}

/// `bloombuf::add`: same corner order as [`quad`], one texcoord for all four.
#[inline]
pub(crate) fn bloom_quad(
    out: &mut Vec<BloomVertex>,
    pos: Vec2,
    dims: Vec2,
    colour: Vec4,
    texcoord: [f32; 4],
) {
    let c = colour.to_array();
    let corners = [
        pos,
        pos + Vec2::new(dims.x, 0.0),
        pos + dims,
        pos + Vec2::new(0.0, dims.y),
    ];
    for p in corners {
        out.push(BloomVertex {
            pos: p.to_array(),
            colour: c,
            texcoord,
        });
    }
}

/// Number of whole quads in a vertex stream.
#[inline]
pub(crate) fn quads_in(len: usize) -> u32 {
    (len / 4) as u32
}

fn same_state(a: &DrawCommand, b: &DrawCommand) -> bool {
    a.stream == b.stream
        && a.program == b.program
        && a.space == b.space
        && a.offset == b.offset
        && a.texture == b.texture
        && a.blend == b.blend
}

/// Append `cmd` unless it is empty; extend the previous command instead when
/// it has the same state and the ranges are contiguous (drawing them in one
/// call is identical to drawing them one after the other).
pub(crate) fn push_command(commands: &mut Vec<DrawCommand>, cmd: DrawCommand) {
    if cmd.quad_count == 0 {
        return;
    }
    if let Some(last) = commands.last_mut()
        && same_state(last, &cmd)
        && last.first_quad + last.quad_count == cmd.first_quad
    {
        last.quad_count += cmd.quad_count;
        return;
    }
    commands.push(cmd);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Blend, Program, Space, Stream, TextureId};

    #[test]
    fn quad_vertex_order_matches_quadbuf_add() {
        let mut v = Vec::new();
        quad(
            &mut v,
            Vec2::new(1.0, 2.0),
            Vec2::new(10.0, 20.0),
            Vec4::ONE,
            [0.1, 0.2, 0.3, 0.4],
        );
        let pos: Vec<[f32; 2]> = v.iter().map(|q| q.pos).collect();
        let uv: Vec<[f32; 2]> = v.iter().map(|q| q.uv).collect();
        assert_eq!(pos, [[1.0, 2.0], [11.0, 2.0], [11.0, 22.0], [1.0, 22.0]]);
        assert_eq!(uv, [[0.1, 0.2], [0.3, 0.2], [0.3, 0.4], [0.1, 0.4]]);
    }

    #[test]
    fn quad4_matches_quad_for_a_rectangle() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let tc = [0.1, 0.2, 0.3, 0.4];
        quad(
            &mut a,
            Vec2::new(1.0, 2.0),
            Vec2::new(10.0, 20.0),
            Vec4::ONE,
            tc,
        );
        let corners = [
            Vec2::new(1.0, 2.0),
            Vec2::new(11.0, 2.0),
            Vec2::new(11.0, 22.0),
            Vec2::new(1.0, 22.0),
        ];
        quad4(&mut b, corners, Vec4::ONE, tc);
        assert_eq!(a, b);
    }

    #[test]
    fn commands_merge_only_when_contiguous_and_identical() {
        let base = DrawCommand {
            stream: Stream::Text,
            first_quad: 0,
            quad_count: 2,
            program: Program::Textured,
            space: Space::Screen,
            offset: [0.0, 0.0],
            texture: TextureId::Glyphs,
            blend: Blend::Alpha,
        };
        let mut cmds = Vec::new();
        push_command(&mut cmds, base);
        push_command(
            &mut cmds,
            DrawCommand {
                first_quad: 2,
                quad_count: 3,
                ..base
            },
        );
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].quad_count, 5);
        push_command(
            &mut cmds,
            DrawCommand {
                first_quad: 5,
                quad_count: 0,
                ..base
            },
        );
        assert_eq!(cmds.len(), 1);
        push_command(
            &mut cmds,
            DrawCommand {
                first_quad: 5,
                quad_count: 1,
                program: Program::Bloom,
                ..base
            },
        );
        assert_eq!(cmds.len(), 2);
    }
}
