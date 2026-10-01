//! Rounded corners for the page card.
//!
//! Servo's page reaches the window through `glBlitFramebuffer`, which copies a
//! plain rectangle and ignores every per-fragment test, so the corners cannot
//! be clipped on the way in. Instead, right after the blit, one quad is drawn
//! over the page with a signed-distance rounded-rectangle shader and the blend
//! function `dst = dst * (1 - src.a)`. Inside the rounded rectangle `src.a` is 0
//! and the page is untouched; outside it, colour *and* alpha go to zero, so the
//! Mica backdrop shows through the corners. The edge is anti-aliased because
//! `src.a` ramps over one pixel.
//!
//! Painting background-coloured corner wedges with egui would not work here:
//! the window is transparent, and drawing a transparent colour over the page
//! leaves the page untouched rather than erasing it.

use std::sync::{Arc, OnceLock};

use egui_glow::glow::{self, HasContext};

const FRAGMENT: &str = r#"#version 300 es
precision mediump float;
uniform vec2 u_origin;  // viewport origin, window pixels, bottom-left
uniform vec2 u_size;    // viewport size, pixels
uniform float u_radius; // corner radius, pixels
out vec4 o_color;
void main() {
    vec2 half_size = u_size * 0.5;
    vec2 p = gl_FragCoord.xy - u_origin - half_size;
    vec2 q = abs(p) - (half_size - vec2(u_radius));
    float distance = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - u_radius;
    float coverage = clamp(0.5 - distance, 0.0, 1.0);
    o_color = vec4(0.0, 0.0, 0.0, 1.0 - coverage);
}
"#;

/// The compiled program and an empty vertex array, created on first use.
/// `None` inside the cell means compilation failed: the page then simply keeps
/// square corners, which is better than failing every frame.
#[derive(Clone, Copy)]
struct Resources {
    program: glow::Program,
    vertex_array: glow::VertexArray,
}

/// Shared, lazily-initialised GL resources. Cheap to clone into each frame's
/// paint callback, which egui requires to be `Send + Sync`.
#[derive(Clone, Default)]
pub struct PageMask(Arc<OnceLock<Option<Resources>>>);

impl PageMask {
    /// Cut rounded corners of `radius` pixels out of the current viewport.
    ///
    /// Call from inside an egui paint callback, after the page blit: egui has
    /// set the viewport to the page rectangle and restores its own GL state
    /// afterwards.
    pub fn apply(&self, gl: &glow::Context, viewport: [i32; 4], radius: f32) {
        if radius <= 0.0 {
            return;
        }
        let Some(resources) = *self.0.get_or_init(|| create(gl)) else {
            return;
        };
        let [x, y, width, height] = viewport;
        // SAFETY: plain GL calls on the current context with valid objects.
        unsafe {
            gl.use_program(Some(resources.program));
            let location = |name| gl.get_uniform_location(resources.program, name);
            gl.uniform_2_f32(location("u_origin").as_ref(), x as f32, y as f32);
            gl.uniform_2_f32(location("u_size").as_ref(), width as f32, height as f32);
            gl.uniform_1_f32(location("u_radius").as_ref(), radius);

            gl.bind_vertex_array(Some(resources.vertex_array));
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func_separate(
                glow::ZERO,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ZERO,
                glow::ONE_MINUS_SRC_ALPHA,
            );
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }
}

fn create(gl: &glow::Context) -> Option<Resources> {
    let program = crate::gpu_fx::compile_program(gl, crate::gpu_fx::FULL_VIEWPORT_VERTEX, FRAGMENT)?;
    // SAFETY: plain GL object creation on the current context.
    let vertex_array = unsafe { gl.create_vertex_array().ok()? };
    Some(Resources { program, vertex_array })
}
