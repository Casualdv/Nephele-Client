//! `nephele-render` - wgpu rendering for Nephele.
//!
//! CPU halves ([`mesh`], [`camera`], [`lines`], [`ui`]) build geometry and
//! layout, fully testable without a GPU. The GPU half ([`gpu`]) owns the
//! surface, pipelines and draw calls.

pub mod camera;
pub mod entity;
pub mod gpu;
pub mod lines;
pub mod mesh;
pub mod particle;
pub mod ui;

pub use camera::Camera;
pub use entity::{
    append_block_entity, append_hand, append_hand_map, append_item_frame, build_entities,
    humanoid_hand, mob_model, BlockEntityDraw, BlockEntityModel, EntityDraw, EntityMesh,
    EntityVertex, HandDraw, HandPose, ItemFrameDraw, MobModel,
};
pub use gpu::{DrawOutcome, Renderer, RendererError, StartupSample, StartupStage, StartupTimings};
pub use lines::{block_outline, LineMesh, LineVertex};
pub use mesh::{
    break_overlay_texture, build_break_overlay, build_column_mesh, build_hand_item,
    build_hand_sprite, build_item_mesh, build_world_mesh, mesh_column, ColumnMeshJob, HandItem,
    ItemDraw, MeshData, Vertex,
};
pub use particle::{build_particle_mesh, ParticleDraw, ParticleKind};
pub use ui::{Rect, UiBatch, UiVertex};
