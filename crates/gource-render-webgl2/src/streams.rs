// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Vertex streams (one VAO + one growable `ARRAY_BUFFER` each) and the shared
//! quad index buffer. Uploads are `bufferSubData` calls on a zero-copy
//! `Float32Array` view over the frame's `Vec`; storage grows geometrically and
//! is never shrunk, so steady-state frames allocate nothing.

use gource_render::{BloomVertex, QuadVertex};
use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext as Gl, WebGlBuffer, WebGlVertexArrayObject};

use crate::shaders::{ATTR_COLOUR, ATTR_POS, ATTR_UV};

/// Smallest buffer allocation, in bytes.
const MIN_VERTEX_BYTES: u32 = 64 * 1024;
/// Smallest index buffer, in quads.
const MIN_INDEX_QUADS: u32 = 4096;
/// Largest single `bufferSubData`, in `f32`s (4 MiB). Chrome copies each
/// upload through its command buffer's transfer memory; one 15 MB call on
/// a 100k-file frame took 7.8 ms with ANGLE's Vulkan backend and 6.4 ms in
/// 4 MiB pieces, with the GL backend unchanged (3.3 ms) (PERFORMANCE.md).
const UPLOAD_CHUNK_FLOATS: usize = 1024 * 1024;

/// Vertex types that are a flat run of `f32`s.
///
/// # Safety
/// Implementors must be `#[repr(C)]` and contain only `f32` fields, with no
/// padding, so a slice of them can be viewed as `FLOATS * len` `f32`s.
pub unsafe trait FloatVertex: Copy {
    const FLOATS: usize;
}

// SAFETY: `QuadVertex` is `#[repr(C)]` with 8 `f32`s (32 bytes, no padding).
unsafe impl FloatVertex for QuadVertex {
    const FLOATS: usize = 8;
}

// SAFETY: `BloomVertex` is `#[repr(C)]` with 10 `f32`s (40 bytes, no padding).
unsafe impl FloatVertex for BloomVertex {
    const FLOATS: usize = 10;
}

const _: () = assert!(size_of::<QuadVertex>() == 4 * QuadVertex::FLOATS);
const _: () = assert!(size_of::<BloomVertex>() == 4 * BloomVertex::FLOATS);

/// Attribute layout of one stream: `(location, components, byte offset)`.
type Layout = [(u32, i32, i32); 3];

/// `QuadVertex`: pos 2f @0, colour 4f @8, uv 2f @24, stride 32.
pub const QUAD_LAYOUT: (Layout, i32) = (
    [(ATTR_POS, 2, 0), (ATTR_COLOUR, 4, 8), (ATTR_UV, 2, 24)],
    32,
);
/// `BloomVertex`: pos 2f @0, colour 4f @8, texcoord 4f @24, stride 40.
pub const BLOOM_LAYOUT: (Layout, i32) = (
    [(ATTR_POS, 2, 0), (ATTR_COLOUR, 4, 8), (ATTR_UV, 4, 24)],
    40,
);

/// One VAO and its vertex buffer.
pub struct VertexStream {
    pub vao: WebGlVertexArrayObject,
    buffer: WebGlBuffer,
    capacity: u32,
    /// Vertices uploaded this frame.
    pub len: u32,
}

impl VertexStream {
    /// Create the VAO with `layout`, bound to `indices` as its element buffer.
    pub fn new(
        gl: &Gl,
        (layout, stride): (Layout, i32),
        indices: &IndexBuffer,
    ) -> Result<Self, JsValue> {
        let vao = gl
            .create_vertex_array()
            .ok_or_else(|| JsValue::from_str("createVertexArray failed"))?;
        let buffer = gl
            .create_buffer()
            .ok_or_else(|| JsValue::from_str("createBuffer failed"))?;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&buffer));
        for (location, components, offset) in layout {
            gl.enable_vertex_attrib_array(location);
            gl.vertex_attrib_pointer_with_i32(
                location,
                components,
                Gl::FLOAT,
                false,
                stride,
                offset,
            );
        }
        gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&indices.buffer));
        gl.bind_vertex_array(None);
        gl.bind_buffer(Gl::ARRAY_BUFFER, None);
        Ok(VertexStream {
            vao,
            buffer,
            capacity: 0,
            len: 0,
        })
    }

    /// Upload `vertices`, returning the number of bytes sent.
    pub fn upload<V: FloatVertex>(&mut self, gl: &Gl, vertices: &[V]) -> u32 {
        self.len = vertices.len() as u32;
        if vertices.is_empty() {
            return 0;
        }
        let bytes = size_of_val(vertices) as u32;
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.buffer));
        if bytes > self.capacity {
            self.capacity = bytes
                .max(self.capacity.saturating_mul(2))
                .max(MIN_VERTEX_BYTES);
            gl.buffer_data_with_i32(Gl::ARRAY_BUFFER, self.capacity as i32, Gl::DYNAMIC_DRAW);
        }
        // SAFETY: `V: FloatVertex` guarantees the slice is `FLOATS * len`
        // contiguous `f32`s.
        let floats = unsafe {
            std::slice::from_raw_parts(vertices.as_ptr().cast::<f32>(), vertices.len() * V::FLOATS)
        };
        for (i, chunk) in floats.chunks(UPLOAD_CHUNK_FLOATS).enumerate() {
            let offset = (i * UPLOAD_CHUNK_FLOATS * 4) as i32;
            // SAFETY: the view aliases wasm memory, which stays valid because
            // nothing allocates between creating it and the GL call.
            unsafe {
                let view = js_sys::Float32Array::view(chunk);
                gl.buffer_sub_data_with_i32_and_array_buffer_view(Gl::ARRAY_BUFFER, offset, &view);
            }
        }
        gl.bind_buffer(Gl::ARRAY_BUFFER, None);
        bytes
    }
}

/// The shared `u32` index buffer: `(0,1,2, 0,2,3)` per quad.
pub struct IndexBuffer {
    pub buffer: WebGlBuffer,
    quads: u32,
}

impl IndexBuffer {
    pub fn new(gl: &Gl) -> Result<Self, JsValue> {
        let buffer = gl
            .create_buffer()
            .ok_or_else(|| JsValue::from_str("createBuffer failed"))?;
        Ok(IndexBuffer { buffer, quads: 0 })
    }

    /// Make room for `quads` quads (rebuilt and re-uploaded only on growth).
    /// Returns the bytes uploaded.
    pub fn reserve(&mut self, gl: &Gl, quads: u32) -> u32 {
        if quads <= self.quads {
            return 0;
        }
        let quads = quads.max(self.quads.saturating_mul(2)).max(MIN_INDEX_QUADS);
        let indices: Vec<u32> = (0..quads)
            .flat_map(|q| {
                let v = q * 4;
                [v, v + 1, v + 2, v, v + 2, v + 3]
            })
            .collect();
        // Bind on the default VAO so no stream's element binding changes.
        gl.bind_vertex_array(None);
        gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&self.buffer));
        // SAFETY: the view aliases `indices`, alive and unmoved for the call.
        unsafe {
            let view = js_sys::Uint32Array::view(&indices);
            gl.buffer_data_with_array_buffer_view(Gl::ELEMENT_ARRAY_BUFFER, &view, Gl::STATIC_DRAW);
        }
        gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, None);
        self.quads = quads;
        size_of_val(indices.as_slice()) as u32
    }
}
