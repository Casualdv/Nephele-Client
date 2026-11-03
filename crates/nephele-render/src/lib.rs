//! `nephele-render` - wgpu rendering for Nephele.
//!
//! CPU halves ([`mesh`], [`camera`], [`lines`], [`ui`]) build geometry and
//! layout, fully testable without a GPU. The GPU half ([`gpu`]) owns the
//! surface, pipelines and draw calls.

pub mod camera;
pub mod entity;
pub mod environment;
pub mod gpu;
pub mod lines;
pub mod mesh;
pub mod ui;

pub use camera::Camera;
pub use entity::{
    append_hand, build_entities, EntityDraw, EntityMesh, EntityVertex, HandDraw, HandPose,
};
pub use environment::{biome_fog_color, Environment};
pub use gpu::{
    DrawOutcome, Renderer, RendererError, RendererMemory, StartupSample, StartupStage,
    StartupTimings,
};
pub use lines::{block_outline, LineMesh, LineVertex};
pub use mesh::{
    break_overlay_texture, build_break_overlay, build_column_mesh, build_hand_item,
    build_item_mesh, build_world_mesh, mesh_column, ColumnMeshJob, HandItem, ItemDraw, MeshData,
    Vertex,
};
pub use ui::{Rect, UiBatch, UiVertex};
