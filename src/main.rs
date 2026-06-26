// objc 0.2 macros used by the macOS overlay helper still expand cfg(cargo-clippy).
#![allow(unexpected_cfgs)]

mod art;
mod config;
mod db;
mod integration;
mod library;
mod media;
mod player;
mod scanner;
mod tui;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{ArgAction, Parser, Subcommand};

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
        #[arg(
            long,
            default_value_t = true,
            action = ArgAction::Set,
            num_args = 0..=1,
            default_missing_value = "true",
            value_name = "BOOL"
        )]
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
    if integration::run_helper_if_requested()? {
        return Ok(());
    }

    let cli = Cli::parse();
    let paths = config::AppPaths::resolve(cli.db)?;
    let conn = db::open(&paths.db_path)
        .with_context(|| format!("opening database at {}", paths.db_path.display()))?;
    db::prepare_art_cache(&conn, &paths.art_dir)?;

    match cli.command.unwrap_or(Command::Tui { path: None }) {
        Command::Scan { path } => {
            let library::LibraryJobResult::Root { report, .. } =
                library::add_root(&conn, &paths, &path)?
            else {
                unreachable!("add_root always scans one root");
            };
            println!(
                "{} scan: scanned {} files, stored {} tracks, cached {} covers, unchanged {} files, skipped {} files",
                report.outcome.label(),
                report.files_seen,
                report.tracks_stored,
                report.art_cached,
                report.files_unchanged,
                report.files_skipped
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
            match art::materialize_cover_for_audio_path(
                &track.path,
                &paths.art_dir,
                stored.media_item_id,
            )? {
                Some(art_path) => {
                    db::set_cover_path(&conn, stored.media_item_id, &art_path)?;
                    println!("{}", art_path.display());
                }
                None => {
                    db::clear_cover_path(&conn, stored.media_item_id)?;
                    println!("no embedded or folder cover art found");
                }
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

            let mut player = player::default_player_backend()?;
            player.load_and_play(&path)?;
            player.sleep_until_end();
            if player.output_failed() {
                anyhow::bail!("audio output disconnected while playing {}", path.display());
            }

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
                let library::LibraryJobResult::Root { report, .. } =
                    library::add_root(&conn, &paths, &path)?
                else {
                    unreachable!("add_root always scans one root");
                };
                eprintln!(
                    "{} scan: scanned {} files, stored {} tracks, cached {} covers, unchanged {} files, skipped {} files",
                    report.outcome.label(),
                    report.files_seen,
                    report.tracks_stored,
                    report.art_cached,
                    report.files_unchanged,
                    report.files_skipped
                );
            }
            tui::run(&conn, &paths)?
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn record_play_accepts_explicit_incomplete_status() {
        let cli =
            Cli::try_parse_from(["gmus", "record-play", "/tmp/song.flac", "--completed=false"])
                .unwrap();

        let Some(Command::RecordPlay { completed, .. }) = cli.command else {
            panic!("expected record-play command");
        };
        assert!(!completed);
    }
}
