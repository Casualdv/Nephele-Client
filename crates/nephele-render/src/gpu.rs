use nephele_assets::{Panorama, SkinAtlas, TextureArray, UiAtlas};
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::entity::{EntityMesh, EntityVertex};
use crate::lines::{LineMesh, LineVertex};
use crate::mesh::{MeshData, Vertex};
use crate::ui::{UiBatch, UiVertex};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SKY_COLOR: wgpu::Color = wgpu::Color {
    r: 0.42,
    g: 0.62,
    b: 0.96,
    a: 1.0,
};

#[derive(Debug, thiserror::Error)]
pub enum RendererError {
    #[error("could not create a surface: {0}")]
    Surface(String),
    #[error("no suitable GPU adapter: {0}")]
    Adapter(String),
    #[error("could not create a GPU device: {0}")]
    Device(String),
}

/// What the caller should do after a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawOutcome {
    Presented,
    /// Nothing was drawn this frame (window occluded, acquire timed out).
    Skipped,
    /// The surface was reconfigured; just keep going next frame.
    Reconfigured,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UiUniforms {
    screen: [f32; 2],
    _pad: [f32; 2],
}

struct UiVertexBuffers {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    vertex_capacity: wgpu::BufferAddress,
    index_capacity: wgpu::BufferAddress,
    index_count: u32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    chunks: std::collections::HashMap<(i32, i32), ChunkBuffers>,
    overlay_pipeline: wgpu::RenderPipeline,
    overlay_vertex_buffer: wgpu::Buffer,
    overlay_index_buffer: wgpu::Buffer,
    overlay_index_count: u32,
    line_pipeline: wgpu::RenderPipeline,
    line_bind_group: wgpu::BindGroup,
    line_vertex_buffer: wgpu::Buffer,
    line_vertex_count: u32,
    ui_uniform_buffer: wgpu::Buffer,
    ui_bind_group: wgpu::BindGroup,
    ui_sprite_pipeline: wgpu::RenderPipeline,
    ui_block_pipeline: wgpu::RenderPipeline,
    ui_back: UiVertexBuffers,
    ui_blocks: UiVertexBuffers,
    ui_front: UiVertexBuffers,
    entity_pipeline: wgpu::RenderPipeline,
    entity_bind_group: wgpu::BindGroup,
    skin_texture: wgpu::Texture,
    /// Atlas version already uploaded, so skins only re-upload when they change.
    skin_version: u64,
    entity_vertices: wgpu::Buffer,
    entity_indices: wgpu::Buffer,
    entity_vertex_capacity: wgpu::BufferAddress,
    entity_index_capacity: wgpu::BufferAddress,
    entity_index_count: u32,
    /// Dropped items, drawn with the terrain pipeline (same texture array).
    item_vertices: wgpu::Buffer,
    item_indices: wgpu::Buffer,
    item_vertex_capacity: wgpu::BufferAddress,
    item_index_capacity: wgpu::BufferAddress,
    item_index_count: u32,
    panorama: Option<PanoramaGpu>,
    show_panorama: bool,
    /// Chunks that survived frustum culling in the last [`Renderer::draw`].
    drawn_chunks: usize,
}

impl Renderer {
    /// Create a renderer for a window. `window` can be an `Arc<winit::window::Window>`
    /// or anything else that converts into a wgpu surface target.
    pub fn new<W: Into<wgpu::SurfaceTarget<'static>>>(
        window: W,
        width: u32,
        height: u32,
        textures: &TextureArray,
        ui_atlas: &UiAtlas,
        panorama: Option<&Panorama>,
    ) -> Result<Self, RendererError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window)
            .map_err(|err| RendererError::Surface(err.to_string()))?;

        let power_preference = match std::env::var("NEPHELE_GPU") {
            Ok(value)
                if value.eq_ignore_ascii_case("integrated")
                    || value.eq_ignore_ascii_case("low-power") =>
            {
                wgpu::PowerPreference::LowPower
            }
            _ => wgpu::PowerPreference::HighPerformance,
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|err| RendererError::Adapter(err.to_string()))?;

        if std::env::var("NEPHELE_PERF")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false)
        {
            let info = adapter.get_info();
            eprintln!(
                "[perf] adapter name={:?} type={:?} backend={:?} driver={:?}",
                info.name, info.device_type, info.backend, info.driver
            );
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nephele device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|err| RendererError::Device(err.to_string()))?;

        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .unwrap_or(capabilities.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let depth_view = create_depth_view(&device, &config);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera uniforms"),
            size: std::mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("block bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let block_textures = create_block_texture_array(&device, &queue, textures);
        let texture_view = block_textures.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("block sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("block bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("block shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("block pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("block pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // The crack overlay reuses the block shader, but must draw on top of the
        // block's own faces, so it compares less-or-equal and writes no depth.
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("break overlay pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("line shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("line.wgsl").into()),
        });
        let line_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("line bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let line_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("line bind group"),
            layout: &line_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let line_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("line pipeline layout"),
            bind_group_layouts: &[Some(&line_bind_group_layout)],
            immediate_size: 0,
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("line pipeline"),
            layout: Some(&line_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &line_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(LineVertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &line_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let (overlay_vertex_buffer, overlay_index_buffer) = empty_buffers(&device);
        let line_vertex_buffer = empty_line_buffer(&device);

        // --- 2D UI ---------------------------------------------------------
        let ui_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui uniforms"),
            size: std::mem::size_of::<UiUniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ui_texture = create_ui_texture(&device, &queue, ui_atlas);
        let ui_texture_view = ui_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let ui_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ui bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui bind group"),
            layout: &ui_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ui_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&ui_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let ui_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ui.wgsl").into()),
        });
        let ui_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui pipeline layout"),
            bind_group_layouts: &[Some(&ui_bind_group_layout)],
            immediate_size: 0,
        });
        let ui_sprite_pipeline = create_ui_pipeline(
            &device,
            &ui_pipeline_layout,
            &ui_shader,
            "fs_sprite",
            format,
            "ui sprite pipeline",
        );
        let ui_block_pipeline = create_ui_pipeline(
            &device,
            &ui_pipeline_layout,
            &ui_shader,
            "fs_block",
            format,
            "ui block icon pipeline",
        );
        let ui_back = empty_ui_buffers(&device);
        let ui_blocks = empty_ui_buffers(&device);
        let ui_front = empty_ui_buffers(&device);

        // Entities: their own 2D skin atlas, so they can't disturb the block
        // texture array the world pass is bound to.
        let entity_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("entity bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let skin_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("skin atlas"),
            size: wgpu::Extent3d {
                width: nephele_assets::ATLAS_SIZE,
                height: nephele_assets::ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let skin_view = skin_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let entity_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("skin sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let entity_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("entity bind group"),
            layout: &entity_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&skin_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&entity_sampler),
                },
            ],
        });

        let entity_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("entity shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("entity.wgsl").into()),
        });
        let entity_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("entity pipeline layout"),
                bind_group_layouts: &[Some(&entity_bind_group_layout)],
                immediate_size: 0,
            });
        let entity_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity pipeline"),
            layout: Some(&entity_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &entity_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(EntityVertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &entity_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let (entity_vertex_capacity, entity_index_capacity) = (16 * 1024, 16 * 1024);
        let entity_vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("entity vertices"),
            size: entity_vertex_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity_indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("entity indices"),
            size: entity_index_capacity,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Item buffers start small: a dropped stack is six quads.
        let (item_vertex_capacity, item_index_capacity) = (1024, 1024);
        let item_vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("item vertices"),
            size: item_vertex_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let item_indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("item indices"),
            size: item_index_capacity,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let panorama = panorama.map(|panorama| {
            create_panorama(
                &device,
                &queue,
                panorama,
                &bind_group_layout,
                &uniform_buffer,
                &sampler,
                &shader,
                &pipeline_layout,
                format,
            )
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            depth_view,
            uniform_buffer,
            bind_group,
            chunks: std::collections::HashMap::new(),
            overlay_pipeline,
            overlay_vertex_buffer,
            overlay_index_buffer,
            overlay_index_count: 0,
            line_pipeline,
            line_bind_group,
            line_vertex_buffer,
            line_vertex_count: 0,
            ui_uniform_buffer,
            ui_bind_group,
            ui_sprite_pipeline,
            ui_block_pipeline,
            ui_back,
            ui_blocks,
            ui_front,
            entity_pipeline,
            entity_bind_group,
            skin_texture,
            skin_version: 0,
            entity_vertices,
            entity_indices,
            entity_vertex_capacity,
            entity_index_capacity,
            entity_index_count: 0,
            item_vertices,
            item_indices,
            item_vertex_capacity,
            item_index_capacity,
            item_index_count: 0,
            panorama,
            show_panorama: false,
            drawn_chunks: 0,
        })
    }

    /// Chunks submitted in the last frame, and how many are loaded at all.
    pub fn draw_stats(&self) -> (usize, usize) {
        (self.drawn_chunks, self.chunks.len())
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
    }

    /// Upload a whole-world mesh as a single, always-visible chunk (used by the
    /// synthetic `view` mode).
    pub fn set_mesh(&mut self, mesh: &MeshData) {
        self.set_chunk((i32::MIN, 0), mesh, [-1.0e9; 3], [1.0e9; 3]);
    }

    /// Upload one chunk column's mesh, replacing whatever was there.
    ///
    /// Meshes are kept per column so a single block change only uploads ~4 KB
    /// instead of the entire world. Buffers are reused and grown in place, so a
    /// steady stream of edits does not allocate on every change.
    pub fn set_chunk(&mut self, key: (i32, i32), mesh: &MeshData, min: [f32; 3], max: [f32; 3]) {
        if mesh.is_empty() {
            self.chunks.remove(&key);
            return;
        }
        if !self.chunks.contains_key(&key) {
            self.chunks.insert(key, ChunkBuffers::empty(&self.device));
        }
        let device = &self.device;
        let queue = &self.queue;
        let chunk = self.chunks.get_mut(&key).expect("just inserted");

        let vertex_bytes = std::mem::size_of_val(mesh.vertices.as_slice()) as wgpu::BufferAddress;
        if vertex_bytes > chunk.vertex_capacity {
            chunk.vertex_capacity = allocate_size(vertex_bytes);
            chunk.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("chunk vertices"),
                size: chunk.vertex_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&chunk.vertices, 0, bytemuck::cast_slice(&mesh.vertices));

        let index_bytes = std::mem::size_of_val(mesh.indices.as_slice()) as wgpu::BufferAddress;
        if index_bytes > chunk.index_capacity {
            chunk.index_capacity = allocate_size(index_bytes);
            chunk.indices = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("chunk indices"),
                size: chunk.index_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&chunk.indices, 0, bytemuck::cast_slice(&mesh.indices));

        chunk.index_count = mesh.indices.len() as u32;
        chunk.min = min;
        chunk.max = max;
    }

    pub fn remove_chunk(&mut self, key: (i32, i32)) {
        self.chunks.remove(&key);
    }

    pub fn clear_chunks(&mut self) {
        self.chunks.clear();
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Push the packed skin atlas to the GPU. No-op when nothing changed.
    pub fn set_skins(&mut self, atlas: &SkinAtlas) {
        if atlas.version() == self.skin_version {
            return;
        }
        self.skin_version = atlas.version();
        let size = nephele_assets::ATLAS_SIZE;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.skin_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            atlas.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Replace every entity's geometry. Entities are few, so the whole buffer is
    /// rebuilt each frame rather than tracked per entity.
    pub fn set_entities(&mut self, mesh: &EntityMesh) {
        self.entity_index_count = mesh.indices.len() as u32;
        if mesh.vertices.is_empty() {
            return;
        }

        let vertex_bytes = std::mem::size_of_val(mesh.vertices.as_slice()) as wgpu::BufferAddress;
        if vertex_bytes > self.entity_vertex_capacity {
            self.entity_vertex_capacity = allocate_size(vertex_bytes);
            self.entity_vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("entity vertices"),
                size: self.entity_vertex_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue.write_buffer(
            &self.entity_vertices,
            0,
            bytemuck::cast_slice(&mesh.vertices),
        );

        let index_bytes = std::mem::size_of_val(mesh.indices.as_slice()) as wgpu::BufferAddress;
        if index_bytes > self.entity_index_capacity {
            self.entity_index_capacity = allocate_size(index_bytes);
            self.entity_indices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("entity indices"),
                size: self.entity_index_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue
            .write_buffer(&self.entity_indices, 0, bytemuck::cast_slice(&mesh.indices));
    }

    pub fn entity_index_count(&self) -> u32 {
        self.entity_index_count
    }

    /// Replace every dropped item's geometry. Like entities, the whole buffer
    /// is rebuilt each frame — a handful of 24-vertex cubes is nothing.
    pub fn set_items(&mut self, mesh: &MeshData) {
        self.item_index_count = mesh.indices.len() as u32;
        if mesh.vertices.is_empty() {
            return;
        }

        let vertex_bytes = std::mem::size_of_val(mesh.vertices.as_slice()) as wgpu::BufferAddress;
        if vertex_bytes > self.item_vertex_capacity {
            self.item_vertex_capacity = allocate_size(vertex_bytes);
            self.item_vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("item vertices"),
                size: self.item_vertex_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue
            .write_buffer(&self.item_vertices, 0, bytemuck::cast_slice(&mesh.vertices));

        let index_bytes = std::mem::size_of_val(mesh.indices.as_slice()) as wgpu::BufferAddress;
        if index_bytes > self.item_index_capacity {
            self.item_index_capacity = allocate_size(index_bytes);
            self.item_indices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("item indices"),
                size: self.item_index_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue
            .write_buffer(&self.item_indices, 0, bytemuck::cast_slice(&mesh.indices));
    }

    /// Upload the crack overlay drawn on the block being mined. Pass an empty
    /// mesh to clear it.
    pub fn set_overlay(&mut self, mesh: &MeshData) {
        if mesh.is_empty() {
            let (vertex_buffer, index_buffer) = empty_buffers(&self.device);
            self.overlay_vertex_buffer = vertex_buffer;
            self.overlay_index_buffer = index_buffer;
            self.overlay_index_count = 0;
            return;
        }
        self.overlay_vertex_buffer =
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("overlay vertices"),
                    contents: bytemuck::cast_slice(&mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
        self.overlay_index_buffer =
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("overlay indices"),
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
        self.overlay_index_count = mesh.indices.len() as u32;
    }

    /// Upload the selection outline.
    pub fn set_lines(&mut self, mesh: &LineMesh) {
        self.line_vertex_count = mesh.vertices.len() as u32;
        if mesh.is_empty() {
            self.line_vertex_buffer = empty_line_buffer(&self.device);
            return;
        }
        self.line_vertex_buffer =
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("line vertices"),
                    contents: bytemuck::cast_slice(&mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
    }

    /// Upload this frame's 2D UI, in painter's order: backgrounds, block icons,
    /// then text and overlays.
    /// Turn vertical sync on (`Fifo`, capped to the display) or off (`Immediate`,
    /// uncapped and may tear).
    pub fn set_vsync(&mut self, enabled: bool) {
        let mode = if enabled {
            wgpu::PresentMode::Fifo
        } else {
            wgpu::PresentMode::Immediate
        };
        if self.config.present_mode == mode {
            return;
        }
        self.config.present_mode = mode;
        // Uncapped frames shouldn't queue up behind each other.
        self.config.desired_maximum_frame_latency = if enabled { 2 } else { 1 };
        self.surface.configure(&self.device, &self.config);
    }

    /// Show the title-screen panorama cube instead of the world mesh.
    pub fn set_panorama(&mut self, visible: bool) {
        self.show_panorama = visible;
    }

    pub fn set_ui(&mut self, back: &UiBatch, icons: &UiBatch, front: &UiBatch) {
        let (device, queue) = (&self.device, &self.queue);
        upload_ui(device, queue, &mut self.ui_back, back);
        upload_ui(device, queue, &mut self.ui_blocks, icons);
        upload_ui(device, queue, &mut self.ui_front, front);
    }

    pub fn draw(&mut self, camera: &Camera) -> DrawOutcome {
        let uniforms = Uniforms {
            view_proj: camera.view_projection().to_cols_array_2d(),
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        let ui_uniforms = UiUniforms {
            screen: [self.config.width as f32, self.config.height as f32],
            _pad: [0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.ui_uniform_buffer, 0, bytemuck::bytes_of(&ui_uniforms));

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return DrawOutcome::Skipped
            }
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => {
                self.surface.configure(&self.device, &self.config);
                return DrawOutcome::Reconfigured;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SKY_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Title screen: a six-faced panorama cube instead of the world.
            if self.show_panorama {
                if let Some(panorama) = &self.panorama {
                    pass.set_pipeline(&panorama.pipeline);
                    pass.set_bind_group(0, &panorama.bind_group, &[]);
                    pass.set_vertex_buffer(0, panorama.vertices.slice(..));
                    pass.set_index_buffer(panorama.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..panorama.index_count, 0, 0..1);
                }
            }

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            self.drawn_chunks = 0;
            if !self.chunks.is_empty() {
                let frustum = Frustum::from_view_projection(&camera.view_projection());
                // Nearest first: the depth buffer rejects the rest of the
                // terrain early instead of shading it and discarding it.
                let eye = camera.position;
                let mut visible: Vec<((i32, i32), f32)> = self
                    .chunks
                    .iter()
                    .filter(|(_, chunk)| {
                        chunk.index_count > 0 && frustum.intersects(chunk.min, chunk.max)
                    })
                    .map(|(&key, chunk)| {
                        let cx = (chunk.min[0] + chunk.max[0]) * 0.5 - eye.x;
                        let cy = (chunk.min[1] + chunk.max[1]) * 0.5 - eye.y;
                        let cz = (chunk.min[2] + chunk.max[2]) * 0.5 - eye.z;
                        (key, cx * cx + cy * cy + cz * cz)
                    })
                    .collect();
                visible.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));

                for (key, _) in &visible {
                    let Some(chunk) = self.chunks.get(key) else {
                        continue;
                    };
                    pass.set_vertex_buffer(0, chunk.vertices.slice(..));
                    pass.set_index_buffer(chunk.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..chunk.index_count, 0, 0..1);
                }
                self.drawn_chunks = visible.len();
            }

            // Dropped items are ordinary block-textured cubes, so they share
            // the terrain pipeline and its texture array.
            if self.item_index_count > 0 {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.item_vertices.slice(..));
                pass.set_index_buffer(self.item_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.item_index_count, 0, 0..1);
            }

            // Entities are depth-tested against the terrain but drawn after it,
            // so a player standing behind a hill is properly hidden.
            if self.entity_index_count > 0 {
                pass.set_pipeline(&self.entity_pipeline);
                pass.set_bind_group(0, &self.entity_bind_group, &[]);
                pass.set_vertex_buffer(0, self.entity_vertices.slice(..));
                pass.set_index_buffer(self.entity_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.entity_index_count, 0, 0..1);
            }

            if self.overlay_index_count > 0 {
                pass.set_pipeline(&self.overlay_pipeline);
                // The overlay inherits whatever bind group the last draw left
                // bound, which after entities is the 2D one. Binding the block
                // group here is what keeps the scanline target valid.
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.overlay_vertex_buffer.slice(..));
                pass.set_index_buffer(
                    self.overlay_index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..self.overlay_index_count, 0, 0..1);
            }

            if self.line_vertex_count > 0 {
                pass.set_pipeline(&self.line_pipeline);
                pass.set_bind_group(0, &self.line_bind_group, &[]);
                pass.set_vertex_buffer(0, self.line_vertex_buffer.slice(..));
                pass.draw(0..self.line_vertex_count, 0..1);
            }

            // The HUD/menus sit on top of everything, with no depth test.
            if self.ui_back.index_count > 0 {
                pass.set_pipeline(&self.ui_sprite_pipeline);
                pass.set_bind_group(0, &self.ui_bind_group, &[]);
                pass.set_vertex_buffer(0, self.ui_back.vertices.slice(..));
                pass.set_index_buffer(self.ui_back.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.ui_back.index_count, 0, 0..1);
            }
            if self.ui_blocks.index_count > 0 {
                pass.set_pipeline(&self.ui_block_pipeline);
                pass.set_bind_group(0, &self.ui_bind_group, &[]);
                pass.set_vertex_buffer(0, self.ui_blocks.vertices.slice(..));
                pass.set_index_buffer(self.ui_blocks.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.ui_blocks.index_count, 0, 0..1);
            }
            if self.ui_front.index_count > 0 {
                pass.set_pipeline(&self.ui_sprite_pipeline);
                pass.set_bind_group(0, &self.ui_bind_group, &[]);
                pass.set_vertex_buffer(0, self.ui_front.vertices.slice(..));
                pass.set_index_buffer(self.ui_front.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.ui_front.index_count, 0, 0..1);
            }
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        DrawOutcome::Presented
    }
}

fn empty_buffers(device: &wgpu::Device) -> (wgpu::Buffer, wgpu::Buffer) {
    let vertex = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("empty vertices"),
        size: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX,
        mapped_at_creation: false,
    });
    let index = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("empty indices"),
        size: 4,
        usage: wgpu::BufferUsages::INDEX,
        mapped_at_creation: false,
    });
    (vertex, index)
}

fn create_ui_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fragment_entry: &str,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(UiVertex::layout())],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        // The UI shares the world's render pass, so it has to declare the same
        // depth format, but it draws over everything unconditionally.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Upload a UI batch into persistent buffers.
///
/// The buffers are only reallocated when the batch outgrows them; the common
/// case is a single `write_buffer` per buffer per frame, instead of six
/// `create_buffer` calls that churn the GPU allocator at hundreds of hertz.
fn upload_ui(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &mut UiVertexBuffers,
    batch: &UiBatch,
) {
    target.index_count = batch.indices.len() as u32;
    if batch.vertices.is_empty() {
        return;
    }

    let vertex_bytes = std::mem::size_of_val(batch.vertices.as_slice()) as wgpu::BufferAddress;
    if vertex_bytes > target.vertex_capacity {
        target.vertex_capacity = allocate_size(vertex_bytes);
        target.vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui vertices"),
            size: target.vertex_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }
    queue.write_buffer(&target.vertices, 0, bytemuck::cast_slice(&batch.vertices));

    let index_bytes = std::mem::size_of_val(batch.indices.as_slice()) as wgpu::BufferAddress;
    if index_bytes > target.index_capacity {
        target.index_capacity = allocate_size(index_bytes);
        target.indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui indices"),
            size: target.index_capacity,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }
    queue.write_buffer(&target.indices, 0, bytemuck::cast_slice(&batch.indices));
}

/// Round a size up so steady growth doesn't reallocate every frame.
fn allocate_size(bytes: wgpu::BufferAddress) -> wgpu::BufferAddress {
    let rounded = bytes.next_power_of_two();
    rounded.max(4096)
}

fn empty_ui_buffers(device: &wgpu::Device) -> UiVertexBuffers {
    UiVertexBuffers {
        vertices: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("empty ui vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        indices: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("empty ui indices"),
            size: 4096,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        vertex_capacity: 4096,
        index_capacity: 4096,
        index_count: 0,
    }
}

fn create_ui_texture(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &UiAtlas) -> wgpu::Texture {
    let size = UiAtlas::SIZE;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        atlas.pixels(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * size),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn empty_line_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("empty lines"),
        size: std::mem::size_of::<LineVertex>() as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX,
        mapped_at_creation: false,
    })
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_block_texture_array(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    textures: &TextureArray,
) -> wgpu::Texture {
    let tile = textures.tile_size();
    let size = wgpu::Extent3d {
        width: tile,
        height: tile,
        depth_or_array_layers: textures.layer_count(),
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("block texture array"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        textures.pixels(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * tile),
            rows_per_image: Some(tile),
        },
        size,
    );
    texture
}

/// GPU resources for the title screen's panoramic cube.
struct PanoramaGpu {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

#[allow(clippy::too_many_arguments)]
fn create_panorama(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    panorama: &Panorama,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    shader: &wgpu::ShaderModule,
    pipeline_layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
) -> PanoramaGpu {
    let size = panorama.size.max(1);
    let layers = panorama.faces.len().max(1) as u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("panorama cube"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels = panorama.pixels();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * size),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("panorama bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("panorama pipeline"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(Vertex::layout())],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            // The camera sits inside the cube, so both windings are visible.
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    let (vertices, indices) = panorama_mesh();
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("panorama vertices"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("panorama indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    PanoramaGpu {
        pipeline,
        bind_group,
        vertices: vertex_buffer,
        indices: index_buffer,
        index_count: indices.len() as u32,
    }
}

/// A cube seen from the inside: one quad per panorama face, in vanilla's order
/// (0-3 sides, 4 top, 5 bottom).
fn panorama_mesh() -> (Vec<Vertex>, Vec<u32>) {
    const R: f32 = 100.0;
    let faces: [[[f32; 3]; 4]; 6] = [
        [[-R, -R, -R], [R, -R, -R], [R, R, -R], [-R, R, -R]],
        [[R, -R, -R], [R, -R, R], [R, R, R], [R, R, -R]],
        [[R, -R, R], [-R, -R, R], [-R, R, R], [R, R, R]],
        [[-R, -R, R], [-R, -R, -R], [-R, R, -R], [-R, R, R]],
        [[-R, R, R], [R, R, R], [R, R, -R], [-R, R, -R]],
        [[-R, -R, -R], [R, -R, -R], [R, -R, R], [-R, -R, R]],
    ];
    let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (layer, corners) in faces.iter().enumerate() {
        let base = vertices.len() as u32;
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(Vertex {
                position: *corner,
                uv: *uv,
                shade: 1.0,
                layer: layer as u32,
                light: 1.0,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

/// Per-column GPU buffers plus the world-space bounds used for culling.
struct ChunkBuffers {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    vertex_capacity: wgpu::BufferAddress,
    index_capacity: wgpu::BufferAddress,
    index_count: u32,
    min: [f32; 3],
    max: [f32; 3],
}

impl ChunkBuffers {
    fn empty(device: &wgpu::Device) -> Self {
        Self {
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("chunk vertices"),
                size: 4096,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            indices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("chunk indices"),
                size: 4096,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertex_capacity: 4096,
            index_capacity: 4096,
            index_count: 0,
            min: [0.0; 3],
            max: [0.0; 3],
        }
    }
}

/// Six clip-space planes, used to skip chunks behind the camera.
struct Frustum {
    planes: [[f32; 4]; 6],
}

impl Frustum {
    fn from_view_projection(matrix: &glam::Mat4) -> Self {
        let row = |index: usize| -> [f32; 4] {
            let v = matrix.row(index);
            [v.x, v.y, v.z, v.w]
        };
        let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
        let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
        let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
        Self {
            planes: [
                add(r3, r0), // left
                sub(r3, r0), // right
                add(r3, r1), // bottom
                sub(r3, r1), // top
                r2,          // near (0..1 depth range)
                sub(r3, r2), // far
            ],
        }
    }

    fn intersects(&self, min: [f32; 3], max: [f32; 3]) -> bool {
        for plane in &self.planes {
            // The corner furthest along the plane normal decides: if even that
            // is behind the plane, the whole box is outside.
            let x = if plane[0] >= 0.0 { max[0] } else { min[0] };
            let y = if plane[1] >= 0.0 { max[1] } else { min[1] };
            let z = if plane[2] >= 0.0 { max[2] } else { min[2] };
            if plane[0] * x + plane[1] * y + plane[2] * z + plane[3] < 0.0 {
                return false;
            }
        }
        true
    }
}
