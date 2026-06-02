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

pub fn add_library_root(
    conn: &Connection,
    paths: &AppPaths,
    root: &Path,
) -> Result<(PathBuf, ScanReport)> {
    let root = canonical_root(root)?;
    let report = scan_canonical_path(conn, paths, &root)?;
    db::upsert_library_root(conn, &root)?;
    db::mark_library_root_scanned(conn, &root)?;
    Ok((root, report))
}

pub fn update_library_root(
    conn: &Connection,
    paths: &AppPaths,
    root: &Path,
) -> Result<(PathBuf, ScanReport)> {
    let root = canonical_root(root)?;
    let report = rescan_canonical_path(conn, paths, &root)?;
    db::upsert_library_root(conn, &root)?;
    db::mark_library_root_scanned(conn, &root)?;
    Ok((root, report))
}

pub fn rescan_path(conn: &Connection, paths: &AppPaths, root: &Path) -> Result<ScanReport> {
    let root = canonical_root(root)?;
    rescan_canonical_path(conn, paths, &root)
}

fn scan_canonical_path(conn: &Connection, paths: &AppPaths, root: &Path) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let mut seen_paths = Vec::new();
    scan_inner(conn, paths, root, &mut report, &mut seen_paths)?;
    Ok(report)
}

fn rescan_canonical_path(conn: &Connection, paths: &AppPaths, root: &Path) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let mut seen_paths = Vec::new();
    scan_inner(conn, paths, root, &mut report, &mut seen_paths)?;
    report.files_marked_missing =
        db::mark_locations_missing_under_root_except(conn, root, &seen_paths)?;
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
    seen_paths: &mut Vec<PathBuf>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading filesystem metadata for {}", path.display()))?;
    let metadata = if metadata.file_type().is_symlink() {
        match fs::metadata(path) {
            Ok(target) if target.is_file() => target,
            _ => {
                report.files_skipped += 1;
                return Ok(());
            }
        }
    } else {
        metadata
    };

    if metadata.is_dir() {
        for entry in
            fs::read_dir(path).with_context(|| format!("reading directory {}", path.display()))?
        {
            let entry = entry?;
            scan_inner(conn, paths, &entry.path(), report, seen_paths)?;
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
    seen_paths.push(path.to_path_buf());

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::TrackMetadata;

    #[test]
    fn rescan_keeps_existing_track_visible_when_audio_metadata_fails() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let audio_path = root.join("song.flac");
        fs::write(&audio_path, b"not really flac").unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        db::upsert_track(&conn, &test_track_metadata(audio_path.clone())).unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        let report = rescan_path(&conn, &paths, &root).unwrap();

        assert_eq!(report.files_seen, 1);
        assert_eq!(report.files_marked_missing, 0);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(db::library_tracks(&conn).unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_directory_symlink_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        let report = scan_canonical_path(&conn, &paths, &root).unwrap();

        assert_eq!(report.files_seen, 0);
        assert_eq!(report.files_skipped, 1);
    }

    fn test_paths(art_dir: PathBuf) -> AppPaths {
        AppPaths {
            data_dir: art_dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("/tmp/gmus-test")),
            db_path: art_dir
                .parent()
                .map(|parent| parent.join("gmus.sqlite3"))
                .unwrap_or_else(|| PathBuf::from("/tmp/gmus-test/gmus.sqlite3")),
            art_dir,
        }
    }

    fn test_track_metadata(path: PathBuf) -> TrackMetadata {
        TrackMetadata {
            path,
            file_size: 10,
            modified_at: Some(1),
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_artist: None,
            album_year: Some(2018),
            release_date: Some("2018-05-11".into()),
            composer: None,
            genre: None,
            track_number: Some(1),
            track_total: Some(10),
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
        }
    }
}
