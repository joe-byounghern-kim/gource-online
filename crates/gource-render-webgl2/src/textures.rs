// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Built-in textures. Gource grabs `file.png`, `user.png` and `beam.png`
//! through `TextureManager::grab(name, mipmaps = true, GL_CLAMP_TO_EDGE)`
//! (gource.cpp, user.cpp), so each gets a mipmap chain, trilinear
//! minification (`GL_LINEAR_MIPMAP_LINEAR`, `texturemanager.trilinear`),
//! linear magnification and clamp-to-edge wrapping (core/texture.cpp). Glyph
//! pages are created with `mipmaps = false`: linear/linear, clamp.
//!
//! The decoration atlas (`decorations.png`, port addition) is mipmapped the
//! same way, but only down to level [`DECO_TEXTURE_MAX_LEVEL`]: below that
//! the 4 px gutter between its cells would be gone and neighbours would
//! bleed in.

use gource_render::DECO_TEXTURE_MAX_LEVEL;
use wasm_bindgen::JsValue;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlTexture};

/// Straight-alpha RGBA8 pixels decoded by `build.rs`.
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: &'static [u8],
}

mod embedded {
    use super::RgbaImage;
    include!(concat!(env!("OUT_DIR"), "/textures.rs"));
}

pub struct Textures {
    pub white: WebGlTexture,
    pub file: WebGlTexture,
    pub user: WebGlTexture,
    pub beam: WebGlTexture,
    pub glyphs: WebGlTexture,
    pub deco: WebGlTexture,
}

impl Textures {
    pub fn new(gl: &Gl) -> Result<Self, JsValue> {
        // Rows are tightly packed RGBA; keep the WebGL unpack defaults.
        gl.pixel_storei(Gl::UNPACK_ALIGNMENT, 4);
        gl.pixel_storei(Gl::UNPACK_FLIP_Y_WEBGL, 0);
        gl.pixel_storei(Gl::UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
        let white = RgbaImage {
            width: 1,
            height: 1,
            pixels: &[255, 255, 255, 255],
        };
        let textures = Textures {
            white: image(gl, &white, false)?,
            file: image(gl, &embedded::FILE, true)?,
            user: image(gl, &embedded::USER, true)?,
            beam: image(gl, &embedded::BEAM, true)?,
            deco: deco_image(gl, &embedded::DECO)?,
            // Transparent until the atlas uploads its first page.
            glyphs: image(
                gl,
                &RgbaImage {
                    width: 1,
                    height: 1,
                    pixels: &[0; 4],
                },
                false,
            )?,
        };
        gl.bind_texture(Gl::TEXTURE_2D, None);
        Ok(textures)
    }

    /// Replace the glyph texture with the atlas canvas.
    pub fn upload_glyphs(&self, gl: &Gl, canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
        gl.bind_texture(Gl::TEXTURE_2D, Some(&self.glyphs));
        gl.tex_image_2d_with_u32_and_u32_and_html_canvas_element(
            Gl::TEXTURE_2D,
            0,
            Gl::RGBA as i32,
            Gl::RGBA,
            Gl::UNSIGNED_BYTE,
            canvas,
        )
    }
}

fn create(gl: &Gl) -> Result<WebGlTexture, JsValue> {
    let texture = gl
        .create_texture()
        .ok_or_else(|| JsValue::from_str("createTexture failed"))?;
    gl.bind_texture(Gl::TEXTURE_2D, Some(&texture));
    Ok(texture)
}

fn image(gl: &Gl, img: &RgbaImage, mipmaps: bool) -> Result<WebGlTexture, JsValue> {
    let texture = create(gl)?;
    gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
        Gl::TEXTURE_2D,
        0,
        Gl::RGBA as i32,
        img.width as i32,
        img.height as i32,
        0,
        Gl::RGBA,
        Gl::UNSIGNED_BYTE,
        Some(img.pixels),
    )?;
    if mipmaps {
        gl.generate_mipmap(Gl::TEXTURE_2D);
    }
    parameters(gl, mipmaps);
    Ok(texture)
}

/// The decoration atlas: mipmaps limited to [`DECO_TEXTURE_MAX_LEVEL`].
fn deco_image(gl: &Gl, img: &RgbaImage) -> Result<WebGlTexture, JsValue> {
    let texture = create(gl)?;
    gl.tex_parameteri(
        Gl::TEXTURE_2D,
        Gl::TEXTURE_MAX_LEVEL,
        DECO_TEXTURE_MAX_LEVEL,
    );
    gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
        Gl::TEXTURE_2D,
        0,
        Gl::RGBA as i32,
        img.width as i32,
        img.height as i32,
        0,
        Gl::RGBA,
        Gl::UNSIGNED_BYTE,
        Some(img.pixels),
    )?;
    gl.generate_mipmap(Gl::TEXTURE_2D);
    parameters(gl, true);
    Ok(texture)
}

/// `TextureResource::setDefaultFiltering` + `setWrapStyle(GL_CLAMP_TO_EDGE)`
/// on the bound texture.
fn parameters(gl: &Gl, mipmaps: bool) {
    let min = if mipmaps {
        Gl::LINEAR_MIPMAP_LINEAR
    } else {
        Gl::LINEAR
    };
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, min as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::LINEAR as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE as i32);
}
