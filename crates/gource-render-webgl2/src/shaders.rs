// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! GLSL ES 3.00 ports of Gource's programs (`data/shaders/*`) plus the
//! fixed-function `texture * colour` path, and their uniform locations.
//!
//! Every vertex shader clamps the vertex colour to [0, 1], as legacy GL does
//! with `GL_CLAMP_VERTEX_COLOR` (on by default) for `gl_FrontColor` and the
//! fixed-function colour: Gource's bloom colours exceed 1 with a high
//! `--bloom-intensity`, and label alphas reach 3 (`FXFont` alpha times
//! fades), which WebGL would otherwise interpolate unclamped.
//!
//! Every vertex shader adds `u_offset` to the vertex position before
//! `u_matrix` (Gource's `glTranslatef` shadow offsets) and scales texture
//! coordinates by `u_uv_scale` (the glyph atlas hands out coordinates in units
//! of its base size so they stay valid when it grows; see `atlas.rs`).

use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext as Gl, WebGlProgram, WebGlShader, WebGlUniformLocation};

/// Attribute locations shared by both vertex layouts.
pub const ATTR_POS: u32 = 0;
pub const ATTR_COLOUR: u32 = 1;
pub const ATTR_UV: u32 = 2;

/// Vertex shader for `QuadVertex` streams (shadow.vert / text.vert are
/// `ftransform()` pass-throughs).
const QUAD_VERT: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 a_colour;
layout(location = 2) in vec2 a_uv;
uniform mat4 u_matrix;
uniform vec2 u_offset;
uniform vec2 u_uv_scale;
out vec4 v_colour;
out vec2 v_uv;
void main() {
    v_colour = clamp(a_colour, 0.0, 1.0);
    v_uv = a_uv * u_uv_scale;
    gl_Position = u_matrix * vec4(a_pos + u_offset, 0.0, 1.0);
}
"#;

/// Fixed-function `GL_MODULATE`.
const TEXTURED_FRAG: &str = r#"#version 300 es
precision highp float;
uniform sampler2D u_tex;
in vec4 v_colour;
in vec2 v_uv;
out vec4 o_colour;
void main() {
    o_colour = texture(u_tex, v_uv) * v_colour;
}
"#;

/// `data/shaders/shadow.frag`.
const SHADOW_FRAG: &str = r#"#version 300 es
precision highp float;
uniform sampler2D u_tex;
uniform float u_strength;
in vec4 v_colour;
in vec2 v_uv;
out vec4 o_colour;
void main() {
    vec4 colour = texture(u_tex, v_uv);
    o_colour = vec4(0.0, 0.0, 0.0, v_colour.w * colour.w * u_strength);
}
"#;

/// `data/shaders/text.frag`; `u_texel` is the (already uv-scaled) texel size.
const TEXT_FRAG: &str = r#"#version 300 es
precision highp float;
uniform sampler2D u_tex;
uniform float u_strength;
uniform vec2 u_texel;
in vec4 v_colour;
in vec2 v_uv;
out vec4 o_colour;
void main() {
    float colour_alpha = texture(u_tex, v_uv).w;
    float shadow_alpha = texture(u_tex, v_uv - u_texel).w * u_strength;

    float combined_alpha = 1.0 - (1.0 - shadow_alpha) * (1.0 - colour_alpha);

    if (combined_alpha > 0.0) colour_alpha /= combined_alpha;

    o_colour = v_colour * vec4(vec3(colour_alpha), combined_alpha);
}
"#;

/// `data/shaders/bloom.vert`: `pos = gl_Vertex.xyz - gl_MultiTexCoord0.yzw`
/// (the untranslated vertex, as `gl_Vertex` is in the C++).
const BLOOM_VERT: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 a_colour;
layout(location = 2) in vec4 a_texcoord;
uniform mat4 u_matrix;
uniform vec2 u_offset;
out vec3 v_pos;
out vec4 v_texcoord;
out vec4 v_colour;
void main() {
    v_pos = vec3(a_pos, 0.0) - a_texcoord.yzw;
    v_texcoord = a_texcoord;
    v_colour = clamp(a_colour, 0.0, 1.0);
    gl_Position = u_matrix * vec4(a_pos + u_offset, 0.0, 1.0);
}
"#;

/// `data/shaders/bloom.frag`. The second `smoothstep` has `edge0 > edge1`,
/// which GLSL leaves undefined; `step_smooth` spells out the Hermite formula
/// that desktop drivers evaluate, so every implementation gives Gource's look.
const BLOOM_FRAG: &str = r#"#version 300 es
precision highp float;
in vec3 v_pos;
in vec4 v_texcoord;
in vec4 v_colour;
out vec4 o_colour;
float step_smooth(float edge0, float edge1, float x) {
    float t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
void main() {
    vec3 pos = v_pos;
    float r = fract(sin(dot(pos.xy, vec2(11.3713, 67.3219))) * 2351.3718);

    float offset = (0.5 - r) * v_texcoord.x * 0.045;

    float intensity = min(1.0, cos((length(pos * 2.0) + offset) / v_texcoord.x));
    float gradient  = intensity * step_smooth(0.0, 2.0, intensity);

    gradient *= step_smooth(1.0, 0.67 + r * 0.33, 1.0 - intensity);

    o_colour = v_colour * gradient;
}
"#;

/// Index into the program table.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProgramKind {
    Textured = 0,
    Shadow = 1,
    Bloom = 2,
    Text = 3,
}

pub const PROGRAM_COUNT: usize = 4;

/// A linked program, its uniform locations and the uniform values last set
/// on it (uniforms are per-program state in GL).
pub struct ProgramState {
    pub program: WebGlProgram,
    pub u_matrix: Option<WebGlUniformLocation>,
    pub u_offset: Option<WebGlUniformLocation>,
    pub u_uv_scale: Option<WebGlUniformLocation>,
    pub u_strength: Option<WebGlUniformLocation>,
    pub u_texel: Option<WebGlUniformLocation>,
    pub cache: UniformCache,
}

/// Last uniform values, `None` when unknown.
#[derive(Copy, Clone, Debug, Default)]
pub struct UniformCache {
    /// Which matrix (by space) is loaded; reset every frame because the
    /// world matrix changes.
    pub matrix: Option<gource_render::Space>,
    pub offset: Option<[f32; 2]>,
    pub uv_scale: Option<[f32; 2]>,
    pub strength: Option<f32>,
    pub texel: Option<[f32; 2]>,
}

/// Compile and link all programs, in [`ProgramKind`] order.
pub fn create_programs(gl: &Gl) -> Result<[ProgramState; PROGRAM_COUNT], JsValue> {
    Ok([
        link(gl, QUAD_VERT, TEXTURED_FRAG, "textured")?,
        link(gl, QUAD_VERT, SHADOW_FRAG, "shadow")?,
        link(gl, BLOOM_VERT, BLOOM_FRAG, "bloom")?,
        link(gl, QUAD_VERT, TEXT_FRAG, "text")?,
    ])
}

fn link(gl: &Gl, vert: &str, frag: &str, name: &str) -> Result<ProgramState, JsValue> {
    let vs = compile(gl, Gl::VERTEX_SHADER, vert, name)?;
    let fs = compile(gl, Gl::FRAGMENT_SHADER, frag, name)?;
    let program = gl
        .create_program()
        .ok_or_else(|| JsValue::from_str("createProgram failed"))?;
    gl.attach_shader(&program, &vs);
    gl.attach_shader(&program, &fs);
    gl.link_program(&program);
    gl.delete_shader(Some(&vs));
    gl.delete_shader(Some(&fs));
    if !gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
        && !gl.is_context_lost()
    {
        let log = gl.get_program_info_log(&program).unwrap_or_default();
        return Err(JsValue::from_str(&format!("{name} program link: {log}")));
    }
    let loc = |n: &str| gl.get_uniform_location(&program, n);
    let state = ProgramState {
        u_matrix: loc("u_matrix"),
        u_offset: loc("u_offset"),
        u_uv_scale: loc("u_uv_scale"),
        u_strength: loc("u_strength"),
        u_texel: loc("u_texel"),
        cache: UniformCache::default(),
        program,
    };
    // Every sampler reads texture unit 0.
    gl.use_program(Some(&state.program));
    if let Some(tex) = gl.get_uniform_location(&state.program, "u_tex") {
        gl.uniform1i(Some(&tex), 0);
    }
    gl.use_program(None);
    Ok(state)
}

fn compile(gl: &Gl, kind: u32, source: &str, name: &str) -> Result<WebGlShader, JsValue> {
    let shader = gl
        .create_shader(kind)
        .ok_or_else(|| JsValue::from_str("createShader failed"))?;
    gl.shader_source(&shader, source);
    gl.compile_shader(&shader);
    if !gl
        .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
        && !gl.is_context_lost()
    {
        let log = gl.get_shader_info_log(&shader).unwrap_or_default();
        gl.delete_shader(Some(&shader));
        let stage = if kind == Gl::VERTEX_SHADER {
            "vertex"
        } else {
            "fragment"
        };
        return Err(JsValue::from_str(&format!("{name} {stage} shader: {log}")));
    }
    Ok(shader)
}
