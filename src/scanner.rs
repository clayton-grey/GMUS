use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::art;
use crate::config::AppPaths;
use crate::db;
use crate::media;

#[derive(Debug, Default)]
pub struct ScanReport {
    pub files_seen: usize,
    pub tracks_stored: usize,
    pub art_cached: usize,
    pub files_skipped: usize,
    pub files_marked_missing: usize,
    pub duplicate_tracks_merged: usize,
    pub errors: Vec<String>,
}

pub fn scan_path(conn: &Connection, paths: &AppPaths, root: &Path) -> Result<ScanReport> {
    let root = canonical_root(root)?;
    let mut report = ScanReport::default();
    scan_inner(conn, paths, &root, &mut report)?;
    Ok(report)
}

pub fn rescan_path(conn: &Connection, paths: &AppPaths, root: &Path) -> Result<ScanReport> {
    let root = canonical_root(root)?;
    let mut report = ScanReport {
        files_marked_missing: db::mark_locations_missing_under_root(conn, &root)?,
        ..ScanReport::default()
    };
    scan_inner(conn, paths, &root, &mut report)?;
    report.duplicate_tracks_merged = db::merge_similar_media_items(conn)?;
    Ok(report)
}

pub fn canonical_root(root: &Path) -> Result<PathBuf> {
    root.canonicalize()
        .with_context(|| format!("resolving scan path {}", root.display()))
}

fn scan_inner(
    conn: &Connection,
    paths: &AppPaths,
    path: &Path,
    report: &mut ScanReport,
) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("reading filesystem metadata for {}", path.display()))?;

    if metadata.is_dir() {
        for entry in
            fs::read_dir(path).with_context(|| format!("reading directory {}", path.display()))?
        {
            let entry = entry?;
            scan_inner(conn, paths, &entry.path(), report)?;
        }
        return Ok(());
    }

    if !metadata.is_file() {
        report.files_skipped += 1;
        return Ok(());
    }

    report.files_seen += 1;
    if !media::is_audio_path(path) {
        report.files_skipped += 1;
        return Ok(());
    }

    match media::read_track(path) {
        Ok(track) => {
            let stored = db::upsert_track(conn, &track)?;
            report.tracks_stored += 1;
            if let Some(cover_path) = art::cache_cover_for_track(&track, &paths.art_dir)? {
                db::set_cover_path(conn, stored.media_item_id, &cover_path)?;
                report.art_cached += 1;
            }
        }
        Err(error) => {
            report.errors.push(format!("{}: {error:#}", path.display()));
        }
    }

    Ok(())
}
