//! Immediate-mode 2D UI: solid rectangles, atlas sprites, vanilla bitmap text,
//! isometric block icons and flat item icons.
//!
//! Geometry is emitted in physical screen pixels; the shader turns it into clip
//! space using the current surface size.

use nephele_assets::{font, sprites, SpriteRect, UiAtlas};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UiVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// Packed texture slot used by block and item icons.
    pub layer: u32,
}

impl UiVertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2,
        2 => Float32x4,
        3 => Uint32,
    ];

    pub const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<UiVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// A quad soup with a shared index buffer.
#[derive(Debug, Default, Clone)]
pub struct UiBatch {
    pub vertices: Vec<UiVertex>,
    pub indices: Vec<u32>,
}

impl UiBatch {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn quad(&mut self, corners: [[f32; 2]; 4], uv: [f32; 4], color: [f32; 4], layer: u32) {
        let base = self.vertices.len() as u32;
        let uvs = [
            [uv[0], uv[1]],
            [uv[2], uv[1]],
            [uv[2], uv[3]],
            [uv[0], uv[3]],
        ];
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            self.vertices.push(UiVertex {
                position: *corner,
                uv: *uv,
                color,
                layer,
            });
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Axis-aligned solid rectangle.
    pub fn rect(&mut self, rect: Rect, color: [f32; 4]) {
        let uv = UiAtlas::white_uv();
        self.quad(
            [
                [rect.x, rect.y],
                [rect.x + rect.w, rect.y],
                [rect.x + rect.w, rect.y + rect.h],
                [rect.x, rect.y + rect.h],
            ],
            uv,
            color,
            0,
        );
    }

    /// Outline of `rect`, `thickness` pixels wide.
    pub fn rect_outline(&mut self, rect: Rect, thickness: f32, color: [f32; 4]) {
        self.rect(Rect::new(rect.x, rect.y, rect.w, thickness), color);
        self.rect(
            Rect::new(rect.x, rect.y + rect.h - thickness, rect.w, thickness),
            color,
        );
        self.rect(Rect::new(rect.x, rect.y, thickness, rect.h), color);
        self.rect(
            Rect::new(rect.x + rect.w - thickness, rect.y, thickness, rect.h),
            color,
        );
    }

    /// Draw an atlas sprite.
    pub fn sprite(&mut self, sprite: SpriteRect, rect: Rect, tint: [f32; 4]) {
        self.quad(
            [
                [rect.x, rect.y],
                [rect.x + rect.w, rect.y],
                [rect.x + rect.w, rect.y + rect.h],
                [rect.x, rect.y + rect.h],
            ],
            sprite.uv(),
            tint,
            0,
        );
    }

    /// Draw an atlas sprite cropped horizontally, for partial bars.
    pub fn sprite_cropped(&mut self, sprite: SpriteRect, rect: Rect, fraction: f32) {
        let fraction = fraction.clamp(0.0, 1.0);
        if fraction <= 0.0 {
            return;
        }
        let uv = sprite.uv();
        let width = uv[2] - uv[0];
        let cropped = [uv[0], uv[1], uv[0] + width * fraction, uv[3]];
        self.quad(
            [
                [rect.x, rect.y],
                [rect.x + rect.w * fraction, rect.y],
                [rect.x + rect.w * fraction, rect.y + rect.h],
                [rect.x, rect.y + rect.h],
            ],
            cropped,
            [1.0, 1.0, 1.0, 1.0],
            0,
        );
    }

    /// Vanilla bitmap text with a 1px drop shadow.
    pub fn text(&mut self, content: &str, x: f32, y: f32, scale: f32, color: [f32; 4]) {
        let shadow = [color[0] * 0.25, color[1] * 0.25, color[2] * 0.25, color[3]];
        let mut cursor_y = y;
        for line in content.split('\n') {
            let mut cursor_x = x;
            for ch in line.chars() {
                if let Some(glyph) = font::glyph(ch) {
                    let target = Rect::new(
                        cursor_x + scale,
                        cursor_y + scale,
                        glyph.w as f32 * scale,
                        glyph.h as f32 * scale,
                    );
                    self.sprite(glyph, target, shadow);
                    let target = Rect::new(
                        cursor_x,
                        cursor_y,
                        glyph.w as f32 * scale,
                        glyph.h as f32 * scale,
                    );
                    self.sprite(glyph, target, color);
                }
                cursor_x += font::advance(ch) * scale;
            }
            cursor_y += (font::HEIGHT + 2.0) * scale;
        }
    }

    /// Width of `content` in pixels at the given scale.
    pub fn text_width(content: &str, scale: f32) -> f32 {
        content
            .split('\n')
            .map(|line| line.chars().map(font::advance).sum::<f32>())
            .fold(0.0f32, f32::max)
            * scale
    }

    /// Centred text around `center_x`.
    pub fn text_centered(
        &mut self,
        content: &str,
        center_x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
    ) {
        let width = Self::text_width(content, scale);
        self.text(content, center_x - width / 2.0, y, scale, color);
    }

    /// A vanilla-style isometric block icon: a top diamond plus two side faces.
    pub fn block_icon(
        &mut self,
        x: f32,
        y: f32,
        size: f32,
        top_layer: u32,
        side_layer: u32,
        alpha: f32,
    ) {
        let half = size / 2.0;
        let top_h = size * 0.25;
        let body = size * 0.5;
        let cx = x + half;
        let uvs = [0.0, 0.0, 1.0, 1.0];

        // Top face.
        self.quad(
            [
                [cx, y],
                [x + size, y + top_h],
                [cx, y + top_h * 2.0],
                [x, y + top_h],
            ],
            uvs,
            [1.0, 1.0, 1.0, alpha],
            top_layer,
        );
        // Left face.
        self.quad(
            [
                [x, y + top_h],
                [cx, y + top_h * 2.0],
                [cx, y + top_h * 2.0 + body],
                [x, y + top_h + body],
            ],
            uvs,
            [0.66, 0.66, 0.66, alpha],
            side_layer,
        );
        // Right face.
        self.quad(
            [
                [cx, y + top_h * 2.0],
                [x + size, y + top_h],
                [x + size, y + top_h + body],
                [cx, y + top_h * 2.0 + body],
            ],
            uvs,
            [0.82, 0.82, 0.82, alpha],
            side_layer,
        );
    }

    /// Convenience: the crosshair the HUD draws over the world.
    pub fn crosshair(&mut self, center_x: f32, center_y: f32, scale: f32) {
        let size = 16.0 * scale;
        self.sprite(
            sprites::CROSSHAIR,
            Rect::new(center_x - size / 2.0, center_y - size / 2.0, size, size),
            [1.0, 1.0, 1.0, 1.0],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_emits_four_vertices_and_six_indices() {
        let mut batch = UiBatch::default();
        batch.rect(Rect::new(0.0, 0.0, 10.0, 10.0), [1.0; 4]);
        assert_eq!(batch.vertices.len(), 4);
        assert_eq!(batch.indices.len(), 6);
    }

    #[test]
    fn text_advances_by_the_font_width() {
        // Without a measured atlas every glyph falls back to 6 px.
        assert!((UiBatch::text_width("abc", 1.0) - 18.0).abs() < 0.01);
        assert!((UiBatch::text_width("ab\nabcd", 1.0) - 24.0).abs() < 0.01);
        assert_eq!(UiBatch::text_width("", 1.0), 0.0);
    }

    #[test]
    fn text_emits_two_quads_per_glyph_for_the_shadow() {
        let mut batch = UiBatch::default();
        batch.text("hi", 0.0, 0.0, 1.0, [1.0; 4]);
        assert_eq!(batch.vertices.len(), 2 * 2 * 4);
    }

    #[test]
    fn block_icons_have_three_faces() {
        let mut batch = UiBatch::default();
        batch.block_icon(0.0, 0.0, 16.0, 1, 2, 1.0);
        assert_eq!(batch.vertices.len(), 12);
        assert!(batch.vertices.iter().all(|v| v.layer <= 2));
    }

    #[test]
    fn rect_hit_testing_excludes_the_far_edges() {
        let rect = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert!(rect.contains(10.0, 10.0));
        assert!(rect.contains(29.9, 29.9));
        assert!(!rect.contains(30.0, 15.0));
        assert!(!rect.contains(9.0, 15.0));
    }
}
