use std::collections::HashSet;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nephele_assets::{TextureArray, UiAtlas};
use nephele_render::{build_world_mesh, Camera, Environment, Renderer};
use nephele_world::{ChunkGenerator, Dimension, World};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

const MOUSE_SENSITIVITY: f32 = 0.0022;
const TITLE_UPDATE: f32 = 0.4;

pub struct ViewerOptions {
    pub radius: i32,
    pub assets_dir: PathBuf,
    pub speed: f32,
    pub seed: i64,
    pub dimension: Dimension,
}

pub fn run(options: ViewerOptions) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut viewer = Viewer::new(options);
    event_loop.run_app(&mut viewer)?;
    match viewer.error.take() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

struct Viewer {
    options: ViewerOptions,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    camera: Camera,
    world: World,
    spawn: glam::Vec3,
    keys: HashSet<KeyCode>,
    mouse_captured: bool,
    last_cursor: Option<PhysicalPosition<f64>>,
    last_frame: Instant,
    title_timer: f32,
    frames: u32,
    error: Option<Box<dyn Error>>,
}

impl Viewer {
    fn new(options: ViewerOptions) -> Self {
        let camera = Camera {
            speed: options.speed,
            ..Camera::default()
        };
        Self {
            options,
            window: None,
            renderer: None,
            camera,
            world: World::new(0),
            spawn: glam::Vec3::ZERO,
            keys: HashSet::new(),
            mouse_captured: false,
            last_cursor: None,
            last_frame: Instant::now(),
            title_timer: 0.0,
            frames: 0,
            error: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, err: impl Error + 'static) {
        self.error = Some(Box::new(err));
        event_loop.exit();
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title("Nephele")
            .with_inner_size(LogicalSize::new(1280.0, 720.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => return self.fail(event_loop, err),
        };

        let textures = match TextureArray::load_from_dir(&self.options.assets_dir) {
            Ok(textures) => textures,
            Err(err) => return self.fail(event_loop, err),
        };
        println!(
            "Textures: {} tiles in {} layers ({}x{} px each, {} unresolved)",
            textures.tile_count(),
            textures.layer_count(),
            textures.layer_size(),
            textures.layer_size(),
            textures.unresolved_names().len()
        );

        let size = window.inner_size();
        let mut renderer = match Renderer::new(
            window.clone(),
            size.width,
            size.height,
            &textures,
            &UiAtlas::blank(),
            None,
        ) {
            Ok(renderer) => renderer,
            Err(err) => return self.fail(event_loop, err),
        };

        let mut environment = Environment::default();
        if let Ok(value) = std::env::var("NEPHELE_TIME") {
            if let Ok(ticks) = value.parse::<f32>() {
                environment.time_of_day = ticks.rem_euclid(24000.0);
            }
        }
        renderer.set_environment(environment);
        if let Ok(value) = std::env::var("NEPHELE_PITCH") {
            if let Ok(degrees) = value.parse::<f32>() {
                self.camera.pitch = degrees.to_radians().clamp(-1.55, 1.55);
            }
        }
        if let Ok(value) = std::env::var("NEPHELE_YAW") {
            if let Ok(degrees) = value.parse::<f32>() {
                self.camera.yaw = degrees.to_radians();
            }
        }

        let seed = self.options.seed;
        let dimension = self.options.dimension;
        let mut generator = ChunkGenerator::new(dimension, seed);
        self.world = World::new(dimension.id());
        // Two stages, as vanilla: `provide_chunk` per chunk, then the population
        // pass once a chunk's 2x2 neighbourhood exists.
        for cx in -self.options.radius..=self.options.radius {
            for cz in -self.options.radius..=self.options.radius {
                self.world.insert(generator.provide_chunk(cx, cz));
                generator.populate(&mut self.world, cx, cz);
            }
        }
        let mesh = build_world_mesh(&self.world, &textures);
        println!(
            "{}: {} chunks, {} vertices, {} triangles",
            dimension.label(),
            self.world.loaded_chunks(),
            mesh.vertices.len(),
            mesh.indices.len() / 3
        );
        renderer.set_mesh(&mesh);

        let spawn_height = self.world.height_at(8, 8).unwrap_or(64) as f32;
        self.spawn = glam::Vec3::new(8.5, spawn_height + 9.0, 8.5);
        self.camera.position = glam::Vec3::new(8.5, spawn_height + 9.0, 8.5);
        self.camera.set_aspect(size.width, size.height);

        self.renderer = Some(renderer);
        self.window = Some(window);
        self.last_frame = Instant::now();
    }

    fn set_mouse_capture(&mut self, captured: bool) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.mouse_captured = captured;
        if captured {
            let _ = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            window.set_cursor_visible(false);
            self.last_cursor = None;
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
    }

    fn frame(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        let now = Instant::now();
        let delta = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        let mut forward = 0.0;
        let mut strafe = 0.0;
        let mut up = 0.0;
        if self.keys.contains(&KeyCode::KeyW) {
            forward += 1.0;
        }
        if self.keys.contains(&KeyCode::KeyS) {
            forward -= 1.0;
        }
        if self.keys.contains(&KeyCode::KeyD) {
            strafe += 1.0;
        }
        if self.keys.contains(&KeyCode::KeyA) {
            strafe -= 1.0;
        }
        if self.keys.contains(&KeyCode::Space) {
            up += 1.0;
        }
        if self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight) {
            up -= 1.0;
        }
        if forward != 0.0 || strafe != 0.0 || up != 0.0 {
            self.camera.fly(forward, strafe, up, delta);
        }

        renderer.draw(&self.camera);

        self.frames += 1;
        self.title_timer += delta;
        if self.title_timer >= TITLE_UPDATE {
            let fps = self.frames as f32 / self.title_timer;
            window.set_title(&format!(
                "Nephele â€” {} chunks â€” {:.0} fps â€” {:.1} {:.1} {:.1}",
                self.world.loaded_chunks(),
                fps,
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z
            ));
            self.title_timer = 0.0;
            self.frames = 0;
        }
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.init(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
                self.camera.set_aspect(size.width, size.height);
            }
            WindowEvent::Focused(false) => {
                self.keys.clear();
                if self.mouse_captured {
                    self.set_mouse_capture(false);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match (code, event.state) {
                        (KeyCode::Escape, ElementState::Pressed) => {
                            if self.mouse_captured {
                                self.set_mouse_capture(false);
                            } else {
                                event_loop.exit();
                            }
                        }
                        (KeyCode::KeyR, ElementState::Pressed) => {
                            // Recentre the camera above spawn.
                            self.camera.position = self.spawn;
                        }
                        (_, ElementState::Pressed) => {
                            self.keys.insert(code);
                        }
                        (_, ElementState::Released) => {
                            self.keys.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Right {
                    self.set_mouse_capture(state == ElementState::Pressed);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.mouse_captured {
                    if let Some(last) = self.last_cursor {
                        let dx = (position.x - last.x) as f32;
                        let dy = (position.y - last.y) as f32;
                        self.camera
                            .rotate(dx * MOUSE_SENSITIVITY, -dy * MOUSE_SENSITIVITY);
                    }
                    self.last_cursor = Some(position);
                }
            }
            WindowEvent::RedrawRequested => {
                self.frame();
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}
