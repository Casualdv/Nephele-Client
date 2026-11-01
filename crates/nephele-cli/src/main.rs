mod commands;
mod live;
mod mesher;
mod paths;
mod servers;
mod settings;
mod skins;
mod ui;
mod viewer;

use std::future::Future;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use nephele_client::GameClient;
use nephele_mock::{MockConfig, MockServer};
use nephele_protocol::packets::play;
use nephele_protocol::{Connection, Error, LoginOutcome, NextState, Result, ServerAddress};

#[derive(Parser)]
#[command(
    name = "nephele",
    version,
    about = "Nephele Client tools - Minecraft Java Edition 1.8.9 (protocol 47)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ping a server: MOTD, version, player counts and round-trip latency.
    Status {
        /// host, host:port, or [ipv6]:port (port defaults to 25565)
        address: String,
        /// Print the raw JSON response instead of a summary
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 5)]
        timeout: u64,
    },
    /// Log in to an offline-mode server, wait for Join Game, then stay connected.
    Login {
        /// host, host:port, or [ipv6]:port (port defaults to 25565)
        address: String,
        /// Username to present
        #[arg(short = 'u', long, default_value = "Nephele")]
        username: String,
        /// How long to hold the connection open after joining
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Optional chat message to send once in-game
        #[arg(long)]
        chat: Option<String>,
        /// Connect/read timeout in seconds
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Join a server headlessly, load chunks, and print a world summary.
    Join {
        /// host, host:port, or [ipv6]:port (port defaults to 25565)
        address: String,
        /// Username to present
        #[arg(short = 'u', long, default_value = "Nephele")]
        username: String,
        /// How long to stay connected
        #[arg(long, default_value_t = 8)]
        seconds: u64,
        /// Optional chat message to send
        #[arg(long)]
        chat: Option<String>,
        /// Connect timeout in seconds
        #[arg(long, default_value_t = 10)]
        timeout: u64,
        /// Don't print the block histogram
        #[arg(long)]
        no_blocks: bool,
    },
    /// Run a local fake 1.8.9 server with a deterministic world (for testing).
    MockServer {
        /// Port to listen on
        #[arg(long, default_value_t = 25565)]
        port: u16,
        /// Chunk radius sent on join
        #[arg(long, default_value_t = 2)]
        radius: i32,
        /// Disable zlib compression
        #[arg(long)]
        no_compression: bool,
        /// Spawn fake walking players, for testing entity rendering
        #[arg(long)]
        bots: bool,
        /// Join as survival: mining timing, drops and pickups are exercised
        #[arg(long)]
        survival: bool,
    },
    /// Join a server and play it in a window (offline mode, WASD + right-drag).
    Play {
        /// host, host:port, or [ipv6]:port. Omit it to open the title screen.
        address: Option<String>,
        /// Username to present
        #[arg(short = 'u', long, default_value = "Nephele")]
        username: String,
        /// Override the vanilla `textures/blocks` directory
        #[arg(long)]
        assets_dir: Option<PathBuf>,
        /// Start flying (F toggles in game)
        #[arg(long)]
        fly: bool,
        /// Single-player world seed (random when omitted)
        #[arg(long)]
        seed: Option<u64>,
        /// Join with the cached Microsoft session (online-mode servers)
        #[arg(long)]
        online: bool,
    },
    /// Sign in with a Microsoft account (device code) and cache the session.
    Auth {
        /// Azure application client id (or set NEPHELE_CLIENT_ID)
        #[arg(long)]
        client_id: Option<String>,
        /// Where to write the cached session
        #[arg(long)]
        out: Option<PathBuf>,
        /// Use the legacy login.live.com endpoint (testing only; needs a legacy app id)
        #[arg(long)]
        legacy: bool,
    },
    /// Render the synthetic world in a window (WASD + right-drag to look).
    View {
        /// Chunk radius to generate
        #[arg(long, default_value_t = 3)]
        radius: i32,
        /// Override the vanilla `textures/blocks` directory
        #[arg(long)]
        assets_dir: Option<PathBuf>,
        /// Flight speed in blocks per second
        #[arg(long, default_value_t = 24.0)]
        speed: f32,
        /// World seed
        #[arg(long, default_value_t = 0)]
        seed: i64,
    },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nephele=info,nephele_protocol=info".into()),
        )
        .with_target(false)
        .without_time()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Play {
            address,
            username,
            assets_dir,
            fly,
            seed,
            online,
        } => {
            let address = match address {
                Some(address) => match ServerAddress::parse(&address) {
                    Ok(address) => Some(address),
                    Err(err) => {
                        eprintln!("error: {err}");
                        return ExitCode::FAILURE;
                    }
                },
                None => None,
            };
            let assets_dir = assets_dir.unwrap_or_else(default_assets_dir);
            migrate_legacy_state();
            match live::run(live::PlayOptions {
                address,
                username,
                assets_dir,
                fly,
                seed,
                online,
                session_path: session_path(),
                servers_path: servers_path(),
            }) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::View {
            radius,
            assets_dir,
            speed,
            seed,
        } => {
            let assets_dir = assets_dir.unwrap_or_else(default_assets_dir);
            match viewer::run(viewer::ViewerOptions {
                radius,
                assets_dir,
                speed,
                seed,
            }) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Auth {
            client_id,
            out,
            legacy,
        } => run_auth(client_id, out, legacy),
        command => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to start tokio runtime");
            match runtime.block_on(run_command(command)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// The two failures people actually hit, with what to change.
fn explain_auth_failure(message: &str) {
    if message.contains("AADSTS700016") || message.contains("does not exist in tenant") {
        eprintln!();
        eprintln!("  That client id is not an app in the consumer tenant.");
        eprintln!("  Check it is the Application (client) id from your own App registration,");
        eprintln!("  and that its supported account types include personal Microsoft accounts.");
    } else if message.contains("AADSTS7000218") || message.contains("public client") {
        eprintln!();
        eprintln!("  The app must allow public client flows:");
        eprintln!("  App registration -> Authentication -> Allow public client flows -> Yes.");
    }
}
fn run_auth(client_id: Option<String>, out: Option<PathBuf>, legacy: bool) -> ExitCode {
    use std::io::Write;

    let client_id = client_id
        .or_else(|| std::env::var("NEPHELE_CLIENT_ID").ok())
        .unwrap_or_default();

    if client_id.trim().is_empty() {
        eprintln!("error: no client id configured");
        eprintln!(
            "       register an Azure application (public client, device code flow enabled),"
        );
        eprintln!("       then run `nephele auth --client-id <id>` or set NEPHELE_CLIENT_ID");
        return ExitCode::FAILURE;
    }

    println!("  client id: {client_id}");

    let code = match nephele_auth::begin_with(&client_id, legacy) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            explain_auth_failure(&err.to_string());
            return ExitCode::FAILURE;
        }
    };

    println!();
    println!("  To sign in, open {}", code.verification_uri);
    println!("  and enter the code:  {}", code.user_code);
    println!();
    print!("  waiting");
    let _ = std::io::stdout().flush();

    let token = match nephele_auth::wait_for_token_with(&client_id, &code, legacy, |_| {
        print!(".");
        let _ = std::io::stdout().flush();
    }) {
        Ok(token) => token,
        Err(err) => {
            println!();
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!(" done");

    let session = match nephele_auth::to_minecraft(&client_id, &token) {
        Ok(session) => session,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "  signed in as {} ({})",
        session.profile.name, session.profile.id
    );

    let path = out.unwrap_or_else(session_path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let stored = nephele_auth::StoredSession::from_session(&client_id, &session);
    match stored.save(&path) {
        Ok(()) => {
            println!("  session saved to {}", path.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: could not save the session: {err}");
            ExitCode::FAILURE
        }
    }
}
async fn run_command(command: Command) -> Result<()> {
    match command {
        Command::View { .. } | Command::Play { .. } | Command::Auth { .. } => {
            unreachable!("windowed commands are handled synchronously")
        }
        Command::Status {
            address,
            json,
            timeout,
        } => run_status(&address, json, timeout).await,
        Command::Login {
            address,
            username,
            seconds,
            chat,
            timeout,
        } => run_login(&address, &username, seconds, chat.as_deref(), timeout).await,
        Command::Join {
            address,
            username,
            seconds,
            chat,
            timeout,
            no_blocks,
        } => {
            run_join(
                &address,
                &username,
                seconds,
                chat.as_deref(),
                timeout,
                no_blocks,
            )
            .await
        }
        Command::MockServer {
            port,
            radius,
            no_compression,
            bots,
            survival,
        } => run_mock_server(port, radius, no_compression, bots, survival).await,
    }
}

/// The vanilla `assets/minecraft` tree.
///
/// Nephele ships none of it: `nephele-prepare` derives it from the player's own
/// client jar. A preparation under `.local/prepared` wins over the older in-repo
/// cache, and `NEPHELE_MINECRAFT_ASSETS` overrides both.
/// The workspace root, which is where `.local/prepared` and `assets/vanilla`
/// live.
///
/// Resolved from the working directory first, then by walking up from the
/// executable — launching `nephele.exe` from Explorer or from `target\debug`
/// sets the working directory to the binary's folder, and looking for
/// resources there finds nothing.
///
/// A `[workspace]` manifest wins over a prepared tree, because a stray
/// `.local/prepared` can exist inside `target` and would otherwise be picked
/// just for being closer.
fn project_root() -> PathBuf {
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

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn minecraft_assets_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("NEPHELE_MINECRAFT_ASSETS").map(PathBuf::from) {
        return dir;
    }
    let root = project_root();
    let prepared = root.join(".local/prepared/1.8.9/assets/minecraft");
    if prepared.is_dir() {
        return prepared;
    }
    root.join("assets/vanilla/1.8.9/assets/minecraft")
}

fn default_assets_dir() -> PathBuf {
    minecraft_assets_root().join("textures").join("blocks")
}

/// Where the cached Microsoft session lives. Per-user, not per-install: a game
/// directory can be read-only, and two installs of the client have no business
/// sharing an identity.
fn session_path() -> PathBuf {
    paths::user_data_dir().join("auth.json")
}

fn servers_path() -> PathBuf {
    paths::user_data_dir().join("servers.json")
}

/// Copy any `auth.json` / `servers.json` left in the old per-install `.local/`
/// directory into the user-data directory, once.
fn migrate_legacy_state() {
    let root = project_root();
    for (new, legacy) in [
        (session_path(), root.join(".local/auth.json")),
        (servers_path(), root.join(".local/servers.json")),
    ] {
        if let Err(err) = paths::migrate_legacy_file(&new, &legacy) {
            eprintln!("could not migrate {}: {err}", legacy.display());
        }
    }
}

async fn run_status(input: &str, json: bool, timeout_secs: u64) -> Result<()> {
    let address = ServerAddress::parse(input)?;
    let timeout = Duration::from_secs(timeout_secs.max(1));

    let mut connection = with_timeout(timeout, Connection::connect(&address)).await?;
    connection.handshake(&address, NextState::Status).await?;
    let status = with_timeout(timeout, connection.status()).await?;
    connection.shutdown().await;

    if json {
        println!("{}", serde_json::to_string_pretty(&status.raw)?);
        return Ok(());
    }

    let protocol_ok = match status.protocol {
        Some(protocol) => protocol == nephele_protocol::PROTOCOL_VERSION,
        None => false,
    };

    println!("Server   {}", address);
    println!(
        "Version  {} (protocol {}){}",
        status.version_name.as_deref().unwrap_or("unknown"),
        status
            .protocol
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".into()),
        if protocol_ok {
            "  [compatible]"
        } else {
            "  [not protocol 47]"
        }
    );
    println!(
        "Players  {}/{}",
        status
            .online_players
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".into()),
        status
            .max_players
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".into())
    );
    if let Some(latency) = status.latency {
        println!("Latency  {} ms", latency.as_millis());
    }
    println!("MOTD");
    for line in strip_formatting(&status.description).lines() {
        println!("  {line}");
    }

    Ok(())
}

async fn run_login(
    input: &str,
    username: &str,
    seconds: u64,
    chat: Option<&str>,
    timeout_secs: u64,
) -> Result<()> {
    let address = ServerAddress::parse(input)?;
    let timeout = Duration::from_secs(timeout_secs.max(1));

    let mut connection = with_timeout(timeout, Connection::connect(&address)).await?;
    println!("Connected to {address}");

    connection.handshake(&address, NextState::Login).await?;
    match with_timeout(timeout, connection.login_start(username)).await? {
        LoginOutcome::Success(success) => {
            println!("Logged in as {} ({})", success.username, success.uuid);
        }
        LoginOutcome::Disconnect(reason) => {
            println!("Server rejected the login: {reason}");
            connection.shutdown().await;
            return Ok(());
        }
        LoginOutcome::EncryptionRequest(_) => {
            println!(
                "Server is in online mode (it sent an Encryption Request). \
                 Nephele currently supports offline-mode servers only."
            );
            connection.shutdown().await;
            return Ok(());
        }
    }

    if let Some(threshold) = connection.compression_threshold() {
        println!("Compression threshold: {threshold} bytes");
    }

    let join = with_timeout(timeout, connection.wait_for_join_game()).await?;
    println!(
        "Joined game: entity {}, gamemode {}, dimension {}, level type \"{}\", {}/{} players",
        join.entity_id,
        join.gamemode,
        join.dimension,
        join.level_type,
        join.max_players,
        join.max_players
    );

    if let Some(message) = chat {
        connection.send_chat(message).await?;
        println!("Sent chat: {message}");
    }

    println!("Holding the connection for {seconds}s (Ctrl+C to stop)...");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut printed_position = false;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        let packet = match tokio::time::timeout(remaining, connection.next_play_packet()).await {
            Err(_) => break,
            Ok(Ok(packet)) => packet,
            Ok(Err(err @ Error::Disconnected(_))) => {
                println!("Disconnected: {err}");
                break;
            }
            Ok(Err(err)) => return Err(err),
        };

        match packet.id {
            play::clientbound::KEEP_ALIVE => tracing::debug!("answered keep-alive"),
            play::clientbound::CHAT_MESSAGE => {
                if let Ok((text, position)) = play::parse_chat_message(&packet) {
                    println!("[chat:{position}] {}", strip_formatting(&text));
                }
            }
            play::clientbound::PLAYER_POSITION_AND_LOOK if !printed_position => {
                if let Ok(pos) = play::parse_player_position_and_look(&packet) {
                    println!(
                        "Spawn position: {:.2} / {:.2} / {:.2} (yaw {:.1}, pitch {:.1})",
                        pos.x, pos.y, pos.z, pos.yaw, pos.pitch
                    );
                    printed_position = true;
                }
            }
            _ => {}
        }
    }

    connection.shutdown().await;
    println!("Connection closed.");
    Ok(())
}

async fn run_join(
    input: &str,
    username: &str,
    seconds: u64,
    chat: Option<&str>,
    timeout_secs: u64,
    no_blocks: bool,
) -> Result<()> {
    let address = ServerAddress::parse(input)?;
    let timeout = Duration::from_secs(timeout_secs.max(1));

    let mut client = with_timeout(timeout, GameClient::join(&address, username)).await?;
    println!("Connected to {address}");
    println!(
        "Logged in as {} ({})",
        client.info.username, client.info.uuid
    );
    println!(
        "Join Game: entity {}, gamemode {}, dimension {}, level type \"{}\", max {} players",
        client.info.entity_id,
        client.info.gamemode,
        client.info.dimension,
        client.info.level_type,
        client.info.max_players
    );
    if let Some(threshold) = client.info.compression {
        println!("Compression threshold: {threshold} bytes");
    }

    // Give the server time to send the spawn position and its chunk batch.
    client
        .pump(Duration::from_secs(2), Duration::from_millis(50))
        .await?;

    if let Some(message) = chat {
        client.send_chat(message).await?;
        client
            .pump(Duration::from_millis(800), Duration::from_millis(50))
            .await?;
    }

    let remaining = Duration::from_secs(seconds).saturating_sub(Duration::from_secs(2));
    if !remaining.is_zero() {
        client.pump(remaining, Duration::from_millis(100)).await?;
    }

    println!();
    println!("Position   {}", client.position);
    if let Some((x, y, z)) = client.spawn_position {
        println!("Spawn      {x} / {y} / {z}");
    }
    println!(
        "Chunks     {} loaded ({} columns received), {} unloaded",
        client.world.loaded_chunks(),
        client.chunks_loaded,
        client.chunks_unloaded
    );
    let column_top = client
        .world
        .height_at(
            client.position.x.floor() as i32,
            client.position.z.floor() as i32,
        )
        .map(|y| y.to_string())
        .unwrap_or_else(|| "-".to_string());
    println!(
        "Blocks     {} non-air, top of this column at y={}",
        client.world.total_non_air(),
        column_top
    );
    println!(
        "Time       day {}, health {:.1}, food {}",
        client.time_of_day % 24_000,
        client.health,
        client.food
    );
    println!("Packets    {} received", client.packets_received);

    if !no_blocks {
        let histogram = client.world.block_histogram();
        if !histogram.is_empty() {
            println!();
            println!("Top blocks:");
            for (id, count) in histogram.iter().take(6) {
                println!("  {:<5} {count:>10}", block_name(*id));
            }
        }
    }

    client.disconnect().await;
    Ok(())
}

async fn run_mock_server(
    port: u16,
    radius: i32,
    no_compression: bool,
    bots: bool,
    survival: bool,
) -> Result<()> {
    let config = MockConfig {
        radius,
        compression_threshold: if no_compression { None } else { Some(64) },
        greeting: true,
        bots,
        survival,
    };
    let server = MockServer::start_on(port, config).await?;
    println!("Nephele mock server listening on {}", server.address());
    if bots {
        println!(
            "Spawning {} walking players around spawn.",
            nephele_mock::BOT_COUNT
        );
    }
    println!(
        "World: {} chunk columns, compression {}",
        (radius * 2 + 1).pow(2),
        if no_compression {
            "off"
        } else {
            "on (64 bytes)"
        }
    );
    println!("Press Ctrl+C to stop.");

    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

fn block_name(id: u16) -> &'static str {
    match id {
        0 => "air",
        1 => "stone",
        2 => "grass",
        3 => "dirt",
        4 => "cobble",
        5 => "planks",
        7 => "bedrock",
        9 => "water",
        12 => "sand",
        13 => "gravel",
        17 => "log",
        18 => "leaves",
        20 => "glass",
        24 => "sandstone",
        78 => "snow",
        _ => "block",
    }
}

async fn with_timeout<T>(duration: Duration, future: impl Future<Output = Result<T>>) -> Result<T> {
    match tokio::time::timeout(duration, future).await {
        Ok(result) => result,
        Err(_) => Err(Error::Protocol(format!(
            "timed out after {}s",
            duration.as_secs()
        ))),
    }
}

/// Strip legacy `Â§` formatting codes so MOTDs are readable in a terminal.
fn strip_formatting(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{00a7}' {
            chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}
