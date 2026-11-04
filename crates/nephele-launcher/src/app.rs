use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use nephele_assets::{TextureArray, UiAtlas};
use nephele_prepare::{client_jar_candidates, Lock, PrepareError, VERSION as GAME_VERSION};
use nephele_render::{Camera, MeshData, Rect, Renderer, UiBatch};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Icon, Window, WindowId};

use crate::crash::{CrashIncident, CrashReport, LogBuffer};
use crate::profiles::{Channel, LauncherConfig};
use crate::updates::{self, VerifiedRelease};

const WINDOW_TITLE: &str = "Nephele Launcher";
const LOGICAL_WIDTH: f64 = 1040.0;
const LOGICAL_HEIGHT: f64 = 660.0;

const BG_TOP: [f32; 4] = [0.035, 0.047, 0.071, 1.0];
const BG_BOTTOM: [f32; 4] = [0.071, 0.086, 0.122, 1.0];
const PANEL: [f32; 4] = [0.075, 0.094, 0.133, 1.0];
const PANEL_LIGHT: [f32; 4] = [0.11, 0.133, 0.18, 1.0];
const BORDER: [f32; 4] = [0.18, 0.22, 0.29, 1.0];
const ACCENT: [f32; 4] = [0.98, 0.67, 0.31, 1.0];
const ACCENT_HOVER: [f32; 4] = [1.0, 0.76, 0.4, 1.0];
const MINT: [f32; 4] = [0.36, 0.83, 0.76, 1.0];
const WHITE: [f32; 4] = [0.92, 0.95, 0.99, 1.0];
const GREY: [f32; 4] = [0.62, 0.68, 0.76, 1.0];
const DIM: [f32; 4] = [0.41, 0.47, 0.56, 1.0];
const RED: [f32; 4] = [0.86, 0.35, 0.34, 1.0];
const BANDS: usize = 36;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Play,
    Profiles,
    Versions,
    News,
    Settings,
    Legal,
    Crash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputField {
    ProfileName,
    PlayerName,
}

#[derive(Debug)]
enum UpdateState {
    NotChecked,
    Checking,
    Available(VerifiedRelease),
    Current(VerifiedRelease),
    Installing,
    Failed(String),
}

#[derive(Debug)]
enum AuthState {
    NotConfigured,
    Idle,
    Starting,
    Waiting {
        user_code: String,
        verification_uri: String,
    },
    SignedIn {
        name: String,
        expires_at: u64,
    },
    Failed(String),
}

#[derive(Debug)]
enum WorkerMessage {
    Prepared(Result<(), String>),
    Checked(Channel, Result<VerifiedRelease, String>),
    Installed(Result<(String, PathBuf), String>),
    Uploaded(Result<String, String>),
    AuthCode {
        user_code: String,
        verification_uri: String,
    },
    AuthFinished(Result<(String, u64), String>),
}

#[derive(Clone)]
struct Paths {
    data_dir: PathBuf,
    install_root: PathBuf,
    versions_root: PathBuf,
    executable_dir: PathBuf,
    lock_path: Option<PathBuf>,
}

impl Paths {
    fn discover(args: &[String]) -> Self {
        let data_dir = user_data_dir();
        let executable_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let explicit = argument_value(args, "--install-root")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("NEPHELE_INSTALL_DIR").map(PathBuf::from));
        let marked = ancestors(&executable_dir)
            .into_iter()
            .find(|dir| dir.join(".nephele-install").is_file());
        let workspace = ancestors(&current_dir)
            .into_iter()
            .chain(ancestors(&executable_dir))
            .find(|dir| {
                fs::read_to_string(dir.join("Cargo.toml"))
                    .map(|text| text.contains("[workspace]"))
                    .unwrap_or(false)
            });
        let managed_root = explicit.or(marked);
        let install_root = managed_root.clone().unwrap_or_else(|| data_dir.clone());
        let versions_root = if managed_root.is_some() {
            install_root.join("versions")
        } else {
            data_dir.join("versions")
        };
        let lock_path = [
            executable_dir.join("research-lock.json"),
            install_root.join("research-lock.json"),
            workspace
                .as_ref()
                .map(|root| root.join("research-lock.json"))
                .unwrap_or_default(),
        ]
        .into_iter()
        .find(|path| path.is_file());

        Self {
            data_dir,
            install_root,
            versions_root,
            executable_dir,
            lock_path,
        }
    }

    fn config_path(&self) -> PathBuf {
        self.data_dir.join("launcher.json")
    }

    fn prepared_dir(&self) -> PathBuf {
        self.data_dir.join("prepared").join(GAME_VERSION)
    }
}

#[derive(Clone, Copy)]
struct Layout {
    content: Rect,
    nav: [Rect; 6],
    launch: Rect,
    demo: Rect,
    locate_folder: Rect,
    locate_jar: Rect,
    profile_rows: [Rect; 4],
    profile_add: Rect,
    profile_remove: Rect,
    profile_name: Rect,
    player_name: Rect,
    profile_stable: Rect,
    profile_preview: Rect,
    account_offline: Rect,
    account_online: Rect,
    auth_action: Rect,
    stable: Rect,
    preview: Rect,
    version_rows: [Rect; 3],
    update_info: Rect,
    check_updates: Rect,
    install_update: Rect,
    consent: Rect,
    auto_update: Rect,
    review_crash: Rect,
    legal_tabs: [Rect; 3],
    crash_upload: Rect,
    crash_open: Rect,
    crash_back: Rect,
}

fn layout(width: f32, height: f32) -> Layout {
    let nav_x = 16.0;
    let nav_w = 178.0;
    let nav =
        std::array::from_fn(|index| Rect::new(nav_x, 102.0 + index as f32 * 50.0, nav_w, 40.0));
    let content = Rect::new(
        220.0,
        92.0,
        (width - 252.0).max(430.0),
        (height - 126.0).max(300.0),
    );
    let x = content.x + 20.0;
    let inner_w = content.w - 40.0;
    let y = content.y + 94.0;
    let button_h = 46.0;
    let half = (inner_w - 14.0) / 2.0;
    Layout {
        content,
        nav,
        launch: Rect::new(x, y + 40.0, inner_w, 58.0),
        demo: Rect::new(x, y + 112.0, inner_w, 42.0),
        locate_folder: Rect::new(x, y + 184.0, half, button_h),
        locate_jar: Rect::new(x + half + 14.0, y + 184.0, half, button_h),
        profile_rows: std::array::from_fn(|index| {
            Rect::new(x, y + index as f32 * 50.0, 174.0, 40.0)
        }),
        profile_add: Rect::new(x, content.y + content.h - 54.0, 80.0, 36.0),
        profile_remove: Rect::new(x + 92.0, content.y + content.h - 54.0, 82.0, 36.0),
        profile_name: Rect::new(x + 204.0, y + 34.0, inner_w - 204.0, 40.0),
        player_name: Rect::new(x + 204.0, y + 108.0, inner_w - 204.0, 40.0),
        profile_stable: Rect::new(x + 204.0, y + 168.0, 126.0, 34.0),
        profile_preview: Rect::new(x + 344.0, y + 168.0, 126.0, 34.0),
        account_offline: Rect::new(x + 204.0, y + 224.0, 126.0, 34.0),
        account_online: Rect::new(x + 344.0, y + 224.0, 126.0, 34.0),
        auth_action: Rect::new(x + 204.0, y + 264.0, inner_w - 204.0, 34.0),
        stable: Rect::new(x, content.y + 94.0, 122.0, 38.0),
        preview: Rect::new(x + 134.0, content.y + 94.0, 122.0, 38.0),
        version_rows: std::array::from_fn(|index| {
            Rect::new(x, content.y + 166.0 + index as f32 * 38.0, 196.0, 32.0)
        }),
        update_info: Rect::new(x + 214.0, content.y + 142.0, inner_w - 214.0, 212.0),
        check_updates: Rect::new(x, content.y + content.h - 54.0, half, 38.0),
        install_update: Rect::new(x + half + 14.0, content.y + content.h - 54.0, half, 38.0),
        consent: Rect::new(x, y + 54.0, 28.0, 28.0),
        auto_update: Rect::new(x, content.y + content.h - 102.0, 28.0, 28.0),
        review_crash: Rect::new(x, content.y + content.h - 54.0, 248.0, 38.0),
        legal_tabs: std::array::from_fn(|index| {
            Rect::new(x + index as f32 * 142.0, y + 28.0, 130.0, 34.0)
        }),
        crash_upload: Rect::new(x, content.y + content.h - 54.0, 154.0, 38.0),
        crash_open: Rect::new(x + 162.0, content.y + content.h - 54.0, 128.0, 38.0),
        crash_back: Rect::new(
            x + 298.0,
            content.y + content.h - 54.0,
            inner_w - 298.0,
            38.0,
        ),
    }
}

struct Launcher {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    camera: Camera,
    cursor: (f32, f32),
    paths: Paths,
    config: LauncherConfig,
    config_warning: Option<String>,
    view: View,
    version_scroll: usize,
    legal_page: usize,
    focused_input: Option<InputField>,
    input_buffer: String,
    child: Option<Child>,
    child_version: String,
    activity_label: &'static str,
    log_buffer: LogBuffer,
    log_sender: Sender<String>,
    log_receiver: Receiver<String>,
    worker_sender: Sender<WorkerMessage>,
    worker_receiver: Receiver<WorkerMessage>,
    update_state: UpdateState,
    checking_channel: Option<Channel>,
    auth_state: AuthState,
    news: Vec<updates::NewsItem>,
    status: String,
    preparing: bool,
    crash: Option<CrashIncident>,
    fatal_error: Option<Box<dyn Error>>,
}

impl Launcher {
    fn new(paths: Paths, config: LauncherConfig, config_warning: Option<String>) -> Self {
        let (log_sender, log_receiver) = mpsc::channel();
        let (worker_sender, worker_receiver) = mpsc::channel();
        let ready = minecraft_root(&paths).is_some();
        let news = updates::load_cached_release(&paths.data_dir, config.selected_profile().channel)
            .map(|release| release.manifest.news)
            .unwrap_or_default();
        let auth_state = load_auth_state(&paths.data_dir);
        Self {
            window: None,
            renderer: None,
            camera: Camera::default(),
            cursor: (-1.0, -1.0),
            paths,
            config,
            config_warning,
            view: View::Play,
            version_scroll: 0,
            legal_page: 0,
            focused_input: None,
            input_buffer: String::new(),
            child: None,
            child_version: env!("CARGO_PKG_VERSION").to_string(),
            activity_label: "GAME",
            log_buffer: LogBuffer::default(),
            log_sender,
            log_receiver,
            worker_sender,
            worker_receiver,
            update_state: UpdateState::NotChecked,
            checking_channel: None,
            auth_state,
            news,
            status: if ready {
                "READY TO PLAY".into()
            } else {
                "SETUP REQUIRED - PREPARE YOUR 1.8.9 RESOURCES".into()
            },
            preparing: false,
            crash: None,
            fatal_error: None,
        }
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_inner_size(LogicalSize::new(LOGICAL_WIDTH, LOGICAL_HEIGHT))
            .with_min_inner_size(LogicalSize::new(760.0, 560.0))
            .with_window_icon(launcher_icon());
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                self.fatal_error = Some(Box::new(err));
                event_loop.exit();
                return;
            }
        };
        let ui_atlas = minecraft_root(&self.paths)
            .filter(|root| root.join("textures/font/ascii.png").is_file())
            .and_then(|root| UiAtlas::load(&root.join("textures")).ok())
            .unwrap_or_else(UiAtlas::launcher_fallback);
        let blocks = minecraft_root(&self.paths)
            .and_then(|root| TextureArray::load_from_dir(&root.join("textures/blocks")).ok())
            .unwrap_or_else(|| TextureArray::from_tiles(Vec::new()));
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
            Err(err) => {
                self.fatal_error = Some(Box::new(err));
                event_loop.exit();
                return;
            }
        };
        renderer.set_mesh(&MeshData {
            vertices: Vec::new(),
            indices: Vec::new(),
        });
        self.camera.set_aspect(size.width, size.height);
        self.window = Some(window);
        self.renderer = Some(renderer);
        self.begin_startup_setup();
        self.begin_update_check();
    }

    fn begin_startup_setup(&mut self) {
        if minecraft_root(&self.paths).is_some() {
            return;
        }
        if let Some(jar) = self.find_installed_jar() {
            self.start_preparation(jar);
        }
    }

    fn find_installed_jar(&self) -> Option<PathBuf> {
        if let Some(jar) = self
            .config
            .minecraft_jar
            .as_ref()
            .filter(|path| path.is_file())
        {
            return Some(jar.clone());
        }
        let mut roots = Vec::new();
        if let Some(dir) = &self.config.minecraft_dir {
            roots.push(dir.clone());
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let appdata = PathBuf::from(appdata);
            roots.push(appdata.join(".minecraft"));
            roots.push(appdata.join("PrismLauncher"));
            roots.push(appdata.join("MultiMC"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            roots.push(home.join(".minecraft"));
            roots.push(home.join(".local/share/PrismLauncher"));
        }
        roots
            .into_iter()
            .flat_map(|root| client_jar_candidates(&root))
            .find(|path| path.is_file())
    }

    fn start_preparation(&mut self, jar: PathBuf) {
        if self.preparing {
            return;
        }
        let Some(lock_path) = self.paths.lock_path.clone() else {
            self.status = "INSTALL FILES ARE INCOMPLETE: RESEARCH-LOCK.JSON IS MISSING".into();
            return;
        };
        self.preparing = true;
        self.status = "VERIFYING YOUR MINECRAFT 1.8.9 COPY AND PREPARING RESOURCES...".into();
        let out_dir = self.paths.prepared_dir();
        let sender = self.worker_sender.clone();
        thread::spawn(move || {
            let result =
                prepare_resources(&jar, &lock_path, &out_dir).map_err(|err| err.to_string());
            let _ = sender.send(WorkerMessage::Prepared(result));
        });
    }

    fn begin_update_check(&mut self) {
        if matches!(self.update_state, UpdateState::Installing) {
            return;
        }
        let channel = self.config.selected_profile().channel;
        if self.checking_channel == Some(channel) {
            return;
        }
        self.checking_channel = Some(channel);
        self.update_state = UpdateState::Checking;
        self.status = "CHECKING SIGNED RELEASES...".into();
        let sender = self.worker_sender.clone();
        thread::spawn(move || {
            let result = updates::check_latest(channel).map_err(|err| err.to_string());
            let _ = sender.send(WorkerMessage::Checked(channel, result));
        });
    }

    fn install_update(&mut self) {
        let release = match &self.update_state {
            UpdateState::Available(release) => release.clone(),
            _ => return,
        };
        self.update_state = UpdateState::Installing;
        self.status = format!(
            "DOWNLOADING AND VERIFYING NEPHELE {}...",
            release.manifest.version
        );
        let versions_root = self.paths.versions_root.clone();
        let install_root = self.paths.install_root.clone();
        let sender = self.worker_sender.clone();
        thread::spawn(move || {
            let result = updates::install_release(&release, &versions_root)
                .and_then(|directory| {
                    updates::set_current_version(&install_root, &release.manifest.version)?;
                    Ok((release.manifest.version.clone(), directory))
                })
                .map_err(|err| err.to_string());
            let _ = sender.send(WorkerMessage::Installed(result));
        });
    }

    fn begin_sign_in(&mut self) {
        if matches!(self.auth_state, AuthState::Starting) {
            return;
        }
        if let AuthState::Waiting {
            verification_uri, ..
        } = &self.auth_state
        {
            match open_url(verification_uri) {
                Ok(()) => self.status = "MICROSOFT SIGN-IN PAGE OPENED".into(),
                Err(error) => self.status = format!("OPEN THE SIGN-IN URL MANUALLY: {error}"),
            }
            return;
        }
        let Some(client_id) = option_env!("NEPHELE_CLIENT_ID")
            .filter(|client_id| !client_id.trim().is_empty())
            .map(str::to_owned)
        else {
            self.auth_state = AuthState::NotConfigured;
            self.status = "MICROSOFT SIGN-IN IS NOT CONFIGURED FOR THIS BUILD".into();
            return;
        };
        self.auth_state = AuthState::Starting;
        self.status = "REQUESTING A MICROSOFT DEVICE CODE...".into();
        let session_path = self.paths.data_dir.join("auth.json");
        let sender = self.worker_sender.clone();
        thread::spawn(move || {
            let result = (|| {
                let device_code = nephele_auth::begin_with(&client_id, false)
                    .map_err(|error| error.to_string())?;
                let _ = sender.send(WorkerMessage::AuthCode {
                    user_code: device_code.user_code.clone(),
                    verification_uri: device_code.verification_uri.clone(),
                });
                let microsoft_token =
                    nephele_auth::wait_for_token_with(&client_id, &device_code, false, |_| {})
                        .map_err(|error| error.to_string())?;
                let session = nephele_auth::to_minecraft(&client_id, &microsoft_token)
                    .map_err(|error| error.to_string())?;
                nephele_auth::StoredSession::from_session(&client_id, &session)
                    .save(&session_path)
                    .map_err(|error| error.to_string())?;
                Ok((session.profile.name, session.expires_at))
            })();
            let _ = sender.send(WorkerMessage::AuthFinished(result));
        });
    }

    fn valid_signed_in_account(&self) -> Option<(&str, u64)> {
        let AuthState::SignedIn { name, expires_at } = &self.auth_state else {
            return None;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (*expires_at > now).then_some((name.as_str(), *expires_at))
    }

    fn upload_crash_report(&mut self) {
        if !self.config.crash_upload_opt_in {
            self.status = "OPT IN TO CRASH UPLOADS IN SETTINGS FIRST".into();
            return;
        }
        let Some(report) = self.crash.as_ref().map(|crash| crash.report.clone()) else {
            return;
        };
        self.status = "SENDING THE CRASH REPORT...".into();
        if let Some(crash) = self.crash.as_mut() {
            crash.upload_status = Some("SENDING...".into());
        }
        let sender = self.worker_sender.clone();
        thread::spawn(move || {
            let _ = sender.send(WorkerMessage::Uploaded(crate::crash::upload(&report)));
        });
    }

    fn handle_worker_messages(&mut self) {
        while let Ok(message) = self.worker_receiver.try_recv() {
            match message {
                WorkerMessage::Prepared(result) => {
                    self.preparing = false;
                    match result {
                        Ok(()) => {
                            self.status = "RESOURCES READY - YOU CAN PLAY".into();
                        }
                        Err(error) => {
                            self.status = format!("SETUP FAILED: {error}");
                        }
                    }
                }
                WorkerMessage::Checked(channel, result) => {
                    if self.checking_channel == Some(channel) {
                        self.checking_channel = None;
                    }
                    if self.config.selected_profile().channel != channel {
                        continue;
                    }
                    match result {
                        Ok(release) => {
                            let _ = updates::save_cached_release(&self.paths.data_dir, &release);
                            self.news = release.manifest.news.clone();
                            if updates::compare_versions(
                                &release.manifest.version,
                                &self.active_version(),
                            )
                            .is_gt()
                            {
                                self.status = format!(
                                    "NEPHELE {} IS READY TO INSTALL",
                                    release.manifest.version
                                );
                                self.update_state = UpdateState::Available(release);
                                if self.config.auto_update && !self.selected_version_is_pinned() {
                                    self.install_update();
                                }
                            } else {
                                self.status = "YOU ARE RUNNING THE LATEST SIGNED RELEASE".into();
                                self.update_state = UpdateState::Current(release);
                            }
                        }
                        Err(error) => {
                            self.status = format!("UPDATE CHECK: {error}");
                            self.update_state = UpdateState::Failed(error);
                        }
                    }
                }
                WorkerMessage::Installed(result) => match result {
                    Ok((version, _directory)) => {
                        self.save_config();
                        self.status = format!(
                            "VERSION {version} VERIFIED AND INSTALLED. CLOSE AND REOPEN TO FINISH."
                        );
                        if let UpdateState::Installing = self.update_state {
                            self.update_state = UpdateState::NotChecked;
                        }
                    }
                    Err(error) => {
                        self.status = format!("UPDATE FAILED: {error}");
                        self.update_state = UpdateState::Failed(error);
                    }
                },
                WorkerMessage::Uploaded(result) => {
                    if let Some(crash) = self.crash.as_mut() {
                        match result {
                            Ok(message) => {
                                crash.upload_status = Some(message.clone());
                                self.status = message;
                            }
                            Err(error) => {
                                crash.upload_status = Some(format!("UPLOAD FAILED: {error}"));
                                self.status = format!("UPLOAD FAILED: {error}");
                            }
                        }
                    }
                }
                WorkerMessage::AuthCode {
                    user_code,
                    verification_uri,
                } => {
                    self.auth_state = AuthState::Waiting {
                        user_code,
                        verification_uri: verification_uri.clone(),
                    };
                    match open_url(&verification_uri) {
                        Ok(()) => {
                            self.status = "BROWSER OPENED - ENTER THE CODE SHOWN IN PROFILES".into()
                        }
                        Err(error) => {
                            self.status = format!("OPEN {verification_uri} MANUALLY: {error}")
                        }
                    }
                }
                WorkerMessage::AuthFinished(result) => match result {
                    Ok((name, expires_at)) => {
                        self.auth_state = AuthState::SignedIn {
                            name: name.clone(),
                            expires_at,
                        };
                        let profile = self.config.selected_profile_mut();
                        profile.username = name.clone();
                        profile.online = true;
                        self.save_config();
                        self.status = format!("SIGNED IN AS {name}");
                    }
                    Err(error) => {
                        self.auth_state = AuthState::Failed(error.clone());
                        self.status = format!("MICROSOFT SIGN-IN FAILED: {error}");
                    }
                },
            }
        }
    }

    fn active_version(&self) -> String {
        if let Some(version) = self.config.selected_profile().version.as_ref() {
            if self
                .paths
                .versions_root
                .join(version)
                .join("nephele.exe")
                .is_file()
            {
                return version.clone();
            }
        }
        if let Ok(current) = fs::read_to_string(self.paths.install_root.join("current-version.txt"))
        {
            let current = current.trim();
            if crate::profiles::valid_version_id(current)
                && self
                    .paths
                    .versions_root
                    .join(current)
                    .join("nephele.exe")
                    .is_file()
            {
                return current.to_string();
            }
        }
        executable_version(&self.paths.executable_dir)
    }

    fn selected_version_is_pinned(&self) -> bool {
        self.config
            .selected_profile()
            .version
            .as_ref()
            .is_some_and(|version| {
                self.paths
                    .versions_root
                    .join(version)
                    .join(exe_name("nephele"))
                    .is_file()
            })
    }

    fn launch_game(&mut self, demo: bool) {
        if self.child.is_some() || self.preparing {
            return;
        }
        let Some(assets_root) = minecraft_root(&self.paths) else {
            self.status = "PREPARE MINECRAFT 1.8.9 RESOURCES BEFORE PLAYING".into();
            return;
        };
        let profile = self.config.selected_profile().clone();
        let online_name = if profile.online {
            match self.valid_signed_in_account() {
                Some((name, _)) => Some(name.to_string()),
                None => {
                    self.status =
                        "MICROSOFT SIGN-IN IS REQUIRED OR HAS EXPIRED. OPEN PROFILES.".into();
                    self.view = View::Profiles;
                    return;
                }
            }
        } else {
            None
        };
        let version = self.active_version();
        let binary = self
            .paths
            .versions_root
            .join(&version)
            .join(exe_name("nephele"));
        let binary = if binary.is_file() {
            binary
        } else {
            self.paths.executable_dir.join(exe_name("nephele"))
        };
        if !binary.is_file() {
            self.status = "THE NEPHELE GAME FILE IS MISSING. REINSTALL THIS VERSION.".into();
            return;
        }
        let game_dir = binary
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.paths.executable_dir.clone());
        let mut command = Command::new(binary);
        if demo {
            command.arg("view");
            self.activity_label = "DEMO";
        } else {
            command
                .arg("play")
                .arg("--username")
                .arg(online_name.as_deref().unwrap_or(profile.username.trim()));
            if profile.online {
                command.arg("--online");
            }
            self.activity_label = "GAME";
        }
        command
            .current_dir(game_dir)
            .env("NEPHELE_DATA_DIR", &self.paths.data_dir)
            .env("NEPHELE_MINECRAFT_ASSETS", &assets_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW; the launcher owns the window.
        }
        match command.spawn() {
            Ok(mut child) => {
                self.log_buffer = LogBuffer::default();
                self.child_version = version;
                if let Some(stdout) = child.stdout.take() {
                    spawn_log_reader(stdout, self.log_sender.clone());
                }
                if let Some(stderr) = child.stderr.take() {
                    spawn_log_reader(stderr, self.log_sender.clone());
                }
                self.child = Some(child);
                self.status = format!("{} RUNNING", self.activity_label);
            }
            Err(error) => self.status = format!("COULD NOT START THE GAME: {error}"),
        }
    }

    fn tick_child(&mut self) {
        while let Ok(line) = self.log_receiver.try_recv() {
            self.log_buffer.push(&line);
            self.append_log_line(&line);
        }
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) => {
                self.child = None;
                if status.success() {
                    self.status = format!("{} CLOSED", self.activity_label);
                } else {
                    let report = CrashReport::new(
                        &self.child_version,
                        status.code(),
                        self.log_buffer.snapshot(),
                    );
                    match report.save(&self.paths.data_dir) {
                        Ok(local_path) => {
                            self.crash = Some(CrashIncident {
                                report,
                                local_path,
                                upload_status: None,
                            });
                            self.view = View::Crash;
                            self.status =
                                "THE GAME STOPPED UNEXPECTEDLY. REPORT SAVED LOCALLY.".into();
                        }
                        Err(error) => {
                            self.status = format!("GAME CRASHED. {error}");
                        }
                    }
                }
            }
            Err(error) => {
                self.child = None;
                self.status = format!("COULD NOT CHECK THE GAME PROCESS: {error}");
            }
        }
    }

    fn append_log_line(&self, line: &str) {
        let directory = self.paths.data_dir.join("logs");
        if fs::create_dir_all(&directory).is_err() {
            return;
        }
        let log_path = directory.join("client.log");
        if fs::metadata(&log_path)
            .map(|metadata| metadata.len() > 4 * 1024 * 1024)
            .unwrap_or(false)
        {
            let previous = directory.join("client.log.1");
            let _ = fs::remove_file(&previous);
            let _ = fs::rename(&log_path, previous);
        }
        let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) else {
            return;
        };
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }

    fn save_config(&mut self) {
        match self.config.save(&self.paths.config_path()) {
            Ok(()) => self.config_warning = None,
            Err(error) => self.config_warning = Some(error),
        }
    }

    fn finish_input(&mut self) {
        if let Some(field) = self.focused_input.take() {
            let value = self.input_buffer.trim().to_string();
            let profile = self.config.selected_profile_mut();
            match field {
                InputField::ProfileName if !value.is_empty() && value.chars().count() <= 32 => {
                    profile.name = value;
                }
                InputField::PlayerName
                    if !value.is_empty()
                        && value.len() <= 16
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') =>
                {
                    profile.username = value;
                }
                _ => {}
            }
            if let Err(error) = profile.validate() {
                self.status = error.to_ascii_uppercase();
            } else {
                self.save_config();
            }
        }
    }

    fn begin_input(&mut self, field: InputField) {
        self.finish_input();
        self.input_buffer = match field {
            InputField::ProfileName => self.config.selected_profile().name.clone(),
            InputField::PlayerName => self.config.selected_profile().username.clone(),
        };
        self.focused_input = Some(field);
    }

    fn choose_minecraft_folder(&mut self) {
        self.finish_input();
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("Choose your Minecraft or Prism instance folder")
            .pick_folder()
        {
            self.config.minecraft_dir = Some(folder.clone());
            self.config.minecraft_jar = None;
            self.save_config();
            let jar = client_jar_candidates(&folder)
                .into_iter()
                .find(|path| path.is_file());
            match jar {
                Some(jar) => self.start_preparation(jar),
                None => {
                    self.status = format!(
                        "NO 1.8.9 CLIENT JAR FOUND IN {}. START 1.8.9 ONCE, THEN TRY AGAIN.",
                        folder.display()
                    )
                }
            }
        }
    }

    fn choose_minecraft_jar(&mut self) {
        self.finish_input();
        if let Some(jar) = rfd::FileDialog::new()
            .set_title("Choose your own Minecraft 1.8.9 client.jar")
            .add_filter("Minecraft client jar", &["jar"])
            .pick_file()
        {
            self.config.minecraft_jar = Some(jar.clone());
            self.config.minecraft_dir = None;
            self.save_config();
            self.start_preparation(jar);
        }
    }

    fn click(&mut self) {
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let (width, height) = renderer.size();
        let l = layout(width as f32, height as f32);
        let (x, y) = self.cursor;
        if self.view != View::Crash {
            for (index, rect) in l.nav.iter().enumerate() {
                if rect.contains(x, y) {
                    self.finish_input();
                    self.view = match index {
                        0 => View::Play,
                        1 => View::Profiles,
                        2 => View::Versions,
                        3 => View::News,
                        4 => View::Settings,
                        _ => View::Legal,
                    };
                    return;
                }
            }
        }
        match self.view {
            View::Play => {
                if l.launch.contains(x, y) {
                    if minecraft_root(&self.paths).is_some() {
                        self.launch_game(false);
                    } else if !self.preparing {
                        if let Some(jar) = self.find_installed_jar() {
                            self.start_preparation(jar);
                        } else {
                            self.choose_minecraft_folder();
                        }
                    }
                } else if l.demo.contains(x, y) {
                    self.launch_game(true);
                } else if l.locate_folder.contains(x, y) {
                    self.choose_minecraft_folder();
                } else if l.locate_jar.contains(x, y) {
                    self.choose_minecraft_jar();
                }
            }
            View::Profiles => {
                for (index, rect) in l.profile_rows.iter().enumerate() {
                    if rect.contains(x, y) && index < self.config.profiles.len() {
                        self.finish_input();
                        self.config.selected_profile = self.config.profiles[index].id.clone();
                        self.save_config();
                        return;
                    }
                }
                if l.profile_add.contains(x, y) && self.config.profiles.len() < 4 {
                    self.finish_input();
                    self.config.add_profile();
                    self.save_config();
                } else if l.profile_remove.contains(x, y) {
                    self.finish_input();
                    self.config.remove_selected_profile();
                    self.save_config();
                } else if l.profile_name.contains(x, y) {
                    self.begin_input(InputField::ProfileName);
                } else if l.player_name.contains(x, y) {
                    self.begin_input(InputField::PlayerName);
                } else if l.profile_stable.contains(x, y) {
                    self.config.selected_profile_mut().channel = Channel::Stable;
                    self.save_config();
                    self.begin_update_check();
                } else if l.profile_preview.contains(x, y) {
                    self.config.selected_profile_mut().channel = Channel::Preview;
                    self.save_config();
                    self.begin_update_check();
                } else if l.account_offline.contains(x, y) {
                    self.config.selected_profile_mut().online = false;
                    self.save_config();
                } else if l.account_online.contains(x, y) {
                    if let Some((name, _)) = self.valid_signed_in_account() {
                        let name = name.to_string();
                        let profile = self.config.selected_profile_mut();
                        profile.username = name;
                        profile.online = true;
                        self.save_config();
                    } else {
                        self.begin_sign_in();
                    }
                } else if l.auth_action.contains(x, y) {
                    self.begin_sign_in();
                }
            }
            View::Versions => {
                if l.stable.contains(x, y) {
                    self.finish_input();
                    self.config.selected_profile_mut().channel = Channel::Stable;
                    self.save_config();
                    self.begin_update_check();
                } else if l.preview.contains(x, y) {
                    self.finish_input();
                    self.config.selected_profile_mut().channel = Channel::Preview;
                    self.save_config();
                    self.begin_update_check();
                } else if l.check_updates.contains(x, y) {
                    self.begin_update_check();
                } else if l.install_update.contains(x, y) {
                    self.install_update();
                } else {
                    let versions = self.visible_versions();
                    for (index, rect) in l.version_rows.iter().enumerate() {
                        let version_index = self.version_scroll + index;
                        if rect.contains(x, y) && version_index < versions.len() {
                            let version = versions[version_index].clone();
                            self.config.selected_profile_mut().version = if version == "LATEST" {
                                None
                            } else {
                                Some(version)
                            };
                            self.save_config();
                            return;
                        }
                    }
                }
            }
            View::News => {}
            View::Settings => {
                if l.consent.contains(x, y) {
                    self.config.crash_upload_opt_in = !self.config.crash_upload_opt_in;
                    self.save_config();
                } else if l.auto_update.contains(x, y) {
                    self.config.auto_update = !self.config.auto_update;
                    self.save_config();
                } else if l.review_crash.contains(x, y) && self.crash.is_some() {
                    self.view = View::Crash;
                }
            }
            View::Legal => {
                for (index, rect) in l.legal_tabs.iter().enumerate() {
                    if rect.contains(x, y) {
                        self.legal_page = index;
                    }
                }
            }
            View::Crash => {
                if l.crash_upload.contains(x, y) {
                    self.upload_crash_report();
                } else if l.crash_open.contains(x, y) {
                    if let Some(path) = self.crash.as_ref().map(|crash| crash.local_path.clone()) {
                        match open_local_file(&path) {
                            Ok(()) => self.status = "OPENED LOCAL CRASH REPORT".into(),
                            Err(error) => self.status = format!("COULD NOT OPEN REPORT: {error}"),
                        }
                    }
                } else if l.crash_back.contains(x, y) {
                    self.view = View::Play;
                }
            }
        }
    }

    fn visible_versions(&self) -> Vec<String> {
        let mut versions = updates::installed_versions(&self.paths.versions_root);
        let current = executable_version(&self.paths.executable_dir);
        if !versions.contains(&current) {
            versions.insert(0, current);
        }
        versions.insert(0, "LATEST".into());
        versions
    }

    fn scroll_versions(&mut self, direction: isize) {
        let count = self.visible_versions().len();
        let max_scroll = count.saturating_sub(3);
        self.version_scroll = if direction < 0 {
            self.version_scroll.saturating_sub(direction.unsigned_abs())
        } else {
            (self.version_scroll + direction as usize).min(max_scroll)
        };
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
        let l = layout(width, height);
        self.draw_brand(&mut ui, width);
        ui.rect(Rect::new(0.0, 70.0, width, 1.0), BORDER);
        ui.rect(
            Rect::new(0.0, 70.0, 205.0, height - 70.0),
            [0.035, 0.047, 0.071, 0.55],
        );
        for (index, (label, view)) in [
            ("PLAY", View::Play),
            ("PROFILES", View::Profiles),
            ("VERSIONS", View::Versions),
            ("NEWS", View::News),
            ("SETTINGS", View::Settings),
            ("LEGAL", View::Legal),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = self.view == view;
            let hovered = l.nav[index].contains(self.cursor.0, self.cursor.1);
            ui.rect(
                l.nav[index],
                if selected {
                    PANEL_LIGHT
                } else if hovered {
                    PANEL
                } else {
                    [0.0; 4]
                },
            );
            if selected {
                ui.rect(
                    Rect::new(l.nav[index].x, l.nav[index].y, 3.0, l.nav[index].h),
                    ACCENT,
                );
            }
            ui.text(
                label,
                l.nav[index].x + 15.0,
                l.nav[index].y + 14.0,
                1.0,
                if selected { WHITE } else { GREY },
            );
        }
        ui.text("EARLY ACCESS BUILD", 24.0, height - 64.0, 1.0, DIM);
        ui.text("NO GAME FILES INCLUDED", 24.0, height - 44.0, 1.0, DIM);
        ui.rect(l.content, [0.047, 0.063, 0.09, 0.92]);
        ui.rect_outline(l.content, 1.0, BORDER);
        self.draw_page(&mut ui, l, height);
        let legal_x = width - 360.0;
        let status_chars = ((legal_x - 230.0).max(60.0)) / 6.0;
        if let Some(warning) = &self.config_warning {
            ui.text(
                &clip_text(warning, status_chars),
                220.0,
                height - 25.0,
                1.0,
                RED,
            );
        } else {
            ui.text(
                &clip_text(&self.status, status_chars),
                220.0,
                height - 25.0,
                1.0,
                DIM,
            );
        }
        ui.text(
            &format!(
                "V{}  |  NOT AN OFFICIAL MINECRAFT PRODUCT",
                env!("CARGO_PKG_VERSION")
            ),
            legal_x,
            height - 25.0,
            1.0,
            DIM,
        );
        ui
    }

    fn draw_brand(&self, ui: &mut UiBatch, width: f32) {
        // Original geometric cloud/star mark, drawn with the launcher's own UI primitives.
        ui.rect(Rect::new(22.0, 20.0, 12.0, 12.0), ACCENT);
        ui.rect(Rect::new(34.0, 12.0, 12.0, 20.0), MINT);
        ui.rect(Rect::new(46.0, 20.0, 12.0, 12.0), ACCENT);
        ui.rect(Rect::new(28.0, 32.0, 24.0, 4.0), MINT);
        ui.text("NEPHELE", 72.0, 14.0, 2.0, WHITE);
        ui.text("MINECRAFT 1.8.9 CLIENT", 74.0, 42.0, 1.0, GREY);
        ui.text("LAUNCHER", width - 105.0, 29.0, 1.0, DIM);
    }

    fn draw_page(&self, ui: &mut UiBatch, l: Layout, height: f32) {
        let x = l.content.x + 20.0;
        let y = l.content.y + 24.0;
        let inner_w = l.content.w - 40.0;
        match self.view {
            View::Play => {
                ui.text("WELCOME BACK", x, y, 2.0, WHITE);
                ui.text(
                    &format!(
                        "PROFILE: {}  |  PLAYER: {}",
                        self.config.selected_profile().name,
                        self.config.selected_profile().username
                    ),
                    x,
                    y + 30.0,
                    1.0,
                    GREY,
                );
                let ready = minecraft_root(&self.paths).is_some();
                draw_button(
                    ui,
                    l.launch,
                    if self.child.is_some() {
                        "GAME RUNNING"
                    } else if ready {
                        "PLAY NEPHELE"
                    } else if self.preparing {
                        "PREPARING RESOURCES..."
                    } else {
                        "FIND MINECRAFT 1.8.9 TO PLAY"
                    },
                    !self.preparing && self.child.is_none(),
                    self.cursor,
                );
                if !ready {
                    ui.text(
                        "NEPHELE USES THE 1.8.9 GAME COPY YOU ALREADY OWN.",
                        x,
                        l.launch.y + 74.0,
                        1.0,
                        GREY,
                    );
                    ui.text(
                        "NOTHING IS DOWNLOADED OR SENT FROM THAT COPY.",
                        x,
                        l.launch.y + 94.0,
                        1.0,
                        DIM,
                    );
                    draw_button(
                        ui,
                        l.locate_folder,
                        "FIND GAME FOLDER",
                        !self.preparing,
                        self.cursor,
                    );
                    draw_button(
                        ui,
                        l.locate_jar,
                        "CHOOSE CLIENT.JAR",
                        !self.preparing,
                        self.cursor,
                    );
                    draw_wrapped(
                        ui,
                        "Install and open Minecraft Java Edition 1.8.9 once in the official launcher, or choose the 1.8.9 client.jar you already own. Nephele verifies the jar before extracting only the resources its client needs.",
                        Rect::new(x, l.locate_folder.y + 64.0, inner_w, 0.0),
                        1.0,
                        GREY,
                    );
                } else {
                    draw_button(
                        ui,
                        l.demo,
                        "OPEN OFFLINE DEMO WORLD",
                        self.child.is_none(),
                        self.cursor,
                    );
                    let news_y = l.demo.y + 68.0;
                    ui.text("LATEST NEWS", x, news_y, 1.0, MINT);
                    if let Some(item) = self.news.first() {
                        ui.text(
                            &clip_text(&item.title, inner_w / 6.0),
                            x,
                            news_y + 25.0,
                            1.0,
                            WHITE,
                        );
                        draw_wrapped(
                            ui,
                            &item.body,
                            Rect::new(x, news_y + 45.0, inner_w, 0.0),
                            1.0,
                            GREY,
                        );
                    } else {
                        ui.text(
                            "NEWS APPEARS HERE AFTER A SIGNED RELEASE CHECK.",
                            x,
                            news_y + 25.0,
                            1.0,
                            DIM,
                        );
                    }
                }
            }
            View::Profiles => {
                ui.text("PLAYER PROFILES", x, y, 2.0, WHITE);
                ui.text(
                    "LOCAL NAMES AND VERSION PREFERENCES",
                    x,
                    y + 28.0,
                    1.0,
                    GREY,
                );
                for (index, rect) in l.profile_rows.iter().enumerate() {
                    if let Some(profile) = self.config.profiles.get(index) {
                        let selected = profile.id == self.config.selected_profile;
                        ui.rect(*rect, if selected { PANEL_LIGHT } else { PANEL });
                        ui.rect_outline(*rect, 1.0, BORDER);
                        ui.text(
                            &clip_text(&profile.name, 24.0),
                            rect.x + 10.0,
                            rect.y + 7.0,
                            1.0,
                            if selected { WHITE } else { GREY },
                        );
                        ui.text(
                            &clip_text(&profile.username, 24.0),
                            rect.x + 10.0,
                            rect.y + 23.0,
                            1.0,
                            DIM,
                        );
                    }
                }
                draw_button(
                    ui,
                    l.profile_add,
                    "ADD",
                    self.config.profiles.len() < 4,
                    self.cursor,
                );
                draw_button(
                    ui,
                    l.profile_remove,
                    "REMOVE",
                    self.config.profiles.len() > 1,
                    self.cursor,
                );
                let right_x = l.profile_name.x;
                ui.text("PROFILE NAME", right_x, y + 12.0, 1.0, GREY);
                draw_input(
                    ui,
                    l.profile_name,
                    self.input_display(InputField::ProfileName),
                    self.focused_input == Some(InputField::ProfileName),
                    self.cursor,
                );
                ui.text("OFFLINE PLAYER NAME", right_x, y + 86.0, 1.0, GREY);
                draw_input(
                    ui,
                    l.player_name,
                    self.input_display(InputField::PlayerName),
                    self.focused_input == Some(InputField::PlayerName),
                    self.cursor,
                );
                ui.text("UPDATE CHANNEL", right_x, y + 154.0, 1.0, GREY);
                draw_button(
                    ui,
                    l.profile_stable,
                    "STABLE",
                    self.config.selected_profile().channel == Channel::Stable,
                    self.cursor,
                );
                ui.text("ACCOUNT MODE", right_x, y + 210.0, 1.0, GREY);
                draw_button(
                    ui,
                    l.account_offline,
                    "OFFLINE",
                    !self.config.selected_profile().online,
                    self.cursor,
                );
                draw_button(
                    ui,
                    l.account_online,
                    "MICROSOFT ONLINE",
                    self.config.selected_profile().online
                        || self.valid_signed_in_account().is_some()
                        || client_id_available(),
                    self.cursor,
                );
                let auth_action_label = match &self.auth_state {
                    AuthState::NotConfigured => "MICROSOFT SIGN-IN UNAVAILABLE",
                    AuthState::Idle => "SIGN IN WITH MICROSOFT",
                    AuthState::Starting => "CONTACTING MICROSOFT...",
                    AuthState::Waiting { .. } => "OPEN SIGN-IN PAGE AGAIN",
                    AuthState::SignedIn { .. } => "CHANGE MICROSOFT ACCOUNT",
                    AuthState::Failed(_) => "RETRY MICROSOFT SIGN-IN",
                };
                draw_button(
                    ui,
                    l.auth_action,
                    auth_action_label,
                    !matches!(
                        self.auth_state,
                        AuthState::NotConfigured | AuthState::Starting
                    ),
                    self.cursor,
                );
                let auth_detail = match &self.auth_state {
                    AuthState::NotConfigured => "THIS BUILD HAS NO MICROSOFT APP ID".to_string(),
                    AuthState::Idle => "MICROSOFT SIGN-IN IS OPTIONAL FOR OFFLINE PLAY".into(),
                    AuthState::Starting => "REQUESTING A SHORT-LIVED DEVICE CODE...".into(),
                    AuthState::Waiting {
                        user_code,
                        verification_uri,
                    } => format!("OPEN {verification_uri} AND ENTER {user_code}"),
                    AuthState::SignedIn { name, expires_at } => {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if *expires_at > now {
                            format!("SIGNED IN AS {name}")
                        } else {
                            "MICROSOFT SESSION EXPIRED - SIGN IN AGAIN".into()
                        }
                    }
                    AuthState::Failed(error) => format!("SIGN-IN: {error}"),
                };
                ui.text(
                    &clip_text(&auth_detail, (l.auth_action.w - 10.0) / 6.0),
                    right_x,
                    l.auth_action.y + 40.0,
                    1.0,
                    DIM,
                );
                ui.text(
                    "ALL ONLINE PROFILES SHARE THIS MICROSOFT ACCOUNT",
                    right_x,
                    l.auth_action.y + 50.0,
                    1.0,
                    DIM,
                );
                draw_button(
                    ui,
                    l.profile_preview,
                    "PREVIEW",
                    self.config.selected_profile().channel == Channel::Preview,
                    self.cursor,
                );
            }
            View::Versions => {
                ui.text("VERSIONS", x, y, 2.0, WHITE);
                ui.text(
                    "SELECT A LOCAL GAME VERSION OR CHECK A SIGNED RELEASE",
                    x,
                    y + 28.0,
                    1.0,
                    GREY,
                );
                draw_button(
                    ui,
                    l.stable,
                    "STABLE",
                    self.config.selected_profile().channel == Channel::Stable,
                    self.cursor,
                );
                draw_button(
                    ui,
                    l.preview,
                    "PREVIEW",
                    self.config.selected_profile().channel == Channel::Preview,
                    self.cursor,
                );
                let versions = self.visible_versions();
                ui.text("INSTALLED GAME VERSIONS", x, y + 116.0, 1.0, MINT);
                for (index, rect) in l.version_rows.iter().enumerate() {
                    if let Some(version) = versions.get(self.version_scroll + index) {
                        let selected = self.active_version() == *version
                            || (version == "LATEST"
                                && self.config.selected_profile().version.is_none());
                        ui.rect(*rect, if selected { PANEL_LIGHT } else { PANEL });
                        ui.rect_outline(*rect, 1.0, BORDER);
                        let label = if version == "LATEST" {
                            format!("LATEST ({})", self.active_version())
                        } else {
                            version.clone()
                        };
                        ui.text(
                            &clip_text(&label, (rect.w - 112.0) / 6.0),
                            rect.x + 12.0,
                            rect.y + 12.0,
                            1.0,
                            if selected { WHITE } else { GREY },
                        );
                        let state_label = if selected { "SELECTED" } else { "AVAILABLE" };
                        let state_width = UiBatch::text_width(state_label, 1.0);
                        ui.text(
                            state_label,
                            rect.x + rect.w - state_width - 10.0,
                            rect.y + 12.0,
                            1.0,
                            DIM,
                        );
                    }
                }
                ui.text(
                    "MOUSE WHEEL OR ARROW KEYS TO BROWSE",
                    x,
                    l.version_rows[2].y + 39.0,
                    1.0,
                    DIM,
                );
                let (release_text, notes) = match &self.update_state {
                    UpdateState::NotChecked => {
                        ("UPDATE STATUS: NOT CHECKED".to_string(), String::new())
                    }
                    UpdateState::Checking => {
                        ("CHECKING SIGNED RELEASES...".to_string(), String::new())
                    }
                    UpdateState::Available(release) => (
                        format!(
                            "AVAILABLE: {} ({})",
                            release.manifest.version,
                            release.manifest.channel.label()
                        ),
                        release.manifest.notes.clone(),
                    ),
                    UpdateState::Current(release) => (
                        format!("LATEST: {}", release.manifest.version),
                        release.manifest.notes.clone(),
                    ),
                    UpdateState::Installing => {
                        ("VERIFYING AND INSTALLING...".into(), String::new())
                    }
                    UpdateState::Failed(error) => (format!("CHECK FAILED: {error}"), String::new()),
                };
                ui.text(
                    &clip_text(&release_text, l.update_info.w / 6.0),
                    l.update_info.x,
                    l.update_info.y,
                    1.0,
                    WHITE,
                );
                if !notes.is_empty() {
                    let notes = notes.chars().take(420).collect::<String>();
                    draw_wrapped(
                        ui,
                        &notes,
                        Rect::new(
                            l.update_info.x,
                            l.update_info.y + 24.0,
                            l.update_info.w,
                            0.0,
                        ),
                        1.0,
                        GREY,
                    );
                }
                draw_button(
                    ui,
                    l.check_updates,
                    "CHECK FOR UPDATES",
                    !matches!(
                        self.update_state,
                        UpdateState::Checking | UpdateState::Installing
                    ),
                    self.cursor,
                );
                draw_button(
                    ui,
                    l.install_update,
                    "INSTALL VERIFIED UPDATE",
                    matches!(self.update_state, UpdateState::Available(_)),
                    self.cursor,
                );
            }
            View::News => {
                ui.text("NEPHELE NEWS", x, y, 2.0, WHITE);
                ui.text(
                    "SIGNED RELEASE NOTES - UPDATED WHEN YOU CHECK FOR UPDATES",
                    x,
                    y + 28.0,
                    1.0,
                    GREY,
                );
                if self.news.is_empty() {
                    draw_wrapped(
                        ui,
                        "No signed news feed is cached yet. Connect to the internet and use Versions > Check for Updates. Nephele does not load third-party web pages inside the launcher.",
                        Rect::new(x, y + 76.0, inner_w, 0.0),
                        1.0,
                        DIM,
                    );
                } else {
                    let mut news_y = y + 68.0;
                    for item in self.news.iter().take(8) {
                        ui.rect(Rect::new(x, news_y, inner_w, 1.0), BORDER);
                        ui.text(
                            &clip_text(&item.title, inner_w / 6.0),
                            x,
                            news_y + 14.0,
                            1.0,
                            MINT,
                        );
                        let lines = draw_wrapped(
                            ui,
                            &item.body,
                            Rect::new(x, news_y + 34.0, inner_w, 0.0),
                            1.0,
                            GREY,
                        );
                        news_y += 48.0 + lines as f32 * 11.0;
                    }
                }
            }
            View::Settings => {
                ui.text("SETTINGS AND PRIVACY", x, y, 2.0, WHITE);
                ui.text("CRASH REPORT UPLOADS", x, y + 42.0, 1.0, MINT);
                let checked = self.config.crash_upload_opt_in;
                ui.rect(l.consent, if checked { ACCENT } else { PANEL_LIGHT });
                ui.rect_outline(l.consent, 1.0, BORDER);
                if checked {
                    ui.text(
                        "X",
                        l.consent.x + 9.0,
                        l.consent.y + 8.0,
                        1.0,
                        [0.05, 0.06, 0.08, 1.0],
                    );
                }
                ui.text(
                    "ALLOW ME TO CHOOSE TO SEND CRASH REPORTS",
                    x + 42.0,
                    l.consent.y + 9.0,
                    1.0,
                    WHITE,
                );
                draw_wrapped(
                    ui,
                    "This setting is off by default. Turning it on only makes the Upload button available after a crash; nothing uploads automatically. A report contains Nephele versions, operating system family, exit code and up to 80 recent client log lines. Logs may include server addresses or player names. Known token fields and your home-folder path are redacted. Review the report before sending it.",
                    Rect::new(x, l.consent.y + 46.0, inner_w, 0.0),
                    1.0,
                    GREY,
                );
                ui.rect(
                    l.auto_update,
                    if self.config.auto_update {
                        ACCENT
                    } else {
                        PANEL_LIGHT
                    },
                );
                ui.rect_outline(l.auto_update, 1.0, BORDER);
                if self.config.auto_update {
                    ui.text(
                        "X",
                        l.auto_update.x + 9.0,
                        l.auto_update.y + 8.0,
                        1.0,
                        [0.05, 0.06, 0.08, 1.0],
                    );
                }
                ui.text(
                    "AUTOMATICALLY INSTALL SIGNED UPDATES WHEN USING LATEST",
                    x + 42.0,
                    l.auto_update.y + 9.0,
                    1.0,
                    WHITE,
                );
                ui.text(
                    "UPDATE CHECKS CONTACT GITHUB. NO GAME FILES OR ANALYTICS ARE SENT.",
                    x,
                    l.auto_update.y + 34.0,
                    1.0,
                    DIM,
                );
                draw_button(
                    ui,
                    l.review_crash,
                    "REVIEW LAST CRASH REPORT",
                    self.crash.is_some(),
                    self.cursor,
                );
            }
            View::Legal => {
                ui.text("LEGAL AND PRIVACY", x, y, 2.0, WHITE);
                for (index, rect) in l.legal_tabs.iter().enumerate() {
                    draw_button(
                        ui,
                        *rect,
                        ["TERMS", "PRIVACY", "TRADEMARK"][index],
                        self.legal_page == index,
                        self.cursor,
                    );
                }
                let (heading, body) = legal_page(self.legal_page);
                ui.text(heading, x, y + 82.0, 1.0, MINT);
                draw_wrapped(ui, body, Rect::new(x, y + 108.0, inner_w, 0.0), 1.0, GREY);
            }
            View::Crash => {
                ui.text("THE GAME CLOSED UNEXPECTEDLY", x, y, 2.0, RED);
                if let Some(crash) = &self.crash {
                    ui.text(
                        &format!("REPORT {}", crash.report.report_id),
                        x,
                        y + 42.0,
                        1.0,
                        WHITE,
                    );
                    ui.text(
                        &format!(
                            "CLIENT {}  |  EXIT CODE {:?}  |  {}",
                            crash.report.client_version,
                            crash.report.exit_code,
                            crash.report.platform
                        ),
                        x,
                        y + 64.0,
                        1.0,
                        GREY,
                    );
                    ui.text(
                        &clip_text(
                            &format!("SAVED LOCALLY: {}", crash.local_path.display()),
                            inner_w / 6.0,
                        ),
                        x,
                        y + 88.0,
                        1.0,
                        DIM,
                    );
                    ui.text("RECENT LOG OUTPUT", x, y + 122.0, 1.0, MINT);
                    for (index, line) in crash
                        .report
                        .log_excerpt
                        .iter()
                        .rev()
                        .take(8)
                        .rev()
                        .enumerate()
                    {
                        ui.text(
                            &clip_text(line, inner_w / 6.0),
                            x,
                            y + 145.0 + index as f32 * 15.0,
                            1.0,
                            GREY,
                        );
                    }
                    draw_wrapped(
                        ui,
                        "Upload is off unless you opted in under Settings. Even then, you must press Upload. The report may contain server addresses or usernames; inspect the local JSON before sharing.",
                        Rect::new(x, l.crash_upload.y - 66.0, inner_w, 0.0),
                        1.0,
                        DIM,
                    );
                    if let Some(status) = &crash.upload_status {
                        ui.text(
                            &clip_text(status, inner_w / 6.0),
                            x,
                            l.crash_upload.y - 22.0,
                            1.0,
                            MINT,
                        );
                    }
                }
                let can_upload = self.config.crash_upload_opt_in
                    && option_env!("NEPHELE_CRASH_REPORT_URL").is_some();
                draw_button(
                    ui,
                    l.crash_upload,
                    "UPLOAD CRASH REPORT",
                    can_upload,
                    self.cursor,
                );
                draw_button(
                    ui,
                    l.crash_open,
                    "OPEN REPORT",
                    self.crash.is_some(),
                    self.cursor,
                );
                draw_button(ui, l.crash_back, "BACK TO LAUNCHER", true, self.cursor);
                if !self.config.crash_upload_opt_in {
                    ui.text(
                        "OPT IN UNDER SETTINGS TO ENABLE UPLOAD.",
                        x,
                        l.crash_upload.y - 18.0,
                        1.0,
                        DIM,
                    );
                } else if option_env!("NEPHELE_CRASH_REPORT_URL").is_none() {
                    ui.text(
                        "UPLOAD SERVICE NOT CONFIGURED IN THIS BUILD.",
                        x,
                        l.crash_upload.y - 18.0,
                        1.0,
                        DIM,
                    );
                }
            }
        }
        let _ = height;
    }

    fn input_display(&self, field: InputField) -> &str {
        if self.focused_input == Some(field) {
            &self.input_buffer
        } else {
            match field {
                InputField::ProfileName => &self.config.selected_profile().name,
                InputField::PlayerName => &self.config.selected_profile().username,
            }
        }
    }

    fn handle_keyboard(&mut self, event_loop: &ActiveEventLoop, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        match event.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => {
                if self.focused_input.is_some() {
                    self.focused_input = None;
                } else if self.view == View::Crash {
                    self.view = View::Play;
                } else {
                    event_loop.exit();
                }
            }
            PhysicalKey::Code(KeyCode::Enter) => self.finish_input(),
            PhysicalKey::Code(KeyCode::Backspace) if self.focused_input.is_some() => {
                self.input_buffer.pop();
            }
            PhysicalKey::Code(KeyCode::ArrowDown) if self.view == View::Versions => {
                self.scroll_versions(1);
            }
            PhysicalKey::Code(KeyCode::ArrowUp) if self.view == View::Versions => {
                self.scroll_versions(-1);
            }
            _ => {}
        }
        if let Some(field) = self.focused_input {
            if let Some(text) = event.text.as_deref() {
                let max_len = match field {
                    InputField::ProfileName => 32,
                    InputField::PlayerName => 16,
                };
                for ch in text.chars() {
                    let acceptable = match field {
                        InputField::ProfileName => !ch.is_control(),
                        InputField::PlayerName => ch.is_ascii_alphanumeric() || ch == '_',
                    };
                    if acceptable && self.input_buffer.chars().count() < max_len {
                        self.input_buffer.push(ch);
                    }
                }
            }
        }
    }

    fn frame(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let (width, height) = renderer.size();
        self.handle_worker_messages();
        self.tick_child();
        let ui = self.build_ui(width as f32, height as f32);
        let empty = UiBatch::default();
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_ui(&empty, &empty, &ui);
            renderer.draw(&self.camera);
        }
        window.set_title(WINDOW_TITLE);
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
            WindowEvent::CloseRequested => {
                self.finish_input();
                self.save_config();
                event_loop.exit();
            }
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
            WindowEvent::MouseWheel { delta, .. } if self.view == View::Versions => {
                let direction = match delta {
                    MouseScrollDelta::LineDelta(_, amount) => amount as isize,
                    MouseScrollDelta::PixelDelta(position) => {
                        if position.y > 0.0 {
                            1
                        } else if position.y < 0.0 {
                            -1
                        } else {
                            0
                        }
                    }
                };
                self.scroll_versions(-direction);
            }
            WindowEvent::KeyboardInput { event, .. } => self.handle_keyboard(event_loop, &event),
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

pub fn run() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nephele_launcher=info".into()),
        )
        .with_target(false)
        .without_time()
        .init();

    let args = std::env::args().collect::<Vec<_>>();
    let paths = Paths::discover(&args);
    let (mut config, config_warning) = match LauncherConfig::load(&paths.config_path()) {
        Ok(config) => (config, None),
        Err(error) => (LauncherConfig::default(), Some(error)),
    };
    if let Some(username) = argument_value(&args, "--username") {
        if username.len() <= 16
            && username
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            config.selected_profile_mut().username = username.to_string();
        }
    }

    if bootstrap_current_version(&args, &paths) {
        return Ok(());
    }
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut launcher = Launcher::new(paths, config, config_warning);
    event_loop.run_app(&mut launcher)?;
    match launcher.fatal_error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn bootstrap_current_version(args: &[String], paths: &Paths) -> bool {
    if args.iter().any(|arg| arg == "--no-bootstrap") {
        return false;
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return false,
    };
    let Some(version_dir) = exe.parent() else {
        return false;
    };
    if version_dir
        .parent()
        .and_then(Path::file_name)
        .is_none_or(|name| name != "versions")
    {
        return false;
    }
    let Ok(selected) = fs::read_to_string(paths.install_root.join("current-version.txt")) else {
        return false;
    };
    let selected = selected.trim();
    if !crate::profiles::valid_version_id(selected)
        || version_dir.file_name().is_some_and(|name| name == selected)
    {
        return false;
    }
    let target = paths
        .versions_root
        .join(selected)
        .join(exe_name("nephele-launcher"));
    if !target.is_file() {
        return false;
    }
    Command::new(target)
        .arg("--install-root")
        .arg(&paths.install_root)
        .spawn()
        .is_ok()
}

fn prepare_resources(jar: &Path, lock_path: &Path, out_dir: &Path) -> Result<(), PrepareError> {
    let lock = Lock::load(lock_path)?;
    nephele_prepare::prepare(jar, out_dir, &lock).map(|_| ())
}

fn spawn_log_reader<R: std::io::Read + Send + 'static>(reader: R, sender: Sender<String>) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
}

fn user_data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("NEPHELE_DATA_DIR") {
        return PathBuf::from(path);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("Nephele");
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data).join("nephele");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/nephele");
    }
    PathBuf::from(".nephele")
}

fn load_auth_state(data_dir: &Path) -> AuthState {
    if let Ok(session) = nephele_auth::StoredSession::load(&data_dir.join("auth.json")) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if session.expires_at > now {
            return AuthState::SignedIn {
                name: session.name,
                expires_at: session.expires_at,
            };
        }
        return AuthState::Failed("Saved Microsoft session expired; sign in again.".into());
    }
    if client_id_available() {
        AuthState::Idle
    } else {
        AuthState::NotConfigured
    }
}

fn client_id_available() -> bool {
    option_env!("NEPHELE_CLIENT_ID").is_some_and(|client_id| !client_id.trim().is_empty())
}

fn minecraft_root(paths: &Paths) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NEPHELE_MINECRAFT_ASSETS").map(PathBuf::from) {
        if path.is_dir() {
            return Some(path);
        }
    }
    [
        paths.prepared_dir().join("assets/minecraft"),
        paths
            .install_root
            .join(".local/prepared/1.8.9/assets/minecraft"),
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".local/prepared/1.8.9/assets/minecraft"),
    ]
    .into_iter()
    .find(|path| path.is_dir())
}

fn ancestors(path: &Path) -> Vec<PathBuf> {
    path.ancestors().map(Path::to_path_buf).collect()
}

fn argument_value<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == key)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn executable_version(executable_dir: &Path) -> String {
    if executable_dir
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "versions")
    {
        if let Some(version) = executable_dir.file_name().and_then(|name| name.to_str()) {
            if crate::profiles::valid_version_id(version) {
                return version.to_string();
            }
        }
    }
    env!("CARGO_PKG_VERSION").to_string()
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn launcher_icon() -> Option<Icon> {
    const SIZE: usize = 64;
    let mut pixels = vec![0u8; SIZE * SIZE * 4];
    fill_icon_rect(&mut pixels, SIZE, 0, 0, SIZE, SIZE, [10, 16, 27, 255]);
    fill_icon_rect(&mut pixels, SIZE, 8, 18, 16, 16, [250, 171, 79, 255]);
    fill_icon_rect(&mut pixels, SIZE, 24, 10, 16, 24, [92, 212, 194, 255]);
    fill_icon_rect(&mut pixels, SIZE, 40, 18, 16, 16, [250, 171, 79, 255]);
    fill_icon_rect(&mut pixels, SIZE, 16, 34, 32, 6, [92, 212, 194, 255]);
    Icon::from_rgba(pixels, SIZE as u32, SIZE as u32).ok()
}

fn fill_icon_rect(
    pixels: &mut [u8],
    size: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: [u8; 4],
) {
    for py in y..(y + height).min(size) {
        for px in x..(x + width).min(size) {
            let index = (py * size + px) * 4;
            pixels[index..index + 4].copy_from_slice(&color);
        }
    }
}

fn open_local_file(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut command = Command::new("notepad.exe");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command.arg(path).spawn().map(|_| ())
}

fn open_url(url: &str) -> std::io::Result<()> {
    if !url.starts_with("https://") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to open a non-HTTPS sign-in URL",
        ));
    }
    #[cfg(windows)]
    let mut command = Command::new("rundll32.exe");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    #[cfg(windows)]
    command.args(["url.dll,FileProtocolHandler", url]);
    #[cfg(not(windows))]
    command.arg(url);
    command.spawn().map(|_| ())
}

fn draw_button(ui: &mut UiBatch, rect: Rect, label: &str, enabled: bool, cursor: (f32, f32)) {
    let hovered = enabled && rect.contains(cursor.0, cursor.1);
    let color = if !enabled {
        [0.12, 0.14, 0.18, 1.0]
    } else if hovered {
        ACCENT
    } else {
        PANEL_LIGHT
    };
    ui.rect(rect, color);
    ui.rect_outline(rect, 1.0, if hovered { ACCENT_HOVER } else { BORDER });
    let label_width = UiBatch::text_width(label, 1.0);
    ui.text(
        label,
        rect.x + (rect.w - label_width) / 2.0,
        rect.y + (rect.h - 8.0) / 2.0,
        1.0,
        if hovered {
            [0.04, 0.05, 0.07, 1.0]
        } else if enabled {
            WHITE
        } else {
            DIM
        },
    );
}

fn draw_input(ui: &mut UiBatch, rect: Rect, value: &str, focused: bool, cursor: (f32, f32)) {
    ui.rect(rect, PANEL);
    ui.rect_outline(
        rect,
        1.0,
        if focused {
            ACCENT
        } else if rect.contains(cursor.0, cursor.1) {
            MINT
        } else {
            BORDER
        },
    );
    ui.text(
        &clip_text(value, (rect.w - 20.0) / 6.0),
        rect.x + 10.0,
        rect.y + 15.0,
        1.0,
        WHITE,
    );
}

fn draw_wrapped(ui: &mut UiBatch, text: &str, rect: Rect, scale: f32, color: [f32; 4]) -> usize {
    let max_columns = (rect.w / (6.0 * scale)).floor().max(1.0) as usize;
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max_columns {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            for ch in word.chars() {
                if line.chars().count() >= max_columns {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    for (index, line) in lines.iter().enumerate() {
        ui.text(
            line,
            rect.x,
            rect.y + index as f32 * (10.0 * scale),
            scale,
            color,
        );
    }
    lines.len()
}

fn clip_text(text: &str, max_chars: f32) -> String {
    let count = max_chars.max(0.0).floor() as usize;
    let mut chars = text.chars();
    let clipped = chars.by_ref().take(count).collect::<String>();
    if chars.next().is_some() && count >= 3 {
        format!("{}...", clipped.chars().take(count - 3).collect::<String>())
    } else {
        clipped
    }
}

fn legal_page(index: usize) -> (&'static str, &'static str) {
    match index {
        0 => (
            "TERMS OF USE",
            "Nephele is independent software provided as-is, without warranty. Use it only with a Minecraft: Java Edition copy you are entitled to use. Nephele does not include, download, or redistribute Minecraft code, game assets, or account credentials. Multiplayer access is controlled by the server and may require an eligible Microsoft account. You are responsible for your account, server choices, and compliance with applicable terms and laws. No affiliation with Mojang or Microsoft is implied.",
        ),
        1 => (
            "PRIVACY",
            "Profiles, launcher settings, logs, and prepared resources are stored on this device. If you sign in, Microsoft session tokens are saved locally in auth.json and used only for online server authentication; the launcher does not upload them. Update checks contact GitHub and disclose the network address and standard HTTP metadata needed to fetch signed release information. Analytics are not enabled. Crash reports are stored locally. Upload is off by default and requires both opting in under Settings and pressing Upload after a crash. An uploaded report contains app versions, operating system family, exit status, and recent client log lines; those lines can include server addresses or player names. Nephele redacts known token fields and the home-folder path, but you can inspect the local JSON before sending. No game jar, textures, world, or auth.json is attached.",
        ),
        _ => (
            "TRADEMARK AND NON-AFFILIATION",
            "Nephele is not an official Minecraft product and is not approved by, endorsed by, or associated with Mojang or Microsoft. Minecraft is a trademark of Mojang Synergies AB. Nephele is an independent, from-scratch client. The game copy and resources used by the client must come from the player's own installation; Nephele does not distribute them.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_layout_keeps_navigation_and_action_buttons_in_bounds() {
        for (width, height) in [(760.0, 560.0), (1040.0, 660.0), (1920.0, 1080.0)] {
            let layout = layout(width, height);
            for rect in layout.nav {
                assert!(rect.x >= 0.0 && rect.y >= 0.0);
                assert!(rect.x + rect.w <= width);
                assert!(rect.y + rect.h <= height);
            }
            assert!(layout.content.x + layout.content.w <= width + 1.0);
            assert!(layout.content.y + layout.content.h <= height + 1.0);
            assert!(!layout.launch.contains(layout.demo.x, layout.demo.y));
        }
    }

    #[test]
    fn text_clipping_does_not_exceed_requested_width() {
        assert_eq!(clip_text("NEPHELE", 4.0), "N...");
        assert_eq!(clip_text("OK", 4.0), "OK");
    }

    #[test]
    fn legal_pages_include_the_non_affiliation_disclaimer() {
        assert!(legal_page(2)
            .1
            .contains("not an official Minecraft product"));
        assert!(legal_page(1).1.contains("Upload is off by default"));
    }

    #[test]
    fn launcher_brand_icon_is_generated_without_external_art() {
        assert!(launcher_icon().is_some());
    }

    #[test]
    fn locally_saved_microsoft_sessions_are_detected_and_expiry_is_reported() {
        let directory =
            std::env::temp_dir().join(format!("nephele-launcher-auth-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("auth.json");
        let session = nephele_auth::StoredSession {
            client_id: "public-client-id".into(),
            access_token: "test-token".into(),
            expires_at: u64::MAX,
            id: "profile-id".into(),
            name: "OnlinePlayer".into(),
            refresh_token: None,
        };
        session.save(&path).unwrap();
        assert!(matches!(
            load_auth_state(&directory),
            AuthState::SignedIn { ref name, .. } if name == "OnlinePlayer"
        ));
        let expired = nephele_auth::StoredSession {
            expires_at: 0,
            ..session
        };
        expired.save(&path).unwrap();
        assert!(matches!(load_auth_state(&directory), AuthState::Failed(_)));
        let _ = std::fs::remove_dir_all(directory);
    }
}
