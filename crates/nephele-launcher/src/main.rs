//! Nephele Client's launcher.
//!
//! Draws with the same immediate-mode UI the in-game menus use, so the launcher
//! and the title screen share a look. The game itself runs as a **child
//! process**: a crash in the client then leaves the launcher alive to say so,
//! and the launcher never has to share a window with it.
//!
//! Resources come from the player's own 1.8.9 install via `nephele-prepare`,
//! which is run from here the first time and again whenever the prepared folder
//! is missing.

use std::error::Error;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

use nephele_assets::{TextureArray, UiAtlas};
use nephele_render::{Camera, MeshData, Rect, Renderer, UiBatch};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const WINDOW_TITLE: &str = "Nephele Client";
const LOGICAL_WIDTH: f64 = 1000.0;
const LOGICAL_HEIGHT: f64 = 620.0;

// Palette: a cool slate background with a single warm accent, so the Play
// button is the only thing that draws the eye.
const BG_TOP: [f32; 4] = [0.043, 0.055, 0.086, 1.0];
const BG_BOTTOM: [f32; 4] = [0.078, 0.094, 0.141, 1.0];
const PANEL: [f32; 4] = [0.098, 0.114, 0.161, 1.0];
const PANEL_LIGHT: [f32; 4] = [0.125, 0.145, 0.200, 1.0];
const BORDER: [f32; 4] = [0.16, 0.19, 0.25, 1.0];
const ACCENT: [f32; 4] = [0.98, 0.66, 0.26, 1.0];
const ACCENT_HOVER: [f32; 4] = [1.0, 0.74, 0.36, 1.0];
const ACCENT_DIM: [f32; 4] = [0.45, 0.32, 0.14, 1.0];
const WHITE: [f32; 4] = [0.92, 0.94, 0.97, 1.0];
const GREY: [f32; 4] = [0.55, 0.60, 0.68, 1.0];
const DIM: [f32; 4] = [0.36, 0.40, 0.47, 1.0];

/// Gradient bands; the UI has no gradient primitive, so the backdrop is drawn
/// as a stack of flat strips.
const BANDS: usize = 40;

/// What the launcher is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activity {
    Idle,
    Play,
    Demo,
}

struct Launcher {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    cursor: (f32, f32),
    camera: Camera,
    activity: Activity,
    child: Option<Child>,
    status: String,
    username: String,
    last_frame: Instant,
    frames: u32,
    fps_timer: f32,
    error: Option<Box<dyn Error>>,
}

/// Screen-space placement, recomputed every frame so the window can be resized.
struct Layout {
    play: Rect,
    demo: Rect,
    quit: Rect,
    card: Rect,
}

fn layout(width: f32, height: f32) -> Layout {
    let card_w = width.min(560.0) - 80.0;
    let card_h = 300.0;
    let card_x = (width - card_w) / 2.0;
    let card_y = (height - card_h) / 2.0 + 20.0;

    let play_w = card_w - 80.0;
    let play = Rect::new(card_x + 40.0, card_y + 96.0, play_w, 74.0);
    let demo = Rect::new(card_x + 40.0, card_y + 186.0, play_w, 50.0);
    let quit = Rect::new(width - 128.0, 26.0, 96.0, 34.0);

    Layout {
        play,
        demo,
        quit,
        card: Rect::new(card_x, card_y, card_w, card_h),
    }
}

impl Launcher {
    fn new(username: String) -> Self {
        Self {
            window: None,
            renderer: None,
            cursor: (-1.0, -1.0),
            camera: Camera::default(),
            activity: Activity::Idle,
            child: None,
            status: "Ready".to_string(),
            username,
            last_frame: Instant::now(),
            frames: 0,
            fps_timer: 0.0,
            error: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, err: impl Error + 'static) {
        self.error = Some(Box::new(err));
        event_loop.exit();
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_inner_size(LogicalSize::new(LOGICAL_WIDTH, LOGICAL_HEIGHT))
            .with_min_inner_size(LogicalSize::new(760.0, 480.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => return self.fail(event_loop, err),
        };

        let root = match minecraft_root() {
            Some(root) => root,
            None => {
                return self.fail(
                    event_loop,
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!(
                            "no 1.8.9 resources at {}; run nephele-prepare",
                            prepared_dir().display()
                        ),
                    ),
                )
            }
        };

        let textures_dir = root.join("textures");
        let ui_atlas = match UiAtlas::load(&textures_dir) {
            Ok(atlas) => atlas,
            Err(err) => return self.fail(event_loop, err),
        };
        // The launcher draws no blocks, but the renderer owns a block array.
        let block_dir = textures_dir.join("blocks");
        let blocks = TextureArray::load_from_dir(&block_dir)
            .unwrap_or_else(|_| TextureArray::from_tiles(Vec::new()));

        let size = window.inner_size();
        let mut renderer = match Renderer::new(
            window.clone(),
            size.width,
            size.height,
            &blocks,
            &ui_atlas,
            None,
        ) {
            Ok(renderer) => renderer,
            Err(err) => return self.fail(event_loop, err),
        };
        renderer.set_mesh(&MeshData {
            vertices: Vec::new(),
            indices: Vec::new(),
        });
        self.camera.set_aspect(size.width, size.height);

        self.renderer = Some(renderer);
        self.window = Some(window);
        self.last_frame = Instant::now();
    }

    fn hovered(&self, rect: Rect) -> bool {
        rect.contains(self.cursor.0, self.cursor.1)
    }

    fn tick_child(&mut self) {
        if self.child.is_none() {
            return;
        }
        let exited = self
            .child
            .as_mut()
            .map(|child| child.try_wait())
            .unwrap_or(Ok(None));
        match exited {
            Ok(Some(status)) => {
                self.status = match status.code() {
                    Some(0) => "Game closed".to_string(),
                    Some(code) => format!("Game exited with code {code}"),
                    None => "Game closed".to_string(),
                };
                self.child = None;
                self.activity = Activity::Idle;
            }
            Ok(None) => {}
            Err(err) => {
                self.status = format!("Could not check on the game: {err}");
                self.child = None;
                self.activity = Activity::Idle;
            }
        }
    }

    fn launch(&mut self, activity: Activity) {
        if self.child.is_some() {
            return;
        }
        let (binary, args, label) = match activity {
            Activity::Play => (
                "nephele",
                vec!["play".to_string(), "-u".to_string(), self.username.clone()],
                "Launching Nephele...",
            ),
            Activity::Demo => (
                "nephele",
                vec!["view".to_string()],
                "Opening the demo world...",
            ),
            Activity::Idle => return,
        };
        let mut args = args;
        // Pass the assets explicitly: the game resolves them itself too, but
        // doing it here means the root the launcher found is the one used,
        // whatever working directory the child ends up with.
        if let Some(root) = minecraft_root() {
            args.push("--assets-dir".to_string());
            args.push(root.join("textures").join("blocks").display().to_string());
        }

        let Some(path) = sibling_binary(binary) else {
            self.status = format!("{binary} is not next to the launcher; build the workspace");
            return;
        };

        match Command::new(path)
            .args(&args)
            .current_dir(project_root())
            .spawn()
        {
            Ok(child) => {
                self.child = Some(child);
                self.activity = activity;
                self.status = label.to_string();
            }
            Err(err) => {
                self.status = format!("Could not start the game: {err}");
            }
        }
    }

    fn build_ui(&self, width: f32, height: f32) -> UiBatch {
        let mut ui = UiBatch::default();

        for band in 0..BANDS {
            let t = band as f32 / (BANDS - 1) as f32;
            let color = [
                BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t,
                BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t,
                BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t,
                1.0,
            ];
            let y = height * band as f32 / BANDS as f32;
            ui.rect(Rect::new(0.0, y, width, height / BANDS as f32 + 1.0), color);
        }

        // Top bar.
        ui.rect(Rect::new(0.0, 0.0, width, 64.0), PANEL);
        ui.rect(Rect::new(0.0, 64.0, width, 2.0), ACCENT_DIM);
        ui.text("NEPHELE", 28.0, 18.0, 3.0, WHITE);
        ui.text("CLIENT", 30.0, 46.0, 1.0, GREY);

        let quit_hover = self.hovered(layout(width, height).quit);
        let l = layout(width, height);
        ui.rect(l.quit, if quit_hover { PANEL_LIGHT } else { PANEL });
        ui.rect_outline(l.quit, 1.0, BORDER);
        ui.text_centered(
            "QUIT",
            l.quit.x + l.quit.w / 2.0,
            l.quit.y + 13.0,
            1.0,
            GREY,
        );

        // Hero card.
        ui.rect(l.card, PANEL);
        ui.rect_outline(l.card, 1.0, BORDER);
        ui.text_centered(
            "MINECRAFT 1.8.9",
            l.card.x + l.card.w / 2.0,
            l.card.y + 30.0,
            2.0,
            WHITE,
        );
        ui.text_centered(
            "A client written from scratch in Rust",
            l.card.x + l.card.w / 2.0,
            l.card.y + 58.0,
            1.0,
            GREY,
        );

        let busy = self.child.is_some();
        let play_hover = self.hovered(l.play) && !busy;
        let play_color = if busy {
            ACCENT_DIM
        } else if play_hover {
            ACCENT_HOVER
        } else {
            ACCENT
        };
        ui.rect(l.play, play_color);
        ui.text_centered(
            if busy { "RUNNING" } else { "PLAY" },
            l.play.x + l.play.w / 2.0,
            l.play.y + 24.0,
            3.0,
            [0.06, 0.07, 0.10, 1.0],
        );

        let demo_hover = self.hovered(l.demo) && !busy;
        ui.rect(l.demo, if demo_hover { PANEL_LIGHT } else { PANEL });
        ui.rect_outline(l.demo, 1.0, BORDER);
        ui.text_centered(
            "VIEW DEMO WORLD",
            l.demo.x + l.demo.w / 2.0,
            l.demo.y + 17.0,
            1.0,
            if demo_hover { WHITE } else { GREY },
        );

        // Account chip and status line.
        ui.text(
            &format!("Offline player: {}", self.username),
            28.0,
            height - 56.0,
            1.0,
            GREY,
        );
        ui.text(&self.status, 28.0, height - 34.0, 1.0, DIM);
        ui.text(
            &format!(
                "v{} - not affiliated with Mojang",
                env!("CARGO_PKG_VERSION")
            ),
            width - 330.0,
            height - 34.0,
            1.0,
            DIM,
        );

        ui
    }

    fn frame(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let (width, height) = match self.renderer.as_ref() {
            Some(renderer) => renderer.size(),
            None => return,
        };

        self.tick_child();

        let now = Instant::now();
        let delta = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        let ui = self.build_ui(width as f32, height as f32);
        let empty = UiBatch::default();
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        renderer.set_ui(&empty, &empty, &ui);
        renderer.draw(&self.camera);

        self.frames += 1;
        self.fps_timer += delta;
        if self.fps_timer >= 0.5 {
            let fps = self.frames as f32 / self.fps_timer;
            window.set_title(&format!("{WINDOW_TITLE} - {fps:.0} fps"));
            self.fps_timer = 0.0;
            self.frames = 0;
        }
    }

    fn click(&mut self) {
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let (width, height) = renderer.size();
        let l = layout(width as f32, height as f32);
        let (x, y) = self.cursor;

        if l.quit.contains(x, y) {
            std::process::exit(0);
        }
        if self.child.is_none() && l.play.contains(x, y) {
            self.launch(Activity::Play);
        } else if self.child.is_none() && l.demo.contains(x, y) {
            self.launch(Activity::Demo);
        }
    }
}

impl ApplicationHandler for Launcher {
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
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left && state == ElementState::Pressed {
                    self.click();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if let winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Escape) =
                        event.physical_key
                    {
                        event_loop.exit();
                    }
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

/// The directory `nephele-prepare` writes into, and where the prepared
/// resources live.
///
/// Found from the current directory first, then by walking up from the
/// executable, so launching from Explorer (which sets the working directory to
/// the binary's folder) resolves the same tree as `cargo run`.
fn project_root() -> PathBuf {
    fn looks_like_root(dir: &std::path::Path) -> bool {
        if dir.join(".local/prepared").is_dir() {
            return true;
        }
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .map(|manifest| manifest.contains("[workspace]"))
            .unwrap_or(false)
    }

    if let Ok(cwd) = std::env::current_dir() {
        if looks_like_root(&cwd) {
            return cwd;
        }
    }

    let mut candidate = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from));
    while let Some(dir) = candidate {
        if looks_like_root(&dir) {
            return dir;
        }
        candidate = dir.parent().map(PathBuf::from);
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn prepared_dir() -> PathBuf {
    project_root().join(".local/prepared/1.8.9/assets/minecraft")
}

/// The prepared resources, or the legacy layout, or `NEPHELE_MINECRAFT_ASSETS`.
fn minecraft_root() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("NEPHELE_MINECRAFT_ASSETS") {
        let path = PathBuf::from(explicit);
        if path.is_dir() {
            return Some(path);
        }
    }
    [
        prepared_dir(),
        project_root().join("assets/vanilla/1.8.9/assets/minecraft"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_dir())
}

/// A program built alongside this one, in the same target directory.
fn sibling_binary(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = if cfg!(windows) {
        dir.join(format!("{name}.exe"))
    } else {
        dir.join(name)
    };
    candidate.is_file().then_some(candidate)
}

/// Runs `nephele-prepare` with the console attached, so the player can see the
/// verification progress. Returns whether resources exist afterwards.
fn ensure_resources() -> bool {
    if minecraft_root().is_some() {
        return true;
    }
    let Some(prepare) = sibling_binary("nephele-prepare") else {
        eprintln!("nephele-launcher: no resources and nephele-prepare is missing");
        return false;
    };
    println!("nephele-launcher: building resources from your Minecraft install");
    let status = Command::new(prepare)
        .current_dir(project_root())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    match status {
        Ok(status) if status.success() => minecraft_root().is_some(),
        Ok(status) => {
            eprintln!("nephele-launcher: nephele-prepare exited with {status}");
            false
        }
        Err(err) => {
            eprintln!("nephele-launcher: could not run nephele-prepare: {err}");
            false
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nephele_launcher=info".into()),
        )
        .with_target(false)
        .without_time()
        .init();

    if !ensure_resources() {
        return Err("resources are not prepared".into());
    }

    let username = std::env::args()
        .skip_while(|arg| arg != "--username")
        .nth(1)
        .unwrap_or_else(|| "Nephele".to_string());

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut launcher = Launcher::new(username);
    event_loop.run_app(&mut launcher)?;

    match launcher.error.take() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f32 = 1000.0;
    const HEIGHT: f32 = 620.0;

    #[test]
    fn the_play_button_sits_inside_the_card() {
        let l = layout(WIDTH, HEIGHT);
        assert!(l.play.x >= l.card.x && l.play.x + l.play.w <= l.card.x + l.card.w);
        assert!(l.play.y >= l.card.y && l.play.y + l.play.h <= l.card.y + l.card.h);
        assert!(l.demo.y >= l.play.y + l.play.h, "demo must sit below play");
        assert!(l.demo.y + l.demo.h <= l.card.y + l.card.h);
    }

    #[test]
    fn the_buttons_never_overlap() {
        for (width, height) in [(760.0, 480.0), (1000.0, 620.0), (1920.0, 1080.0)] {
            let l = layout(width, height);
            let hit = |r: Rect, x: f32, y: f32| r.contains(x, y);
            for y in (0..height as i32).step_by(3) {
                for x in (0..width as i32).step_by(3) {
                    let (x, y) = (x as f32, y as f32);
                    let count = [hit(l.play, x, y), hit(l.demo, x, y), hit(l.quit, x, y)]
                        .into_iter()
                        .filter(|hit| *hit)
                        .count();
                    assert!(count <= 1, "two buttons overlap at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn the_card_is_horizontally_centred() {
        for width in [760.0f32, 1000.0, 1600.0] {
            let l = layout(width, HEIGHT);
            let left = l.card.x;
            let right = width - (l.card.x + l.card.w);
            assert!((left - right).abs() < 0.01, "{left} vs {right}");
        }
    }

    #[test]
    fn the_interface_emits_geometry() {
        let launcher = Launcher::new("Tester".to_string());
        let ui = launcher.build_ui(WIDTH, HEIGHT);
        assert!(!ui.is_empty());
        assert_eq!(ui.indices.len() % 6, 0);
        assert_eq!(ui.vertices.len(), ui.indices.len() / 6 * 4);
    }

    #[test]
    fn hover_only_hits_the_button_under_the_cursor() {
        let mut launcher = Launcher::new("Tester".to_string());
        let l = layout(WIDTH, HEIGHT);
        launcher.cursor = (l.play.x + 4.0, l.play.y + 4.0);
        assert!(launcher.hovered(l.play));
        assert!(!launcher.hovered(l.demo));
        assert!(!launcher.hovered(l.quit));
    }
}
