//! Flat-coloured lines, used for the block selection outline.

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl LineVertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub const fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct LineMesh {
    pub vertices: Vec<LineVertex>,
}

impl LineMesh {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Add a wireframe box from `min` to `max`.
    pub fn push_box(&mut self, min: [f32; 3], max: [f32; 3], color: [f32; 3]) {
        let corners = [
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], min[1], max[2]],
            [min[0], min[1], max[2]],
            [min[0], max[1], min[2]],
            [max[0], max[1], min[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
        ];
        const EDGES: [(usize, usize); 12] = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ];
        for (a, b) in EDGES {
            self.vertices.push(LineVertex {
                position: corners[a],
                color,
            });
            self.vertices.push(LineVertex {
                position: corners[b],
                color,
            });
        }
    }
}

/// Outline of the block at `x,y,z`, grown slightly so it doesn't z-fight with
/// the block's own faces.
pub fn block_outline(x: i32, y: i32, z: i32, color: [f32; 3]) -> LineMesh {
    const GROW: f32 = 0.002;
    let mut mesh = LineMesh::default();
    mesh.push_box(
        [x as f32 - GROW, y as f32 - GROW, z as f32 - GROW],
        [
            x as f32 + 1.0 + GROW,
            y as f32 + 1.0 + GROW,
            z as f32 + 1.0 + GROW,
        ],
        color,
    );
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_has_twelve_edges() {
        let mut mesh = LineMesh::default();
        mesh.push_box([0.0; 3], [1.0; 3], [1.0, 1.0, 1.0]);
        assert_eq!(mesh.vertices.len(), 24);
    }

    #[test]
    fn outline_surrounds_the_block() {
        let mesh = block_outline(4, 70, -2, [0.0, 0.0, 0.0]);
        let min_x = mesh
            .vertices
            .iter()
            .fold(f32::MAX, |acc, v| acc.min(v.position[0]));
        let max_x = mesh
            .vertices
            .iter()
            .fold(f32::MIN, |acc, v| acc.max(v.position[0]));
        assert!(min_x <= 4.0);
        assert!(max_x >= 5.0);
    }
}
