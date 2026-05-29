mod art;
mod config;
mod db;
mod media;
mod media_session;
mod player;
mod scanner;
mod tui;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "gmus")]
#[command(about = "A small terminal music player inspired by cmus")]
struct Cli {
    /// Override the SQLite database path.
    #[arg(long)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Scan files or directories into the local metadata/history store.
    Scan {
        /// File or directory to scan.
        path: PathBuf,
    },
    /// Extract or locate cover art for one audio file.
    Art {
        /// Audio file to inspect.
        path: PathBuf,
    },
    /// Print a compact database summary.
    Stats,
    /// Record a play event for a track.
    RecordPlay {
        /// Audio file that was played.
        path: PathBuf,
        /// How much audio was played, in milliseconds.
        #[arg(long, default_value_t = 0)]
        duration_ms: i64,
        /// Whether playback crossed the configured play-count threshold.
        #[arg(long, default_value_t = true)]
        completed: bool,
    },
    /// Play one file through the default lightweight backend.
    Play {
        /// Audio file to play.
        path: PathBuf,
    },
    /// Launch the terminal interface.
    Tui {
        /// Scan a file or directory before launching the TUI.
        path: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = config::AppPaths::resolve(cli.db)?;
    let conn = db::open(&paths.db_path)
        .with_context(|| format!("opening database at {}", paths.db_path.display()))?;

    match cli.command.unwrap_or(Command::Tui { path: None }) {
        Command::Scan { path } => {
            let root = scanner::canonical_root(&path)?;
            let report = scanner::scan_path(&conn, &paths, &root)?;
            db::upsert_library_root(&conn, &root)?;
            db::mark_library_root_scanned(&conn, &root)?;
            println!(
                "scanned {} files, stored {} tracks, cached {} covers, skipped {} files",
                report.files_seen, report.tracks_stored, report.art_cached, report.files_skipped
            );
            if !report.errors.is_empty() {
                println!("{} files had metadata/read errors:", report.errors.len());
                for error in report.errors.iter().take(10) {
                    println!("  {}", error);
                }
                if report.errors.len() > 10 {
                    println!("  ... {} more", report.errors.len() - 10);
                }
            }
        }
        Command::Art { path } => {
            let track = media::read_track(&path)?;
            let stored = db::upsert_track(&conn, &track)?;
            match art::cache_cover_for_track(&track, &paths.art_dir)? {
                Some(cached) => {
                    db::set_cover_path(&conn, stored.media_item_id, &cached)?;
                    println!("{}", cached.display());
                }
                None => println!("no embedded or folder cover art found"),
            }
        }
        Command::Stats => {
            let stats = db::stats(&conn)?;
            println!("data dir: {}", paths.data_dir.display());
            println!("tracks: {}", stats.media_items);
            println!("locations: {}", stats.locations);
            println!("play events: {}", stats.play_events);
            println!("completed plays: {}", stats.completed_plays);
        }
        Command::RecordPlay {
            path,
            duration_ms,
            completed,
        } => {
            let track = media::read_track(&path)?;
            let stored = db::upsert_track(&conn, &track)?;
            db::record_play(
                &conn,
                stored.media_item_id,
                stored.location_id,
                duration_ms,
                completed,
            )?;
            println!(
                "recorded {}play for {}",
                if completed { "completed " } else { "" },
                path.display()
            );
        }
        Command::Play { path } => {
            let track = media::read_track(&path)?;
            let stored = db::upsert_track(&conn, &track)?;
            if let Some(cover_path) = art::cache_cover_for_track(&track, &paths.art_dir)? {
                db::set_cover_path(&conn, stored.media_item_id, &cover_path)?;
            }

            let mut player = player::default_player_backend()?;
            player.load(&path)?;
            player.play()?;
            player.sleep_until_end();

            let mut played_ms = player.position().as_millis() as i64;
            if player.is_finished() {
                if let Some(duration_ms) = track.duration_ms {
                    played_ms = played_ms.max(duration_ms);
                }
            }
            let completed = player::play_count_threshold_met(track.duration_ms, played_ms);
            db::record_play(
                &conn,
                stored.media_item_id,
                stored.location_id,
                played_ms,
                completed,
            )?;
            println!(
                "played {}{}",
                path.display(),
                if completed { " and counted it" } else { "" }
            );
        }
        Command::Tui { path } => {
            if let Some(path) = path {
                let root = scanner::canonical_root(&path)?;
                let report = scanner::scan_path(&conn, &paths, &root)?;
                db::upsert_library_root(&conn, &root)?;
                db::mark_library_root_scanned(&conn, &root)?;
                eprintln!(
                    "scanned {} files, stored {} tracks, cached {} covers, skipped {} files",
                    report.files_seen,
                    report.tracks_stored,
                    report.art_cached,
                    report.files_skipped
                );
            }
            tui::run(&conn, &paths)?
        }
    }

    Ok(())
}
