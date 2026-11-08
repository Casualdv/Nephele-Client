//! `nephele-render` - wgpu rendering for Nephele.
//!
//! CPU halves ([`mesh`], [`camera`], [`lines`], [`ui`]) build geometry and
//! layout, fully testable without a GPU. The GPU half ([`gpu`]) owns the
//! surface, pipelines and draw calls.

pub mod beacon;
pub mod camera;
pub mod entity;
pub mod environment;
pub mod gpu;
pub mod lines;
pub mod mesh;
pub mod particles;
pub mod ui;

pub use beacon::{
    beam_at, beams_in_column, build_beams, dye_color, find_beams, BeaconBeam, BeaconSegment,
};
pub use camera::Camera;
pub use entity::{
    append_hand, append_hand_map, append_item_frame, build_entities, humanoid_hand, EntityDraw,
    EntityMesh, EntityVertex, HandDraw, HandPose, ItemFrameDraw, MobModel,
};
pub use environment::{biome_fog_color, Environment};
pub use gpu::{DrawOutcome, Renderer, RendererError, StartupSample, StartupStage, StartupTimings};
pub use lines::{block_outline, LineMesh, LineVertex};
pub use mesh::{
    break_overlay_texture, build_break_overlay, build_column_mesh, build_hand_item,
    build_hand_sprite, build_item_mesh, build_world_mesh, mesh_column, ColumnMeshJob, HandItem,
    ItemDraw, MeshData, Vertex,
};
pub use particles::{note_color, Particles};
pub use ui::{Rect, UiBatch, UiVertex};
