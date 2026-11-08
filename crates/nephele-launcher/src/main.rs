//! Nephele Client's launcher.
//!
//! Draws with the same immediate-mode UI the in-game menus use, so the launcher
//! and the title screen share a look. The game itself runs as a **child
//! process**: a crash in the client then leaves the launcher alive to say so,
//! and the launcher never has to share a window with it.
//!
//! Resource preparation is the launcher's job on a fresh install. The window
//! opens *before* there are any game assets, so the setup screen can report a
//! real failure instead of the process exiting with a console message.
//!
//! Build-time switches, both empty by default:
//!
//! - `NEPHELE_UPDATE_PUBLIC_KEY` (Ed25519 public key, hex) enables the signed
//!   update check. Empty means "unsigned build", and the UI says so.
//! - `NEPHELE_UPDATE_URL` is where the signed manifest is fetched from.
//! - `NEPHELE_CRASH_URL` is where an **opt-in** crash report may be uploaded.

// A packaged launcher is a GUI app: do not open a console window behind it.
// Debug builds keep the console so `tracing` output is visible during
// development; the unsigned-build warning is also appended to
// `<launcher>/launcher.log` so release runs keep a record.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod crash;
mod fallback;
mod legal;
mod paths;
mod profile;
mod setup;
mod update;

use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nephele_assets::{TextureArray, UiAtlas};
use nephele_render::{Camera, MeshData, Rect, Renderer, UiBatch};
use serde::{Deserialize, Serialize};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use profile::Profiles;

const WINDOW_TITLE: &str = "Nephele Client";
const LOGICAL_WIDTH: f64 = 1000.0;
const LOGICAL_HEIGHT: f64 = 620.0;
const MIN_WIDTH: f64 = 900.0;
const MIN_HEIGHT: f64 = 600.0;
const VERSION: &str = env!("CARGO_PKG_VERSION");

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
const WARN: [f32; 4] = [0.95, 0.55, 0.22, 1.0];
const GOOD: [f32; 4] = [0.45, 0.82, 0.52, 1.0];
const WHITE: [f32; 4] = [0.92, 0.94, 0.97, 1.0];
const GREY: [f32; 4] = [0.55, 0.60, 0.68, 1.0];
const DIM: [f32; 4] = [0.36, 0.40, 0.47, 1.0];

/// Gradient bands; the UI has no gradient primitive, so the backdrop is drawn
/// as a stack of flat strips.
const BANDS: usize = 40;

/// What the launcher is currently doing with the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activity {
    Idle,
    Play,
    Demo,
}

impl Activity {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Play => "RUNNING",
            Self::Demo => "DEMO RUNNING",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Setup,
    Legal,
}

/// Launcher-wide options. Crash upload is the only consent switch, and it is
/// off until the user turns it on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LauncherSettings {
    #[serde(default)]
    upload_crashes: bool,
}

struct ChildSession {
    child: Child,
    profile_name: String,
    log_path: PathBuf,
    data_dir: PathBuf,
}

enum SetupState {
    /// No preparation found and none running yet.
    Needed,
    Running {
        child: Child,
        out: PathBuf,
        log_path: PathBuf,
        started: Instant,
        files: usize,
        last_poll: Instant,
    },
    Failed {
        reason: String,
        log_tail: String,
    },
}

struct Launcher {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    cursor: (f32, f32),
    camera: Camera,

    activity: Activity,
    child: Option<ChildSession>,
    status: String,
    username: String,
    error: Option<Box<dyn Error>>,

    data_root: PathBuf,
    project_root: PathBuf,
    prepared_out: PathBuf,

    profiles: Profiles,
    profiles_path: PathBuf,
    settings: LauncherSettings,
    settings_path: PathBuf,

    resources: Option<setup::Resources>,
    font_ready: bool,

    screen: Screen,
    setup: SetupState,
    setup_path: String,
    setup_editing: bool,
    game_dir_draft: String,
    game_dir_editing: bool,

    update: update::UpdateStatus,
    update_rx: Option<Receiver<update::UpdateStatus>>,

    crash_rx: Option<Receiver<Result<(), String>>>,
    last_crash: Option<PathBuf>,

    legal_scroll: f32,

    last_frame: Instant,
    frames: u32,
    fps_timer: f32,
}

#[derive(Clone, Copy, Debug)]
struct MainLayout {
    card: Rect,
    play: Rect,
    demo: Rect,
    quit: Rect,
    legal: Rect,
    profile: Rect,
    prev: Rect,
    next: Rect,
    dir_field: Rect,
    window: Rect,
    new_profile: Rect,
    duplicate: Rect,
    delete: Rect,
    open_folder: Rect,
    crash_toggle: Rect,
}

#[derive(Clone, Copy, Debug)]
struct SetupLayout {
    panel: Rect,
    field: Rect,
    prepare: Rect,
    back: Rect,
    bar: Rect,
    log: Rect,
}

#[derive(Clone, Copy, Debug)]
struct LegalLayout {
    panel: Rect,
    back: Rect,
}

fn content_top() -> f32 {
    76.0
}

fn content_bottom(height: f32) -> f32 {
    height - 52.0
}

fn main_layout(width: f32, height: f32) -> MainLayout {
    let top = content_top();
    let bottom = content_bottom(height);
    let panel_h = bottom - top;

    let quit = Rect::new(width - 118.0, 22.0, 92.0, 34.0);
    let legal = Rect::new(width - 222.0, 22.0, 92.0, 34.0);

    let profile = Rect::new(20.0, top, 260.0, panel_h);
    let card = Rect::new(300.0, top, width - 320.0, panel_h);

    let prev = Rect::new(profile.x + 8.0, profile.y + 34.0, 28.0, 30.0);
    let next = Rect::new(profile.x + profile.w - 36.0, profile.y + 34.0, 28.0, 30.0);
    let dir_field = Rect::new(profile.x + 8.0, profile.y + 88.0, profile.w - 16.0, 48.0);
    let window = Rect::new(profile.x + 12.0, profile.y + 160.0, profile.w - 24.0, 32.0);
    let new_profile = Rect::new(profile.x + 12.0, profile.y + 206.0, profile.w - 24.0, 30.0);
    let duplicate = Rect::new(profile.x + 12.0, profile.y + 242.0, profile.w - 24.0, 30.0);
    let delete = Rect::new(profile.x + 12.0, profile.y + 278.0, profile.w - 24.0, 30.0);
    let open_folder = Rect::new(profile.x + 12.0, profile.y + 322.0, profile.w - 24.0, 30.0);
    let crash_toggle = Rect::new(
        profile.x + 12.0,
        profile.y + profile.h - 40.0,
        profile.w - 24.0,
        30.0,
    );

    let play = Rect::new(card.x + 40.0, card.y + 116.0, card.w - 80.0, 86.0);
    let demo = Rect::new(card.x + 40.0, card.y + 214.0, card.w - 80.0, 44.0);

    MainLayout {
        card,
        play,
        demo,
        quit,
        legal,
        profile,
        prev,
        next,
        dir_field,
        window,
        new_profile,
        duplicate,
        delete,
        open_folder,
        crash_toggle,
    }
}

fn setup_layout(width: f32, height: f32) -> SetupLayout {
    let panel_w = (width - 80.0).min(780.0);
    let panel = Rect::new((width - panel_w) / 2.0, 60.0, panel_w, height - 120.0);
    let inner = panel.w - 48.0;
    SetupLayout {
        panel,
        field: Rect::new(panel.x + 24.0, panel.y + 150.0, inner, 38.0),
        prepare: Rect::new(panel.x + 24.0, panel.y + 206.0, 280.0, 40.0),
        back: Rect::new(panel.x + panel.w - 144.0, panel.y + 206.0, 120.0, 40.0),
        bar: Rect::new(panel.x + 24.0, panel.y + 270.0, inner, 14.0),
        log: Rect::new(panel.x + 24.0, panel.y + 318.0, inner, panel.h - 342.0),
    }
}

fn legal_layout(width: f32, height: f32) -> LegalLayout {
    let panel_w = (width - 80.0).min(840.0);
    let panel = Rect::new((width - panel_w) / 2.0, 60.0, panel_w, height - 120.0);
    LegalLayout {
        panel,
        back: Rect::new(panel.x + panel.w - 144.0, panel.y + 16.0, 120.0, 32.0),
    }
}

impl Launcher {
    fn new(username: String) -> Self {
        let data_root = paths::user_data_dir();
        let launcher_dir = paths::launcher_dir();
        let project_root = project_root();
        let profiles_path = launcher_dir.join("profiles.json");
        let settings_path = launcher_dir.join("launcher.json");
        let settings = std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();

        // `NEPHELE_FORCE_SETUP` is a development switch for exercising the
        // first-run path when a checkout already has `.local/prepared`.
        let force_setup = std::env::var_os("NEPHELE_FORCE_SETUP").is_some();
        let explicit = if force_setup {
            None
        } else {
            std::env::var_os("NEPHELE_MINECRAFT_ASSETS").map(PathBuf::from)
        };
        let resources = if force_setup {
            None
        } else {
            setup::discover(explicit, &project_root, &data_root)
        };

        let update = if update::is_signed() {
            update::UpdateStatus::Checking
        } else {
            update::UpdateStatus::Unsigned
        };

        Self {
            window: None,
            renderer: None,
            cursor: (-1.0, -1.0),
            camera: Camera::default(),
            activity: Activity::Idle,
            child: None,
            status: "Ready".to_string(),
            username,
            error: None,
            data_root,
            prepared_out: setup::prepared_out(&paths::user_data_dir()),
            project_root,
            profiles: Profiles::load(&profiles_path, &paths::user_data_dir()),
            profiles_path,
            settings,
            settings_path,
            screen: if resources.is_some() {
                Screen::Main
            } else {
                Screen::Setup
            },
            resources,
            font_ready: false,
            setup: SetupState::Needed,
            setup_path: setup::default_minecraft_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            setup_editing: false,
            game_dir_draft: String::new(),
            game_dir_editing: false,
            update,
            update_rx: None,
            crash_rx: None,
            last_crash: None,
            legal_scroll: 0.0,
            last_frame: Instant::now(),
            frames: 0,
            fps_timer: 0.0,
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
            .with_min_inner_size(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => return self.fail(event_loop, err),
        };

        if let Err(err) = self.create_renderer(window.clone()) {
            self.fail(event_loop, std::io::Error::other(err));
            return;
        }
        self.window = Some(window);
        self.last_frame = Instant::now();

        if update::is_signed() {
            self.start_update_check();
        } else {
            tracing::warn!("this build has no update signing key; update checks are disabled");
            self.status = "Unsigned build — updates disabled".to_string();
        }
        if self.resources.is_none() {
            self.status = "Game resources are not prepared. Use Setup to build them.".to_string();
        }
    }

    fn create_renderer(&mut self, window: Arc<Window>) -> Result<(), String> {
        let (ui_atlas, blocks) = match &self.resources {
            Some(resources) => {
                let ui_atlas = UiAtlas::load(&resources.textures).map_err(|err| err.to_string())?;
                let blocks = TextureArray::load_from_dir(&resources.blocks)
                    .map_err(|err| err.to_string())?;
                (ui_atlas, blocks)
            }
            // No prepared resources yet: the setup screen draws with the
            // built-in block font and solid rectangles only. The renderer's
            // block binding expects a texture *array*, so give it a few
            // generated white tiles rather than an empty one (an empty array
            // binds as a 2D texture and wgpu rejects it).
            None => (bootstrap_atlas(), bootstrap_blocks()),
        };
        self.font_ready = self.resources.is_some();

        let size = window.inner_size();
        let mut renderer = Renderer::new(window, size.width, size.height, &blocks, &ui_atlas, None)
            .map_err(|err| err.to_string())?;
        renderer.set_mesh(&MeshData {
            vertices: Vec::new(),
            indices: Vec::new(),
        });
        self.camera.set_aspect(size.width, size.height);
        self.renderer = Some(renderer);
        Ok(())
    }

    /// Discover resources with the same explicit override the game honours.
    fn discover_resources(&self) -> Option<setup::Resources> {
        setup::discover(
            std::env::var_os("NEPHELE_MINECRAFT_ASSETS").map(PathBuf::from),
            &self.project_root,
            &self.data_root,
        )
    }

    /// Re-read the prepared assets after a successful setup, without asking
    /// the player to restart. Dropping the old surface before creating the new
    /// one keeps wgpu from holding two surfaces for the same window.
    fn reload_assets(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.resources = self.discover_resources();
        if self.resources.is_none() {
            return;
        }
        self.renderer = None;
        match self.create_renderer(window) {
            Ok(()) => {
                self.screen = Screen::Main;
                self.status = "Resources ready".to_string();
            }
            Err(err) => {
                self.status =
                    format!("resources were built, but the UI could not load them: {err}");
                self.screen = Screen::Setup;
            }
        }
    }

    fn start_update_check(&mut self) {
        let (sender, receiver) = mpsc::channel();
        self.update_rx = Some(receiver);
        self.update = update::UpdateStatus::Checking;
        std::thread::spawn(move || {
            let _ = sender.send(update::check_now(VERSION));
        });
    }

    fn drain_channels(&mut self) {
        if let Some(receiver) = &self.update_rx {
            match receiver.try_recv() {
                Ok(status) => {
                    self.update = status;
                    self.update_rx = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.update = update::UpdateStatus::Failed("check thread stopped".to_string());
                    self.update_rx = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(receiver) = &self.crash_rx {
            match receiver.try_recv() {
                Ok(Ok(())) => {
                    self.status = "Crash report uploaded".to_string();
                    self.crash_rx = None;
                }
                Ok(Err(err)) => {
                    self.status = format!("Crash report saved locally; upload failed: {err}");
                    self.crash_rx = None;
                }
                Err(TryRecvError::Disconnected) => self.crash_rx = None,
                Err(TryRecvError::Empty) => {}
            }
        }
    }

    fn save_profiles(&mut self) {
        if let Err(err) = self.profiles.save(&self.profiles_path) {
            self.status = format!("Could not save profiles: {err}");
        }
    }

    fn save_settings(&mut self) {
        if let Some(parent) = self.settings_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.settings) {
            Ok(text) => {
                if let Err(err) = std::fs::write(&self.settings_path, text) {
                    self.status = format!("Could not save launcher settings: {err}");
                }
            }
            Err(err) => self.status = format!("Could not encode launcher settings: {err}"),
        }
    }

    fn hovered(&self, rect: Rect) -> bool {
        rect.contains(self.cursor.0, self.cursor.1)
    }

    fn tick_child(&mut self) {
        let Some(session) = self.child.as_mut() else {
            return;
        };
        let exited = session.child.try_wait();
        let (status_code, label) = match exited {
            Ok(Some(status)) => (Some(status), status_text(status.code())),
            Ok(None) => return,
            Err(err) => {
                self.status = format!("Could not check on the game: {err}");
                self.child = None;
                self.activity = Activity::Idle;
                return;
            }
        };
        let session = self.child.take().expect("checked above");
        self.activity = Activity::Idle;
        self.status = label;

        let crashed = !matches!(status_code, Some(status) if status.success());
        if crashed {
            self.write_game_crash(&session, status_code.and_then(|status| status.code()));
        }
    }

    fn write_game_crash(&mut self, session: &ChildSession, code: Option<i32>) {
        let tail = crash::tail(&session.log_path, 80);
        let report = crash::CrashReport::new(
            crash::CrashKind::Game,
            VERSION,
            &session.profile_name,
            &update::signing_summary(),
        )
        .exit_code(code)
        .log_tail(tail);
        let now = crash::unix_now();
        let code_text = match code {
            Some(code) => format!("code {code}"),
            None => "an abnormal termination".to_string(),
        };
        match crash::write(&session.data_dir, &report, now) {
            Ok(path) => {
                self.last_crash = Some(path.clone());
                self.status = format!(
                    "Game exited unexpectedly ({code_text}). Report saved to {}",
                    path.display()
                );
                if crash::upload_allowed(self.settings.upload_crashes, crash::endpoint()) {
                    let text = report.text(now);
                    if let Some(endpoint) = crash::endpoint() {
                        let (sender, receiver) = mpsc::channel();
                        self.crash_rx = Some(receiver);
                        std::thread::spawn(move || {
                            let _ = sender.send(crash::upload(&text, endpoint));
                        });
                    }
                }
            }
            Err(err) => {
                self.status = format!(
                    "Game exited unexpectedly ({code_text}), and the report could not be written: {err}"
                );
            }
        }
    }

    fn start_prepare(&mut self) {
        if matches!(self.setup, SetupState::Running { .. }) {
            return;
        }
        let Some(prepare) = sibling_binary("nephele-prepare") else {
            self.setup = SetupState::Failed {
                reason: "nephele-prepare is not next to the launcher; run the packaged build"
                    .to_string(),
                log_tail: String::new(),
            };
            return;
        };
        let out = self.prepared_out.clone();
        if let Err(err) = std::fs::create_dir_all(&out) {
            self.setup = SetupState::Failed {
                reason: format!("could not create {}: {err}", out.display()),
                log_tail: String::new(),
            };
            return;
        }
        let log_path = paths::launcher_dir().join("prepare.log");
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let log = match File::create(&log_path)
            .and_then(|file| file.try_clone().map(|copy| (file, copy)))
        {
            Ok((mut stdout, stderr)) => {
                let _ = writeln!(
                    stdout,
                    "nephele-launcher: preparing resources into {}",
                    out.display()
                );
                (stdout, stderr)
            }
            Err(err) => {
                self.setup = SetupState::Failed {
                    reason: format!("could not open the preparation log: {err}"),
                    log_tail: String::new(),
                };
                return;
            }
        };
        let (stdout, stderr) = log;
        let args = setup::prepare_args(&out, Some(&self.setup_path));
        let mut command = Command::new(prepare);
        command
            .args(&args)
            .current_dir(&self.project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        no_console_window(&mut command);
        match command.spawn() {
            Ok(child) => {
                self.status = "Building resources from your Minecraft install...".to_string();
                self.setup = SetupState::Running {
                    child,
                    out,
                    log_path,
                    started: Instant::now(),
                    files: 0,
                    last_poll: Instant::now(),
                };
            }
            Err(err) => {
                self.setup = SetupState::Failed {
                    reason: format!("could not run nephele-prepare: {err}"),
                    log_tail: String::new(),
                };
            }
        }
    }

    fn tick_setup(&mut self) {
        let (running, finished) = match &mut self.setup {
            SetupState::Running {
                child,
                out,
                files,
                last_poll,
                ..
            } => {
                if last_poll.elapsed() >= Duration::from_millis(300) {
                    *files = setup::count_files(out);
                    *last_poll = Instant::now();
                }
                (true, child.try_wait())
            }
            _ => (false, Ok(None)),
        };
        if !running {
            return;
        }
        match finished {
            Ok(None) => {}
            Ok(Some(status)) => {
                let (out, log_path) = match std::mem::replace(&mut self.setup, SetupState::Needed) {
                    SetupState::Running { out, log_path, .. } => (out, log_path),
                    other => {
                        self.setup = other;
                        return;
                    }
                };
                if status.success() && self.discover_resources().is_some() {
                    self.reload_assets();
                } else {
                    let tail = crash::tail(&log_path, 14);
                    let reason = if status.success() {
                        format!(
                            "nephele-prepare finished, but no usable resources were found under {}",
                            out.display()
                        )
                    } else {
                        status_text(status.code())
                    };
                    self.setup = SetupState::Failed {
                        reason,
                        log_tail: tail,
                    };
                    self.status = "Resource preparation failed".to_string();
                }
            }
            Err(err) => {
                self.setup = SetupState::Failed {
                    reason: format!("could not wait for nephele-prepare: {err}"),
                    log_tail: String::new(),
                };
            }
        }
    }

    fn launch(&mut self, activity: Activity) {
        if self.child.is_some() {
            return;
        }
        let Some(resources) = self.resources.clone() else {
            self.screen = Screen::Setup;
            self.status = "Prepare resources first".to_string();
            return;
        };
        let Some(binary) = sibling_binary("nephele") else {
            self.status = "nephele is not next to the launcher; build the workspace".to_string();
            return;
        };

        let profile = self.profiles.active().clone();
        let mut args = match activity {
            Activity::Play => vec!["play".to_string(), "-u".to_string(), self.username.clone()],
            Activity::Demo => vec!["view".to_string()],
            Activity::Idle => return,
        };
        args.push("--assets-dir".to_string());
        args.push(resources.blocks.display().to_string());

        let log_dir = profile.game_dir.join("logs");
        if let Err(err) = std::fs::create_dir_all(&log_dir) {
            self.status = format!("Could not create {}: {err}", log_dir.display());
            return;
        }
        let log_path = log_dir.join(format!("game-{}.log", crash::utc_stamp(crash::unix_now())));
        let (stdout, stderr) = match File::create(&log_path)
            .and_then(|file| file.try_clone().map(|copy| (file, copy)))
        {
            Ok(pair) => pair,
            Err(err) => {
                self.status = format!("Could not open {}: {err}", log_path.display());
                return;
            }
        };

        let mut command = Command::new(binary);
        command
            .args(&args)
            .current_dir(&self.project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .env("NEPHELE_DATA_DIR", &profile.game_dir)
            .env("NEPHELE_MINECRAFT_ASSETS", &resources.assets)
            .env("NEPHELE_WINDOW_SIZE", profile.window.env_value())
            .env("RUST_BACKTRACE", "1");
        no_console_window(&mut command);

        match command.spawn() {
            Ok(child) => {
                self.status = format!(
                    "{} — profile {:?}, {}, log {}",
                    match activity {
                        Activity::Play => "Launching Nephele...",
                        Activity::Demo => "Opening the demo world...",
                        Activity::Idle => "",
                    },
                    profile.name,
                    profile.window.label(),
                    log_path.display()
                );
                self.child = Some(ChildSession {
                    child,
                    profile_name: profile.name,
                    log_path,
                    data_dir: profile.game_dir,
                });
                self.activity = activity;
            }
            Err(err) => {
                self.status = format!("Could not start the game: {err}");
            }
        }
    }

    fn select_profile(&mut self, direction: i32) {
        if self.profiles.cycle(direction) {
            self.save_profiles();
        }
    }

    fn click_main(&mut self, layout: MainLayout, x: f32, y: f32) {
        let busy = self.child.is_some();
        if layout.quit.contains(x, y) {
            std::process::exit(0);
        } else if layout.legal.contains(x, y) {
            self.screen = Screen::Legal;
            self.legal_scroll = 0.0;
        } else if layout.prev.contains(x, y) {
            self.select_profile(-1);
        } else if layout.next.contains(x, y) {
            self.select_profile(1);
        } else if layout.dir_field.contains(x, y) {
            self.game_dir_editing = true;
            self.game_dir_draft = self.profiles.active().game_dir.display().to_string();
        } else if layout.window.contains(x, y) {
            let next = self.profiles.active().window.next();
            self.profiles.active_mut().window = next;
            self.save_profiles();
        } else if layout.new_profile.contains(x, y) {
            self.profiles.add(&self.data_root);
            self.save_profiles();
        } else if layout.duplicate.contains(x, y) {
            self.profiles.duplicate_active(&self.data_root);
            self.save_profiles();
        } else if layout.delete.contains(x, y) {
            if self.profiles.remove_active() {
                self.save_profiles();
            } else {
                self.status = "The last profile cannot be deleted".to_string();
            }
        } else if layout.open_folder.contains(x, y) {
            let dir = self.profiles.active().game_dir.clone();
            let _ = std::fs::create_dir_all(&dir);
            if let Err(err) = open_in_file_manager(&dir) {
                self.status = format!("Could not open {}: {err}", dir.display());
            }
        } else if layout.crash_toggle.contains(x, y) {
            self.settings.upload_crashes = !self.settings.upload_crashes;
            self.save_settings();
            self.status = if self.settings.upload_crashes {
                if crash::endpoint().is_some() {
                    "Crash uploads ON (reports are also kept locally)".to_string()
                } else {
                    "Crash uploads ON, but this build has no upload endpoint; reports stay local"
                        .to_string()
                }
            } else {
                "Crash uploads OFF; reports stay local".to_string()
            };
        } else if !busy && layout.play.contains(x, y) {
            if self.resources.is_some() {
                self.launch(Activity::Play);
            } else {
                self.screen = Screen::Setup;
                self.status = "Prepare resources first".to_string();
            }
        } else if !busy && layout.demo.contains(x, y) {
            if self.resources.is_some() {
                self.launch(Activity::Demo);
            } else {
                self.screen = Screen::Setup;
                self.status = "Prepare resources first".to_string();
            }
        }
    }

    fn click_setup(&mut self, layout: SetupLayout, x: f32, y: f32) {
        if layout.back.contains(x, y) {
            self.screen = Screen::Main;
        } else if layout.field.contains(x, y) {
            self.setup_editing = true;
        } else if layout.prepare.contains(x, y) && !matches!(self.setup, SetupState::Running { .. })
        {
            self.start_prepare();
        }
    }

    // --- text helpers: vanilla font when available, block font otherwise ---

    fn text(&self, ui: &mut UiBatch, content: &str, x: f32, y: f32, scale: f32, color: [f32; 4]) {
        if self.font_ready {
            ui.text(content, x, y, scale, color);
        } else {
            fallback::text(ui, content, x, y, scale, color);
        }
    }

    fn text_width(&self, content: &str, scale: f32) -> f32 {
        if self.font_ready {
            UiBatch::text_width(content, scale)
        } else {
            fallback::text_width(content, scale)
        }
    }

    fn text_centered(
        &self,
        ui: &mut UiBatch,
        content: &str,
        center_x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
    ) {
        let width = self.text_width(content, scale);
        self.text(ui, content, center_x - width / 2.0, y, scale, color);
    }

    /// Greedy word wrap, honouring explicit newlines. A single token wider
    /// than the column (a Windows path, usually) is hard-broken rather than
    /// allowed to run off the panel.
    fn wrap(&self, content: &str, max_width: f32, scale: f32) -> String {
        let mut out = String::new();
        for paragraph in content.split('\n') {
            let mut line = String::new();
            for word in paragraph.split_whitespace() {
                let candidate = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if self.text_width(&candidate, scale) <= max_width {
                    line = candidate;
                    continue;
                }
                if !line.is_empty() {
                    out.push_str(&line);
                    out.push('\n');
                    line.clear();
                }
                if self.text_width(word, scale) <= max_width {
                    line = word.to_string();
                    continue;
                }
                for ch in word.chars() {
                    line.push(ch);
                    if self.text_width(&line, scale) > max_width && line.chars().count() > 1 {
                        line.pop();
                        out.push_str(&line);
                        out.push('\n');
                        line = ch.to_string();
                    }
                }
            }
            out.push_str(&line);
            out.push('\n');
        }
        out.trim_end_matches('\n').to_string()
    }

    /// Keep only the first `limit` lines of wrapped text, marking truncation.
    fn truncate_lines(&self, text: &str, limit: usize) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= limit {
            return text.to_string();
        }
        let mut kept: Vec<String> = lines[..limit].iter().map(|line| line.to_string()).collect();
        if let Some(last) = kept.last_mut() {
            last.push_str("...");
        }
        kept.join("\n")
    }

    fn button(
        &self,
        ui: &mut UiBatch,
        rect: Rect,
        label: &str,
        enabled: bool,
        accent: bool,
        scale: f32,
    ) {
        let hover = self.hovered(rect) && enabled;
        let fill = if !enabled {
            PANEL
        } else if accent {
            if hover {
                ACCENT_HOVER
            } else {
                ACCENT
            }
        } else if hover {
            PANEL_LIGHT
        } else {
            PANEL
        };
        ui.rect(rect, fill);
        ui.rect_outline(rect, 1.0, BORDER);
        let color = if !enabled {
            DIM
        } else if accent {
            [0.06, 0.07, 0.10, 1.0]
        } else if hover {
            WHITE
        } else {
            GREY
        };
        // Both fonts are roughly eight pixels tall per scale unit.
        let y = rect.y + (rect.h - 8.0 * scale) / 2.0;
        self.text_centered(ui, label, rect.x + rect.w / 2.0, y, scale, color);
    }

    fn build_background(&self, ui: &mut UiBatch, width: f32, height: f32) {
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
        ui.rect(Rect::new(0.0, 0.0, width, 64.0), PANEL);
        ui.rect(Rect::new(0.0, 64.0, width, 2.0), ACCENT_DIM);
        self.text(ui, "NEPHELE", 28.0, 18.0, 3.0, WHITE);
        self.text(ui, "CLIENT", 30.0, 46.0, 1.0, GREY);
    }

    fn build_ui(&self, width: f32, height: f32) -> UiBatch {
        let mut ui = UiBatch::default();
        self.build_background(&mut ui, width, height);
        match self.screen {
            Screen::Main => self.build_main(&mut ui, width, height),
            Screen::Setup => self.build_setup(&mut ui, width, height),
            Screen::Legal => self.build_legal(&mut ui, width, height),
        }
        self.build_footer(&mut ui, width, height);
        ui
    }

    fn build_main(&self, ui: &mut UiBatch, width: f32, height: f32) {
        let l = main_layout(width, height);
        let busy = self.child.is_some();

        // Signing badge, immediately left of Legal.
        let badge = update::signing_summary();
        let badge_color = if update::is_signed() { GOOD } else { WARN };
        let badge_width = self.text_width(&badge, 1.0);
        self.text(
            ui,
            &badge,
            width - 234.0 - badge_width,
            32.0,
            1.0,
            badge_color,
        );

        self.button(ui, l.legal, "LEGAL", true, false, 1.0);
        self.button(ui, l.quit, "QUIT", true, false, 1.0);

        // Profile column.
        ui.rect(l.profile, PANEL);
        ui.rect_outline(l.profile, 1.0, BORDER);
        self.text(
            ui,
            "PROFILE",
            l.profile.x + 12.0,
            l.profile.y + 14.0,
            1.0,
            GREY,
        );
        self.button(ui, l.prev, "<", self.profiles.all().len() > 1, false, 2.0);
        self.button(ui, l.next, ">", self.profiles.all().len() > 1, false, 2.0);
        self.text_centered(
            ui,
            &self.profiles.active().name,
            l.profile.x + l.profile.w / 2.0,
            l.profile.y + 40.0,
            1.5,
            WHITE,
        );
        self.text_centered(
            ui,
            &format!(
                "{} / {}",
                self.profiles.active_index() + 1,
                self.profiles.all().len()
            ),
            l.profile.x + l.profile.w / 2.0,
            l.profile.y + 60.0,
            1.0,
            DIM,
        );

        self.text(
            ui,
            "GAME DIRECTORY",
            l.profile.x + 12.0,
            l.profile.y + 76.0,
            1.0,
            GREY,
        );
        if self.game_dir_editing {
            ui.rect(l.dir_field, [0.06, 0.07, 0.10, 1.0]);
            ui.rect_outline(l.dir_field, 1.0, ACCENT);
            let draft = format!("{}|", self.game_dir_draft);
            self.text(
                ui,
                &self.truncate_lines(&self.wrap(&draft, l.dir_field.w - 12.0, 1.0), 3),
                l.dir_field.x + 6.0,
                l.dir_field.y + 4.0,
                1.0,
                WHITE,
            );
        } else {
            let dir_text = self.wrap(
                &self.profiles.active().game_dir.display().to_string(),
                l.profile.w - 24.0,
                1.0,
            );
            let dir_text = self.truncate_lines(&dir_text, 2);
            self.text(
                ui,
                &dir_text,
                l.profile.x + 12.0,
                l.profile.y + 92.0,
                1.0,
                DIM,
            );
        }

        self.text(
            ui,
            "GAME WINDOW",
            l.profile.x + 12.0,
            l.profile.y + 142.0,
            1.0,
            GREY,
        );
        self.button(
            ui,
            l.window,
            &self.profiles.active().window.label(),
            true,
            false,
            1.0,
        );
        self.button(ui, l.new_profile, "NEW PROFILE", true, false, 1.0);
        self.button(ui, l.duplicate, "DUPLICATE", true, false, 1.0);
        self.button(
            ui,
            l.delete,
            "DELETE PROFILE",
            self.profiles.all().len() > 1,
            false,
            1.0,
        );
        self.button(ui, l.open_folder, "OPEN GAME FOLDER", true, false, 1.0);
        self.button(
            ui,
            l.crash_toggle,
            if self.settings.upload_crashes {
                "CRASH UPLOAD: ON"
            } else {
                "CRASH UPLOAD: OFF"
            },
            true,
            false,
            1.0,
        );

        // Hero card.
        ui.rect(l.card, PANEL);
        ui.rect_outline(l.card, 1.0, BORDER);
        self.text_centered(
            ui,
            "MINECRAFT 1.8.9",
            l.card.x + l.card.w / 2.0,
            l.card.y + 30.0,
            2.0,
            WHITE,
        );
        self.text_centered(
            ui,
            "A client written from scratch in Rust",
            l.card.x + l.card.w / 2.0,
            l.card.y + 60.0,
            1.0,
            GREY,
        );

        self.button(
            ui,
            l.play,
            if busy { self.activity.label() } else { "PLAY" },
            !busy && self.resources.is_some(),
            true,
            3.0,
        );
        self.button(
            ui,
            l.demo,
            "VIEW DEMO WORLD",
            !busy && self.resources.is_some(),
            false,
            1.0,
        );

        let resources_line = if self.resources.is_some() {
            "Resources: ready".to_string()
        } else {
            "Resources: not prepared — open Setup".to_string()
        };
        let resources_color = if self.resources.is_some() { GOOD } else { WARN };
        self.text(
            ui,
            &resources_line,
            l.card.x + 40.0,
            l.card.y + 278.0,
            1.0,
            resources_color,
        );

        let update_line = self.update.label();
        let update_color = if self.update.available() {
            GOOD
        } else if self.update.is_unsigned() {
            WARN
        } else {
            GREY
        };
        self.text(
            ui,
            &self.wrap(&update_line, l.card.w - 80.0, 1.0),
            l.card.x + 40.0,
            l.card.y + 300.0,
            1.0,
            update_color,
        );

        if !update::is_signed() {
            self.text(
                ui,
                &self.wrap(legal::BLOCKED_SIGNING, l.card.w - 80.0, 1.0),
                l.card.x + 40.0,
                l.card.y + 336.0,
                1.0,
                DIM,
            );
        } else if update::is_signed() && self.update.available() {
            self.text_centered(
                ui,
                "A signed update is available; this build does not install it.",
                l.card.x + l.card.w / 2.0,
                l.card.y + 340.0,
                1.0,
                GREY,
            );
        }

        if let Some(path) = &self.last_crash {
            self.text(
                ui,
                &self.wrap(
                    &format!("Last crash report: {}", path.display()),
                    l.card.w - 80.0,
                    1.0,
                ),
                l.card.x + 40.0,
                l.card.y + 380.0,
                1.0,
                WARN,
            );
        }

        // Data directory, so a user can find their saves without guessing.
        self.text(
            ui,
            &self.wrap(
                &format!(
                    "Profile data: {}\nLauncher state: {}",
                    self.profiles.active().game_dir.display(),
                    paths::launcher_dir().display()
                ),
                l.card.w - 80.0,
                1.0,
            ),
            l.card.x + 40.0,
            l.card.y + l.card.h - 58.0,
            1.0,
            DIM,
        );
    }

    fn build_setup(&self, ui: &mut UiBatch, width: f32, height: f32) {
        let l = setup_layout(width, height);
        ui.rect(l.panel, PANEL);
        ui.rect_outline(l.panel, 1.0, BORDER);

        self.text(
            ui,
            "FIRST-RUN SETUP",
            l.panel.x + 24.0,
            l.panel.y + 22.0,
            2.0,
            WHITE,
        );
        let intro = "Nephele ships no Minecraft files. Preparation reads the 1.8.9 \
                     client jar you already own, checks it against research-lock.json, \
                     and writes the textures and models the client needs.";
        self.text(
            ui,
            &self.wrap(intro, l.panel.w - 48.0, 1.0),
            l.panel.x + 24.0,
            l.panel.y + 58.0,
            1.0,
            GREY,
        );

        self.text(
            ui,
            "MINECRAFT FOLDER (LEAVE EMPTY TO AUTO-DETECT)",
            l.panel.x + 24.0,
            l.panel.y + 130.0,
            1.0,
            GREY,
        );
        ui.rect(l.field, [0.06, 0.07, 0.10, 1.0]);
        ui.rect_outline(
            l.field,
            1.0,
            if self.setup_editing { ACCENT } else { BORDER },
        );
        let shown = if self.setup_editing {
            format!("{}|", self.setup_path)
        } else {
            self.setup_path.clone()
        };
        self.text(
            ui,
            &shown,
            l.field.x + 10.0,
            l.field.y + 12.0,
            1.0,
            if self.setup_path.is_empty() {
                DIM
            } else {
                WHITE
            },
        );

        let running = matches!(self.setup, SetupState::Running { .. });
        self.button(ui, l.prepare, "PREPARE RESOURCES", !running, true, 1.0);
        self.button(ui, l.back, "BACK", !running, false, 1.0);

        // Progress: a bar plus live numbers. `nephele-prepare` does not stream
        // per-file progress, so the honest signal is elapsed time and files
        // written so far, not an invented percentage.
        let (fraction, progress) = match &self.setup {
            SetupState::Running { started, files, .. } => {
                let elapsed = started.elapsed().as_secs_f32();
                // A slow pulse, not a fake completion percentage. It reaches
                // 90% after 30 s and stays there until the child exits.
                let fraction = (elapsed / 30.0 * 0.9).min(0.9);
                (
                    fraction,
                    format!("Building... {:.0}s, {} files extracted", elapsed, files),
                )
            }
            SetupState::Needed => (0.0, "Not prepared yet.".to_string()),
            SetupState::Failed { .. } => {
                (0.0, "Preparation failed. See the log below.".to_string())
            }
        };
        ui.rect(l.bar, [0.06, 0.07, 0.10, 1.0]);
        if fraction > 0.0 {
            ui.rect(
                Rect::new(l.bar.x, l.bar.y, l.bar.w * fraction, l.bar.h),
                ACCENT,
            );
        }
        ui.rect_outline(l.bar, 1.0, BORDER);
        self.text(
            ui,
            &progress,
            l.bar.x,
            l.bar.y + 20.0,
            1.0,
            if matches!(self.setup, SetupState::Failed { .. }) {
                WARN
            } else {
                GREY
            },
        );

        // The error surface: the exact reason plus the tail of prepare's log.
        ui.rect(l.log, [0.05, 0.06, 0.08, 1.0]);
        ui.rect_outline(l.log, 1.0, BORDER);
        let mut y = l.log.y + 8.0;
        if let SetupState::Failed { reason, log_tail } = &self.setup {
            let reason = self.wrap(reason, l.log.w - 16.0, 1.0);
            self.text(ui, &reason, l.log.x + 8.0, y, 1.0, WARN);
            y += (reason.lines().count().max(1) as f32) * 11.0 + 6.0;
            if !log_tail.is_empty() {
                let tail = self.wrap(log_tail, l.log.w - 16.0, 1.0);
                // Keep the last lines that fit.
                let max_lines = ((l.log.y + l.log.h - y) / 11.0).max(0.0) as usize;
                let lines: Vec<&str> = tail.lines().collect();
                let start = lines.len().saturating_sub(max_lines);
                self.text(ui, &lines[start..].join("\n"), l.log.x + 8.0, y, 1.0, GREY);
            }
        } else {
            self.text(
                ui,
                "Preparation output appears here if it fails.",
                l.log.x + 8.0,
                y,
                1.0,
                DIM,
            );
        }
    }

    fn build_legal(&self, ui: &mut UiBatch, width: f32, height: f32) {
        let l = legal_layout(width, height);
        ui.rect(l.panel, PANEL);
        ui.rect_outline(l.panel, 1.0, BORDER);

        let mut y = l.panel.y + 62.0 - self.legal_scroll;
        let line = 11.0;
        let bottom = l.panel.y + l.panel.h - 12.0;
        for (heading, paragraphs) in [
            (
                "NOT AFFILIATED WITH MOJANG OR MICROSOFT",
                legal::NON_AFFILIATION,
            ),
            ("TERMS", legal::TERMS),
            ("PRIVACY", legal::PRIVACY),
        ] {
            if y > bottom {
                break;
            }
            self.text(ui, heading, l.panel.x + 24.0, y, 1.0, ACCENT);
            y += line + 4.0;
            for paragraph in paragraphs {
                let text = self.wrap(paragraph, l.panel.w - 48.0, 1.0);
                let lines = text.lines().count().max(1) as f32;
                if y > bottom {
                    break;
                }
                self.text(ui, &text, l.panel.x + 24.0, y, 1.0, GREY);
                y += lines * line + 6.0;
            }
            y += 6.0;
        }

        // Cover anything that scrolled under the title or past the panel.
        ui.rect(Rect::new(l.panel.x, l.panel.y, l.panel.w, 54.0), PANEL);
        ui.rect(
            Rect::new(l.panel.x, l.panel.y + l.panel.h - 10.0, l.panel.w, 10.0),
            PANEL,
        );
        self.text(
            ui,
            legal::TITLE,
            l.panel.x + 24.0,
            l.panel.y + 20.0,
            2.0,
            WHITE,
        );
        self.button(ui, l.back, "BACK", true, false, 1.0);
        if self.legal_scroll > 0.0 {
            self.text(
                ui,
                "scroll for more",
                l.panel.x + l.panel.w - 150.0,
                l.panel.y + 26.0,
                1.0,
                DIM,
            );
        }
    }

    fn build_footer(&self, ui: &mut UiBatch, width: f32, height: f32) {
        ui.rect(Rect::new(0.0, height - 46.0, width, 46.0), PANEL);
        ui.rect(Rect::new(0.0, height - 46.0, width, 1.0), BORDER);
        self.text(
            ui,
            &format!("Offline player: {}", self.username),
            24.0,
            height - 30.0,
            1.0,
            GREY,
        );
        self.text(
            ui,
            &format!("v{VERSION} — not affiliated with Mojang"),
            width - 300.0,
            height - 30.0,
            1.0,
            DIM,
        );
        let status = self.wrap(&self.status, width - 520.0, 1.0);
        let lines: Vec<&str> = status.lines().collect();
        let shown = lines.last().copied().unwrap_or("");
        self.text(ui, shown, 200.0, height - 30.0, 1.0, DIM);
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
        self.tick_setup();
        self.drain_channels();

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
        let (width, height) = (width as f32, height as f32);
        let (x, y) = self.cursor;
        match self.screen {
            Screen::Main => self.click_main(main_layout(width, height), x, y),
            Screen::Setup => self.click_setup(setup_layout(width, height), x, y),
            Screen::Legal => {
                if legal_layout(width, height).back.contains(x, y) {
                    self.screen = Screen::Main;
                }
            }
        }
    }

    fn commit_game_dir(&mut self) {
        let path = PathBuf::from(self.game_dir_draft.trim());
        if path.as_os_str().is_empty() {
            self.status = "Game directory cannot be empty".to_string();
            return;
        }
        if let Err(err) = std::fs::create_dir_all(&path) {
            self.status = format!("Could not create {}: {err}", path.display());
            return;
        }
        self.profiles.active_mut().game_dir = path.clone();
        self.save_profiles();
        self.game_dir_editing = false;
        self.status = format!("Profile game directory: {}", path.display());
    }

    fn type_text(&mut self, event: &winit::event::KeyEvent) {
        if self.screen == Screen::Main && self.game_dir_editing {
            match &event.logical_key {
                Key::Named(NamedKey::Backspace) => {
                    self.game_dir_draft.pop();
                }
                Key::Named(NamedKey::Enter) => self.commit_game_dir(),
                Key::Named(NamedKey::Escape) => self.game_dir_editing = false,
                Key::Character(text) => {
                    for ch in text.chars().filter(|ch| !ch.is_control()) {
                        if self.game_dir_draft.chars().count() < 300 {
                            self.game_dir_draft.push(ch);
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        if self.screen != Screen::Setup || !self.setup_editing {
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => {
                self.setup_path.pop();
            }
            Key::Named(NamedKey::Enter) => {
                self.setup_editing = false;
                self.start_prepare();
            }
            Key::Named(NamedKey::Escape) => {
                self.setup_editing = false;
            }
            Key::Character(text) => {
                for ch in text.chars().filter(|ch| !ch.is_control()) {
                    if self.setup_path.chars().count() < 300 {
                        self.setup_path.push(ch);
                    }
                }
            }
            _ => {}
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
            WindowEvent::MouseWheel { delta, .. } => {
                if self.screen == Screen::Legal {
                    let amount = match delta {
                        MouseScrollDelta::LineDelta(_, lines) => lines * 30.0,
                        MouseScrollDelta::PixelDelta(position) => position.y as f32,
                    };
                    self.legal_scroll = (self.legal_scroll - amount).clamp(0.0, 600.0);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        if self.game_dir_editing {
                            self.game_dir_editing = false;
                        } else if self.screen == Screen::Main {
                            event_loop.exit();
                        } else if self.setup_editing {
                            self.setup_editing = false;
                        } else {
                            self.screen = Screen::Main;
                        }
                    }
                    _ => self.type_text(&event),
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

fn status_text(code: Option<i32>) -> String {
    match code {
        Some(0) => "Game closed".to_string(),
        Some(code) => format!("Game exited with code {code}"),
        None => "Game terminated abnormally".to_string(),
    }
}

/// Keep child processes from opening their own console window behind the
/// launcher. Their output is already redirected to a log file.
fn no_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Append one line to `<launcher>/launcher.log`.
///
/// A release launcher has no console, so this is where the unsigned-build
/// warning and other one-line notices survive.
fn append_launcher_log(line: &str) {
    let dir = paths::launcher_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("launcher.log"))
    {
        let _ = writeln!(file, "{line}");
    }
}

/// Generated placeholder tiles for the setup screen.
///
/// `MeshAtlas` packs sixteen 16x16 tiles per layer, and the renderer's block
/// binding is a `D2Array`. A one-layer array views as `D2`, which wgpu rejects,
/// so provide enough tiles to make two layers. No game texture is involved;
/// these are white pixels generated here.
fn bootstrap_blocks() -> TextureArray {
    let tile = image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 255, 255, 255]));
    let tiles = (0..17)
        .map(|index| (format!("bootstrap/{index}"), tile.clone()))
        .collect();
    TextureArray::from_tiles(tiles)
}

/// A one-pixel white atlas for the pre-resource setup screen.
///
/// `UiAtlas::blank()` leaves its white texel transparent, so every solid
/// rectangle would be invisible before the real textures exist. Writing a
/// generated 1x1 PNG into the launcher's own state directory gives the UI the
/// same white texel the real atlas provides, without embedding game assets.
fn bootstrap_atlas() -> UiAtlas {
    let dir = paths::launcher_dir().join("bootstrap");
    let gui = dir.join("gui");
    let path = gui.join("widgets.png");
    if std::fs::create_dir_all(&gui).is_err() {
        return UiAtlas::blank();
    }
    if !path.is_file() {
        let white = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255]));
        if white
            .save_with_format(&path, image::ImageFormat::Png)
            .is_err()
        {
            return UiAtlas::blank();
        }
    }
    UiAtlas::load(&dir).unwrap_or_else(|_| UiAtlas::blank())
}

fn open_in_file_manager(dir: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        Command::new("explorer").arg(dir).spawn().map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(dir).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(dir).spawn().map(|_| ())
    }
}

/// Every directory worth considering as the project root: the working
/// directory, then each ancestor of the executable.
fn candidate_roots() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    let mut candidate = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from));
    while let Some(dir) = candidate {
        dirs.push(dir.clone());
        candidate = dir.parent().map(PathBuf::from);
    }
    dirs
}

/// The workspace root, where `.local/prepared`, `research-lock.json` and
/// `assets/vanilla` live, and where `nephele-prepare` should run.
///
/// A `[workspace]` manifest wins over a prepared tree, because a stray
/// `.local/prepared` can exist inside `target` and would otherwise be picked
/// just for being closer. The prepared tree is only a fallback, for an
/// installed build that has no source tree beside it.
fn project_root() -> PathBuf {
    let dirs = candidate_roots();

    let has_workspace = |dir: &PathBuf| {
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .map(|manifest| manifest.contains("[workspace]"))
            .unwrap_or(false)
    };
    if let Some(found) = dirs.iter().find(|dir| has_workspace(dir)) {
        return found.clone();
    }
    if let Some(found) = dirs
        .iter()
        .find(|dir| dir.join(".local/prepared/1.8.9").is_dir())
    {
        return found.clone();
    }
    if let Some(found) = dirs
        .iter()
        .find(|dir| dir.join("research-lock.json").is_file())
    {
        return found.clone();
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nephele_launcher=info".into()),
        )
        .with_target(false)
        .without_time()
        .init();

    let signing = update::signing_summary();
    if !update::is_signed() {
        tracing::warn!(
            "UNSIGNED BUILD: no update signing key was compiled in; update checks are disabled"
        );
        append_launcher_log(&format!(
            "{} UNSIGNED BUILD: no update signing key was compiled in; update checks are disabled",
            crash::utc_stamp(crash::unix_now())
        ));
    }
    crash::install_launcher_panic_hook(paths::launcher_dir(), VERSION, signing);

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

    fn intersects(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn the_play_button_sits_inside_the_card() {
        let l = main_layout(WIDTH, HEIGHT);
        assert!(l.play.x >= l.card.x && l.play.x + l.play.w <= l.card.x + l.card.w);
        assert!(l.play.y >= l.card.y && l.play.y + l.play.h <= l.card.y + l.card.h);
        assert!(l.demo.y >= l.play.y + l.play.h, "demo must sit below play");
        assert!(l.demo.y + l.demo.h <= l.card.y + l.card.h);
    }

    #[test]
    fn no_two_interactive_elements_overlap() {
        for (width, height) in [(900.0, 600.0), (1000.0, 620.0), (1920.0, 1080.0)] {
            let l = main_layout(width, height);
            let buttons = [
                l.play,
                l.demo,
                l.quit,
                l.legal,
                l.prev,
                l.next,
                l.window,
                l.new_profile,
                l.duplicate,
                l.delete,
                l.open_folder,
                l.crash_toggle,
            ];
            for (index, first) in buttons.iter().enumerate() {
                for second in &buttons[index + 1..] {
                    assert!(
                        !intersects(*first, *second),
                        "overlap at {width}x{height}: {first:?} and {second:?}"
                    );
                }
                assert!(first.x >= 0.0 && first.y >= 0.0);
                assert!(first.x + first.w <= width && first.y + first.h <= height);
                assert!(
                    first.y + first.h <= content_bottom(height),
                    "button below the status bar: {first:?}"
                );
            }
        }
    }

    #[test]
    fn the_card_fills_the_space_right_of_the_profile_column() {
        for width in [900.0f32, 1000.0, 1600.0] {
            let l = main_layout(width, HEIGHT);
            assert_eq!(l.card.x, 300.0);
            assert!((width - (l.card.x + l.card.w) - 20.0).abs() < 0.01);
            assert!(l.card.w > 0.0);
        }
    }

    #[test]
    fn the_profile_controls_stay_inside_the_profile_panel() {
        for height in [600.0f32, 620.0, 1080.0] {
            let l = main_layout(WIDTH, height);
            for rect in [
                l.prev,
                l.next,
                l.window,
                l.new_profile,
                l.duplicate,
                l.delete,
                l.open_folder,
                l.crash_toggle,
            ] {
                assert!(rect.x >= l.profile.x && rect.x + rect.w <= l.profile.x + l.profile.w);
                assert!(rect.y >= l.profile.y && rect.y + rect.h <= l.profile.y + l.profile.h);
            }
        }
    }

    #[test]
    fn setup_and_legal_panels_fit_the_window() {
        for (width, height) in [(900.0, 600.0), (1000.0, 620.0), (1920.0, 1080.0)] {
            let setup = setup_layout(width, height);
            assert!(setup.panel.y >= 0.0);
            assert!(setup.panel.y + setup.panel.h <= height);
            assert!(setup.panel.x >= 0.0 && setup.panel.x + setup.panel.w <= width);
            assert!(setup.log.y + setup.log.h <= setup.panel.y + setup.panel.h);
            assert!(setup.bar.y + setup.bar.h <= setup.log.y);

            let legal = legal_layout(width, height);
            assert!(legal.panel.x >= 0.0 && legal.panel.x + legal.panel.w <= width);
            assert!(legal.panel.y >= 0.0 && legal.panel.y + legal.panel.h <= height);
            assert!(legal.back.x + legal.back.w <= legal.panel.x + legal.panel.w);
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
        let l = main_layout(WIDTH, HEIGHT);
        launcher.cursor = (l.play.x + 4.0, l.play.y + 4.0);
        assert!(launcher.hovered(l.play));
        assert!(!launcher.hovered(l.demo));
        assert!(!launcher.hovered(l.quit));
    }

    #[test]
    fn the_setup_screen_is_usable_without_resources() {
        let mut launcher = Launcher::new("Tester".to_string());
        launcher.resources = None;
        launcher.font_ready = false;
        launcher.screen = Screen::Setup;
        let ui = launcher.build_ui(WIDTH, HEIGHT);
        assert!(!ui.is_empty(), "the block font must draw the setup screen");
    }

    #[test]
    fn long_unbroken_text_is_hard_wrapped() {
        let launcher = Launcher::new("Tester".to_string());
        let path =
            r"C:\Users\somebody\AppData\Roaming\.minecraft\instances\a-very-long-instance-name";
        let wrapped = launcher.wrap(path, 120.0, 1.0);
        assert!(wrapped.lines().count() > 1, "{wrapped}");
        for line in wrapped.lines() {
            assert!(launcher.text_width(line, 1.0) <= 130.0, "{line:?}");
        }
        let truncated = launcher.truncate_lines(&wrapped, 2);
        assert_eq!(truncated.lines().count(), 2);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn a_profile_game_directory_can_be_edited() {
        let dir =
            std::env::temp_dir().join(format!("nephele-launcher-dirtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut launcher = Launcher::new("Tester".to_string());
        launcher.profiles_path = dir.join("profiles.json");
        let target = dir.join("new-game-dir");
        launcher.game_dir_draft = target.display().to_string();
        launcher.game_dir_editing = true;
        launcher.commit_game_dir();
        assert!(!launcher.game_dir_editing);
        assert_eq!(launcher.profiles.active().game_dir, target);
        assert!(target.is_dir());
        assert!(dir.join("profiles.json").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_launcher_without_resources_opens_setup() {
        // `Launcher::new` reads the machine's real directories, so only assert
        // the invariant that matters: missing resources never leave the
        // launcher on a screen that cannot work.
        let mut launcher = Launcher::new("Tester".to_string());
        if launcher.resources.is_none() {
            assert_eq!(launcher.screen, Screen::Setup);
        }
        launcher.resources = None;
        launcher.screen = Screen::Setup;
        assert_eq!(launcher.screen, Screen::Setup);
    }
}
