//! `nephele-prepare` - the one-time preparation step.
//!
//! ```text
//! nephele-prepare                     build resources from the jar you already have
//! nephele-prepare --minecraft-dir DIR read the jar from your launcher's install
//! nephele-prepare --jar PATH          read a specific client jar
//! nephele-prepare --verify-only       rehash an existing preparation
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use nephele_prepare::{client_jar_candidates, Lock, PrepareError, MANIFEST_NAME, VERSION};

#[derive(Debug, Parser)]
#[command(
    name = "nephele-prepare",
    about = "Build Nephele's resources from your own copy of Minecraft: Java Edition 1.8.9",
    long_about = "Nephele contains no Minecraft code, assets or data. This step reads the 1.8.9 client jar you \
already own, checks it against the hash pinned in research-lock.json, and extracts the textures, models \
and language files the client reads into a folder of its own. Nothing is downloaded and nothing is shared."
)]
struct Cli {
    /// Your Minecraft 1.8.9 install: a game directory, a Prism/MultiMC instance, or a launcher root.
    #[arg(long, value_name = "DIR")]
    minecraft_dir: Option<PathBuf>,

    /// The 1.8.9 client jar, if you prefer to point at it directly.
    #[arg(long, value_name = "JAR")]
    jar: Option<PathBuf>,

    /// Where prepared resources go. Defaults to .local/prepared.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,

    /// Only check an existing preparation, then exit.
    #[arg(long)]
    verify_only: bool,

    /// Extract again even if the preparation already looks current.
    #[arg(long)]
    force: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("nephele-prepare: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), PrepareError> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let out_root = cli
        .out
        .clone()
        .or_else(|| std::env::var_os("NEPHELE_PREPARED_OUT").map(PathBuf::from))
        .unwrap_or_else(|| cwd.join(".local/prepared"));
    let prepared_dir = out_root.join(VERSION);

    let lock_path = Lock::find(&cwd)
        .or_else(|| Lock::find(&out_root))
        .ok_or_else(|| {
            PrepareError::Message(format!(
                "could not find {}\nrun this from the Nephele checkout",
                nephele_prepare::LOCK_FILE_NAME
            ))
        })?;
    let lock = Lock::load(&lock_path)?;

    if cli.verify_only {
        let (prepared, report) = nephele_prepare::verify(&prepared_dir, &lock)?;
        println!("Prepared for Minecraft {}", prepared.minecraft);
        println!("  output     {}", prepared_dir.display());
        println!("  files      {}", prepared.file_count);
        println!("  digest     {}", prepared.digest);
        println!("  checked    {}", report.checked);
        if report.ok() {
            println!("\nVerified: every file matches {MANIFEST_NAME}.");
            return Ok(());
        }
        for path in report.missing.iter().take(10) {
            println!("  missing    {path}");
        }
        for path in report.mismatched.iter().take(10) {
            println!("  modified   {path}");
        }
        for path in report.extra.iter().take(10) {
            println!("  unexpected {path}");
        }
        return Err(PrepareError::Message(format!(
            "{} problem(s) found; run without --verify-only to prepare again",
            report.problems()
        )));
    }

    if cli.minecraft_dir.is_some() && cli.jar.is_some() {
        return Err(PrepareError::Message(
            "pass either --minecraft-dir or --jar, not both".to_string(),
        ));
    }

    let jar = resolve_jar(&cli, &prepared_dir)?;
    let jar_sha1 = nephele_prepare::sha1_file(&jar)?;

    if !cli.force {
        if let Some(existing) = read_manifest(&prepared_dir) {
            let current = existing.jar_sha1 == jar_sha1
                && existing.output_version == lock.preparer.output_version;
            if current {
                println!("Prepared for Minecraft {} already.", existing.minecraft);
                println!("  output     {}", prepared_dir.display());
                println!("  files      {}", existing.file_count);
                println!("  digest     {}", existing.digest);
                println!("\nNothing to do. Pass --force to extract again.");
                return Ok(());
            }
        }
    }

    println!("Nephele prepare - Minecraft Java Edition {VERSION}");
    println!("  jar        {}", jar.display());
    println!("  sha1       {jar_sha1}");
    println!("  output     {}", prepared_dir.display());
    println!();

    let prepared = nephele_prepare::prepare(&jar, &prepared_dir, &lock)?;
    let bytes = dir_size(&prepared_dir.join("assets")).unwrap_or(0);

    println!(
        "Extracted {} file(s), {:.2} MiB.",
        prepared.file_count,
        bytes as f64 / (1024.0 * 1024.0)
    );
    println!("  digest     {}", prepared.digest);
    println!("\nPrepared. Start the client with:");
    println!("  cargo run -p nephele -- play");
    Ok(())
}

fn read_manifest(dir: &Path) -> Option<nephele_prepare::Prepared> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_NAME)).ok()?;
    serde_json::from_str(&text).ok()
}

fn dir_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += dir_size(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

fn resolve_jar(cli: &Cli, prepared_dir: &Path) -> Result<PathBuf, PrepareError> {
    if let Some(jar) = &cli.jar {
        return jar.is_file().then(|| jar.clone()).ok_or_else(|| {
            PrepareError::Message(format!("--jar {} does not exist", jar.display()))
        });
    }

    if let Some(dir) = &cli.minecraft_dir {
        let candidates = client_jar_candidates(dir);
        if let Some(found) = candidates.iter().find(|p| p.is_file()) {
            return Ok(found.clone());
        }
        let looked: Vec<String> = candidates
            .iter()
            .map(|p| format!("  {}", p.display()))
            .collect();
        return Err(PrepareError::Message(format!(
            "no Minecraft {VERSION} client jar under {}.\nLooked in:\n{}\n\
             Start the version once in your launcher so it downloads, then run this again.",
            dir.display(),
            looked.join("\n")
        )));
    }

    if let Some(env) = std::env::var_os("NEPHELE_CLIENT_JAR").map(PathBuf::from) {
        if env.is_file() {
            return Ok(env);
        }
    }

    let mut fallbacks = Vec::new();
    if let Some(dir) = std::env::var_os("NEPHELE_MINECRAFT_DIR").map(PathBuf::from) {
        fallbacks.extend(client_jar_candidates(&dir));
    }
    if let Some(home) = std::env::var_os("APPDATA").map(PathBuf::from) {
        fallbacks.extend(client_jar_candidates(&home.join(".minecraft")));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        fallbacks.extend(client_jar_candidates(&home.join(".minecraft")));
    }
    if let Ok(cwd) = std::env::current_dir() {
        fallbacks.push(cwd.join("assets/vanilla").join(VERSION).join("client.jar"));
    }
    if let Some(found) = fallbacks.iter().find(|p| p.is_file()) {
        return Ok(found.clone());
    }

    let _ = prepared_dir;
    Err(PrepareError::Message(format!(
        "could not find a Minecraft {VERSION} client jar.\n\
         Pass --minecraft-dir pointing at your game directory or Prism instance, or --jar at the jar itself.\n\
         Nephele never downloads the game: it must come from your own copy."
    )))
}
