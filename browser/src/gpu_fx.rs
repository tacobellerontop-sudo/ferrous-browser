//! GPU effects drawn inside egui paint callbacks: things egui's own triangle
//! renderer cannot express.
//!
//! - [`Glass`]: real frosted glass behind the chrome panel. The page pixels
//!   under the panel are copied out of the window's framebuffer at quarter
//!   resolution, Gaussian-blurred in two separable passes, and drawn back under
//!   the panel through a rounded-rectangle mask. egui can only draw flat
//!   translucent fills, which tint the page but never soften it.
//! - [`EdgeGlow`]: a soft accent light that sweeps around the page card while a
//!   page loads, computed per pixel from a signed distance to the card's
//!   rounded rectangle.
//!
//! Both are a handful of draw calls on a few thousand pixels, so their GPU cost
//! is negligible next to compositing the page itself.
//!
//! # GL state
//!
//! egui calls these between its own draws and afterwards restores its program,
//! blend state, viewport, scissor, vertex array and texture unit — but **not**
//! the framebuffer binding, so every effect rebinds the framebuffer it found.
//!
//! All resources are created lazily on first use and kept behind
//! `Arc<Mutex<_>>`, because egui requires paint callbacks to be `Send + Sync`.
//! If a shader fails to compile the effect quietly does nothing: a browser
//! without frosted glass is fine, one that fails every frame is not.

use std::sync::{Arc, Mutex};

use egui_glow::glow::{self, HasContext};

/// Compile and link a program from GLSL ES 3.00 sources, logging and returning
/// `None` on failure.
pub fn compile_program(gl: &glow::Context, vertex: &str, fragment: &str) -> Option<glow::Program> {
    // SAFETY: plain GL object creation on the current context. Every failure
    // path deletes what it created.
    unsafe {
        let program = gl.create_program().ok()?;
        let mut shaders = Vec::new();
        for (kind, source) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
            let Ok(shader) = gl.create_shader(kind) else {
                gl.delete_program(program);
                return None;
            };
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                log::error!("gpu_fx: shader failed: {}", gl.get_shader_info_log(shader));
                gl.delete_shader(shader);
                gl.delete_program(program);
                return None;
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            log::error!("gpu_fx: link failed: {}", gl.get_program_info_log(program));
            gl.delete_program(program);
            return None;
        }
        Some(program)
    }
}

/// A quad covering the current viewport, generated from the vertex index so no
/// vertex buffer is needed. Shared by every effect.
pub const FULL_VIEWPORT_VERTEX: &str = r#"#version 300 es
const vec2 CORNERS[4] = vec2[4](vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(1.0, 1.0));
void main() {
    gl_Position = vec4(CORNERS[gl_VertexID], 0.0, 1.0);
}
"#;

/// Signed distance from `p` to a rounded rectangle centred on the origin with
/// half-size `half` and per-corner radii `r` (top-right, bottom-right,
/// bottom-left, top-left in GL's y-up space). Negative inside.
const ROUNDED_RECT_SDF: &str = r#"
float rounded_rect(vec2 p, vec2 half_size, vec4 r) {
    float radius = p.x > 0.0 ? (p.y > 0.0 ? r.x : r.y) : (p.y > 0.0 ? r.w : r.z);
    vec2 q = abs(p) - half_size + vec2(radius);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}
"#;

/// A rectangle in window pixels, GL style: origin bottom-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    /// The viewport egui set up for a paint callback.
    pub fn from_viewport(info: &egui::PaintCallbackInfo) -> Self {
        let v = info.viewport_in_pixels();
        Self { x: v.left_px, y: v.from_bottom_px, width: v.width_px, height: v.height_px }
    }

    fn expand(self, by: i32) -> Self {
        Self { x: self.x - by, y: self.y - by, width: self.width + 2 * by, height: self.height + 2 * by }
    }

    /// Clamp to a framebuffer of `width` x `height`.
    fn clamp_to(self, width: i32, height: i32) -> Self {
        let x0 = self.x.clamp(0, width);
        let y0 = self.y.clamp(0, height);
        let x1 = (self.x + self.width).clamp(0, width);
        let y1 = (self.y + self.height).clamp(0, height);
        Self { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
    }

    fn is_empty(self) -> bool {
        self.width <= 0 || self.height <= 0
    }
}

/// Corner radii in pixels, in screen terms. Converted to the SDF's GL order
/// when uploaded.
#[derive(Debug, Clone, Copy)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub fn uniform(radius: f32) -> Self {
        Self { top_left: radius, top_right: radius, bottom_right: radius, bottom_left: radius }
    }

    fn gl_order(self) -> [f32; 4] {
        // GL's y axis points up, so screen-top is the SDF's positive y.
        [self.top_right, self.bottom_right, self.bottom_left, self.top_left]
    }
}

// ---------------------------------------------------------------------------
// Frosted glass
// ---------------------------------------------------------------------------

/// Work at quarter resolution: a blur this wide hides the lost detail, and it
/// cuts the pixels blurred by 16x.
const GLASS_DOWNSCALE: i32 = 4;
/// Extra source pixels read around the panel, so the blur near its edges
/// samples real page content instead of smearing the border inwards.
const GLASS_MARGIN: i32 = 24;
/// Each iteration is one horizontal plus one vertical pass. Two give a soft,
/// even frost without visible banding.
const GLASS_ITERATIONS: usize = 2;

const BLUR_FRAGMENT: &str = r#"#version 300 es
precision mediump float;
uniform sampler2D u_source;
uniform vec2 u_texel_step;  // one texel along the blur direction
out vec4 o_color;
void main() {
    vec2 uv = gl_FragCoord.xy / vec2(textureSize(u_source, 0));
    // 9-tap Gaussian folded into 5 bilinear fetches.
    vec4 sum = texture(u_source, uv) * 0.2270270270;
    sum += texture(u_source, uv + u_texel_step * 1.3846153846) * 0.3162162162;
    sum += texture(u_source, uv - u_texel_step * 1.3846153846) * 0.3162162162;
    sum += texture(u_source, uv + u_texel_step * 3.2307692308) * 0.0702702703;
    sum += texture(u_source, uv - u_texel_step * 3.2307692308) * 0.0702702703;
    o_color = sum;
}
"#;

fn glass_composite_fragment() -> String {
    format!(
        r#"#version 300 es
precision mediump float;
uniform sampler2D u_blurred;
uniform vec4 u_region;  // blurred region: x, y, width, height in window pixels
uniform vec4 u_panel;   // panel: x, y, width, height in window pixels
uniform vec4 u_radii;
uniform float u_opacity;
out vec4 o_color;
{ROUNDED_RECT_SDF}
void main() {{
    vec2 uv = (gl_FragCoord.xy - u_region.xy) / u_region.zw;
    vec4 color = texture(u_blurred, uv);
    // A touch more saturation keeps the frost from looking washed out.
    float luma = dot(color.rgb, vec3(0.299, 0.587, 0.114));
    // The colour is premultiplied, so clamp to alpha to keep it valid.
    color.rgb = clamp(mix(vec3(luma), color.rgb, 1.25), 0.0, color.a);
    vec2 p = gl_FragCoord.xy - (u_panel.xy + u_panel.zw * 0.5);
    float coverage = clamp(0.5 - rounded_rect(p, u_panel.zw * 0.5, u_radii), 0.0, 1.0);
    o_color = color * coverage * u_opacity;
}}
"#
    )
}

struct GlassGl {
    blur: glow::Program,
    composite: glow::Program,
    vertex_array: glow::VertexArray,
    /// Two ping-pong render targets at quarter resolution.
    targets: [(glow::Texture, glow::Framebuffer); 2],
    size: (i32, i32),
}

/// Frosted glass behind a rounded rectangle. Clone it into the paint callback.
#[derive(Clone, Default)]
pub struct Glass(Arc<Mutex<Option<Option<GlassGl>>>>);

impl Glass {
    /// Blur what is already in the framebuffer under the callback's viewport
    /// and draw it back with `corners`, faded by `opacity`.
    pub fn draw(&self, info: &egui::PaintCallbackInfo, gl: &glow::Context, corners: Corners, opacity: f32) {
        let Ok(mut slot) = self.0.lock() else {
            return;
        };
        let Some(state) = slot.get_or_insert_with(|| create_glass(gl)).as_mut() else {
            return;
        };

        let panel = PixelRect::from_viewport(info);
        let [screen_w, screen_h] = info.screen_size_px.map(|v| v as i32);
        let region = panel.expand(GLASS_MARGIN).clamp_to(screen_w, screen_h);
        if region.is_empty() || panel.is_empty() {
            return;
        }
        let small = (
            (region.width / GLASS_DOWNSCALE).max(1),
            (region.height / GLASS_DOWNSCALE).max(1),
        );

        // SAFETY: plain GL calls on the current context with objects we own.
        unsafe {
            let target = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            if state.size != small {
                resize_targets(gl, state, small);
            }
            let [(tex_a, fbo_a), (tex_b, fbo_b)] = state.targets;

            // 1. Copy the page under the panel into target A, downsampling.
            //    Scissoring also clips blits, so it must be off.
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, target);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(fbo_a));
            gl.blit_framebuffer(
                region.x,
                region.y,
                region.x + region.width,
                region.y + region.height,
                0,
                0,
                small.0,
                small.1,
                glow::COLOR_BUFFER_BIT,
                glow::LINEAR,
            );

            // 2. Separable Gaussian: A -> B horizontally, B -> A vertically.
            gl.viewport(0, 0, small.0, small.1);
            gl.use_program(Some(state.blur));
            gl.bind_vertex_array(Some(state.vertex_array));
            gl.active_texture(glow::TEXTURE0);
            let step = gl.get_uniform_location(state.blur, "u_texel_step");
            gl.uniform_1_i32(gl.get_uniform_location(state.blur, "u_source").as_ref(), 0);
            for _ in 0..GLASS_ITERATIONS {
                for (source, destination, direction) in [
                    (tex_a, fbo_b, (1.0 / small.0 as f32, 0.0)),
                    (tex_b, fbo_a, (0.0, 1.0 / small.1 as f32)),
                ] {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(destination));
                    gl.bind_texture(glow::TEXTURE_2D, Some(source));
                    gl.uniform_2_f32(step.as_ref(), direction.0, direction.1);
                    gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                }
            }

            // 3. Draw the frost back under the panel, through its rounded shape.
            gl.bind_framebuffer(glow::FRAMEBUFFER, target);
            gl.viewport(panel.x, panel.y, panel.width, panel.height);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.use_program(Some(state.composite));
            gl.bind_texture(glow::TEXTURE_2D, Some(tex_a));
            let uniform = |name| gl.get_uniform_location(state.composite, name);
            gl.uniform_1_i32(uniform("u_blurred").as_ref(), 0);
            gl.uniform_4_f32(
                uniform("u_region").as_ref(),
                region.x as f32,
                region.y as f32,
                region.width as f32,
                region.height as f32,
            );
            gl.uniform_4_f32(
                uniform("u_panel").as_ref(),
                panel.x as f32,
                panel.y as f32,
                panel.width as f32,
                panel.height as f32,
            );
            let [a, b, c, d] = corners.gl_order();
            gl.uniform_4_f32(uniform("u_radii").as_ref(), a, b, c, d);
            gl.uniform_1_f32(uniform("u_opacity").as_ref(), opacity);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(None);
        }
    }
}

fn create_glass(gl: &glow::Context) -> Option<GlassGl> {
    let blur = compile_program(gl, FULL_VIEWPORT_VERTEX, BLUR_FRAGMENT)?;
    let composite = compile_program(gl, FULL_VIEWPORT_VERTEX, &glass_composite_fragment())?;
    // SAFETY: plain GL object creation on the current context.
    unsafe {
        let vertex_array = gl.create_vertex_array().ok()?;
        let target = || -> Option<(glow::Texture, glow::Framebuffer)> {
            Some((gl.create_texture().ok()?, gl.create_framebuffer().ok()?))
        };
        let targets = [target()?, target()?];
        Some(GlassGl { blur, composite, vertex_array, targets, size: (0, 0) })
    }
}

/// (Re)allocate both render targets at `size`.
///
/// # Safety
/// Must be called with the GL context current.
unsafe fn resize_targets(gl: &glow::Context, state: &mut GlassGl, size: (i32, i32)) {
    unsafe {
        for (texture, framebuffer) in state.targets {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size.0,
                size.1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (parameter, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
        }
        state.size = size;
    }
}

// ---------------------------------------------------------------------------
// Edge glow
// ---------------------------------------------------------------------------

/// How far the glow reaches outside the card, in pixels at 100% scale.
pub const GLOW_REACH: f32 = 12.0;

fn glow_fragment() -> String {
    format!(
        r#"#version 300 es
precision mediump float;
uniform vec4 u_card;   // x, y, width, height in window pixels
uniform vec4 u_radii;
uniform float u_time;
uniform float u_strength;
uniform float u_scale; // pixels per point
uniform vec3 u_color_a;
uniform vec3 u_color_b;
out vec4 o_color;
{ROUNDED_RECT_SDF}
const float TAU = 6.28318530718;
void main() {{
    vec2 center = u_card.xy + u_card.zw * 0.5;
    vec2 p = gl_FragCoord.xy - center;
    float d = rounded_rect(p, u_card.zw * 0.5, u_radii) / u_scale;
    // A crisp rim just inside the edge, and a soft halo outside it.
    float rim = d < 0.0 ? exp(d * 0.9) : exp(-d / 4.0);
    // Two bright arcs chase each other round the card.
    float angle = atan(p.y * u_card.z / u_card.w, p.x);
    float sweep = 0.5 + 0.5 * cos(2.0 * (angle - u_time * 2.2));
    float highlight = pow(sweep, 5.0);
    vec3 color = mix(u_color_a, u_color_b, 0.5 + 0.5 * sin(angle + u_time * 0.9));
    float alpha = rim * (0.18 + 0.82 * highlight) * u_strength;
    o_color = vec4(color * alpha, alpha);
}}
"#
    )
}

/// The loading glow. Clone it into the paint callback.
#[derive(Clone, Default)]
pub struct EdgeGlow(Arc<Mutex<Option<Option<(glow::Program, glow::VertexArray)>>>>);

pub struct GlowParams {
    pub card: PixelRect,
    pub corners: Corners,
    pub time: f32,
    pub strength: f32,
    pub pixels_per_point: f32,
    pub color_a: egui::Rgba,
    pub color_b: egui::Rgba,
}

impl EdgeGlow {
    /// Draw the glow over the callback's viewport, which must cover the card
    /// plus [`GLOW_REACH`] on every side.
    pub fn draw(&self, gl: &glow::Context, params: &GlowParams) {
        let Ok(mut slot) = self.0.lock() else {
            return;
        };
        let created = slot.get_or_insert_with(|| {
            let program = compile_program(gl, FULL_VIEWPORT_VERTEX, &glow_fragment())?;
            // SAFETY: plain GL object creation on the current context.
            let vertex_array = unsafe { gl.create_vertex_array().ok()? };
            Some((program, vertex_array))
        });
        let Some((program, vertex_array)) = *created else {
            return;
        };
        // SAFETY: plain GL calls on the current context with objects we own.
        unsafe {
            gl.use_program(Some(program));
            let uniform = |name| gl.get_uniform_location(program, name);
            let card = params.card;
            gl.uniform_4_f32(
                uniform("u_card").as_ref(),
                card.x as f32,
                card.y as f32,
                card.width as f32,
                card.height as f32,
            );
            let [a, b, c, d] = params.corners.gl_order();
            gl.uniform_4_f32(uniform("u_radii").as_ref(), a, b, c, d);
            gl.uniform_1_f32(uniform("u_time").as_ref(), params.time);
            gl.uniform_1_f32(uniform("u_strength").as_ref(), params.strength);
            gl.uniform_1_f32(uniform("u_scale").as_ref(), params.pixels_per_point);
            let [r, g, b2, _] = params.color_a.to_array();
            gl.uniform_3_f32(uniform("u_color_a").as_ref(), r, g, b2);
            let [r, g, b2, _] = params.color_b.to_array();
            gl.uniform_3_f32(uniform("u_color_b").as_ref(), r, g, b2);

            gl.bind_vertex_array(Some(vertex_array));
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_clamp_to_the_framebuffer() {
        let r = PixelRect { x: -10, y: 790, width: 200, height: 40 }.clamp_to(1100, 820);
        assert_eq!(r, PixelRect { x: 0, y: 790, width: 190, height: 30 });
        assert!(PixelRect { x: 2000, y: 0, width: 10, height: 10 }.clamp_to(1100, 820).is_empty());
    }

    #[test]
    fn corners_map_to_gl_y_up_order() {
        let c = Corners { top_left: 1.0, top_right: 2.0, bottom_right: 3.0, bottom_left: 4.0 };
        // The SDF picks r.x for +x/+y, which in GL's y-up space is top-right.
        assert_eq!(c.gl_order(), [2.0, 3.0, 4.0, 1.0]);
    }

    #[test]
    fn shaders_declare_glsl_es_3() {
        for source in [FULL_VIEWPORT_VERTEX, BLUR_FRAGMENT, &glass_composite_fragment(), &glow_fragment()] {
            assert!(source.starts_with("#version 300 es\n"), "ANGLE needs the version on line 1");
        }
    }
}
