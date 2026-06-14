use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::art;
use crate::config::AppPaths;
use crate::db;
use crate::media;

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum ScanOutcome {
    #[default]
    Complete,
    Partial,
    Unavailable,
}

impl ScanOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub outcome: ScanOutcome,
    pub files_seen: usize,
    pub tracks_stored: usize,
    pub art_cached: usize,
    pub files_unchanged: usize,
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
    let report = rescan_canonical_path(conn, paths, &root)?;
    if report.outcome == ScanOutcome::Unavailable {
        anyhow::bail!(
            "scanning {}: {}",
            root.display(),
            report
                .errors
                .first()
                .map(String::as_str)
                .unwrap_or("root became unavailable")
        );
    }
    persist_scanned_root(conn, &root, &report)?;
    Ok((root, report))
}

pub fn update_library_root(
    conn: &Connection,
    paths: &AppPaths,
    root: &Path,
) -> Result<(PathBuf, ScanReport)> {
    let root = match canonical_root(root) {
        Ok(root) => root,
        Err(error) => return Ok((root.to_path_buf(), unavailable_report(root, error))),
    };
    let report = rescan_canonical_path(conn, paths, &root)?;
    persist_scanned_root(conn, &root, &report)?;
    Ok((root, report))
}

fn persist_scanned_root(conn: &Connection, root: &Path, report: &ScanReport) -> Result<()> {
    if report.outcome == ScanOutcome::Unavailable {
        return Ok(());
    }
    db::upsert_library_root(conn, root)?;
    if report.outcome == ScanOutcome::Complete {
        db::mark_library_root_scanned(conn, root)?;
    }
    Ok(())
}

pub(crate) fn rescan_path_deferred_merge(
    conn: &Connection,
    paths: &AppPaths,
    root: &Path,
) -> Result<ScanReport> {
    let root = match canonical_root(root) {
        Ok(root) => root,
        Err(error) => return Ok(unavailable_report(root, error)),
    };
    reconcile_canonical_path(conn, paths, &root)
}

fn rescan_canonical_path(conn: &Connection, paths: &AppPaths, root: &Path) -> Result<ScanReport> {
    let mut report = reconcile_canonical_path(conn, paths, root)?;
    if report.outcome != ScanOutcome::Unavailable {
        report.duplicate_tracks_merged = db::reconcile_renamed_media_items(conn)?;
    }
    Ok(report)
}

fn reconcile_canonical_path(
    conn: &Connection,
    paths: &AppPaths,
    root: &Path,
) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let mut seen_paths = Vec::new();
    scan_inner(conn, paths, root, &mut report, &mut seen_paths, false)?;
    if report.outcome == ScanOutcome::Complete {
        report.files_marked_missing =
            db::mark_locations_missing_under_root_except(conn, root, &seen_paths)?;
    }
    Ok(report)
}

fn unavailable_report(root: &Path, error: anyhow::Error) -> ScanReport {
    ScanReport {
        outcome: ScanOutcome::Unavailable,
        errors: vec![format!("{}: {error:#}", root.display())],
        ..ScanReport::default()
    }
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
    nested: bool,
) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            record_filesystem_error(report, nested);
            report.errors.push(format!(
                "{}: reading filesystem metadata: {error:#}",
                path.display()
            ));
            return Ok(());
        }
    };
    let metadata = if metadata.file_type().is_symlink() {
        match fs::metadata(path) {
            Ok(target) if target.is_file() => target,
            Ok(_) => {
                report.files_skipped += 1;
                return Ok(());
            }
            Err(error) => {
                record_filesystem_error(report, nested);
                report.errors.push(format!(
                    "{}: reading symlink target metadata: {error:#}",
                    path.display()
                ));
                return Ok(());
            }
        }
    } else {
        metadata
    };

    if metadata.is_dir() {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) => {
                record_filesystem_error(report, nested);
                report
                    .errors
                    .push(format!("{}: reading directory: {error:#}", path.display()));
                return Ok(());
            }
        };
        for entry in entries {
            match entry {
                Ok(entry) => scan_inner(conn, paths, &entry.path(), report, seen_paths, true)?,
                Err(error) => {
                    report.outcome = ScanOutcome::Partial;
                    report.errors.push(format!(
                        "reading directory entry in {}: {error:#}",
                        path.display()
                    ));
                }
            }
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
    let stamp = media::file_stamp(&metadata);
    if db::location_scan_is_current(conn, path, stamp)? {
        report.files_unchanged += 1;
        return Ok(());
    }

    match media::read_track_with_stamp(path, stamp) {
        Ok(track) => {
            let stored = db::upsert_track(conn, &track)?;
            report.tracks_stored += 1;
            match art::cache_cover_for_track(&track, &paths.art_dir, stored.media_item_id) {
                Ok(Some(cover_path)) => {
                    db::set_cover_path(conn, stored.media_item_id, &cover_path)?;
                    db::mark_location_scan_current(conn, path)?;
                    report.art_cached += 1;
                }
                Ok(None) => db::mark_location_scan_current(conn, path)?,
                Err(error) => report
                    .errors
                    .push(format!("{}: caching cover art: {error:#}", path.display())),
            }
        }
        Err(error) => {
            report.errors.push(format!("{}: {error:#}", path.display()));
        }
    }

    Ok(())
}

fn record_filesystem_error(report: &mut ScanReport, nested: bool) {
    report.outcome = if nested {
        ScanOutcome::Partial
    } else {
        ScanOutcome::Unavailable
    };
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

        let report = rescan_canonical_path(&conn, &paths, &root).unwrap();

        assert_eq!(report.files_seen, 1);
        assert_eq!(report.files_marked_missing, 0);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(db::library_tracks(&conn).unwrap().len(), 1);
    }

    #[test]
    fn repeated_add_marks_deleted_tracks_missing() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let audio_path = root.join("song.wav");
        write_test_wav(&audio_path);
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        add_library_root(&conn, &paths, &root).unwrap();
        fs::remove_file(&audio_path).unwrap();
        let (_, report) = add_library_root(&conn, &paths, &root).unwrap();

        assert_eq!(report.files_marked_missing, 1);
        assert!(db::library_tracks(&conn).unwrap().is_empty());
    }

    #[test]
    fn repeated_scan_skips_unchanged_audio_metadata_and_artwork() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let audio_path = root.join("song.wav");
        write_test_wav(&audio_path);
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        add_library_root(&conn, &paths, &root).unwrap();
        let stamp = media::file_stamp(&fs::metadata(&audio_path).unwrap());
        assert!(db::location_scan_is_current(&conn, &audio_path, stamp).unwrap());
        let changed_stamp = media::FileStamp {
            modified_at_ns: stamp.modified_at_ns.map(|value| value + 1),
            ..stamp
        };
        assert!(!db::location_scan_is_current(&conn, &audio_path, changed_stamp).unwrap());
        let (_, report) = update_library_root(&conn, &paths, &root).unwrap();

        assert_eq!(report.files_seen, 1);
        assert_eq!(report.files_unchanged, 1);
        assert_eq!(report.tracks_stored, 0);
        assert_eq!(report.art_cached, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_roots_and_audio_paths_round_trip_without_colliding() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(OsString::from_vec(b"music-\xff".to_vec()));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let first = root.join(OsString::from_vec(b"\xfe.wav".to_vec()));
        let second = root.join(OsString::from_vec(b"\xff.wav".to_vec()));
        write_test_wav(&first);
        write_test_wav(&second);
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        add_library_root(&conn, &paths, &root).unwrap();

        let tracks = db::library_tracks(&conn).unwrap();
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().any(|track| track.file_path == first));
        assert!(tracks.iter().any(|track| track.file_path == second));
        let roots = db::active_library_roots(&conn).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].file_path, root);

        let (_, report) = update_library_root(&conn, &paths, &root).unwrap();
        assert_eq!(report.files_unchanged, 2);
    }

    #[test]
    fn stale_scan_version_forces_metadata_and_artwork_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let audio_path = root.join("song.wav");
        write_test_wav(&audio_path);
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));
        add_library_root(&conn, &paths, &root).unwrap();
        conn.execute("UPDATE locations SET scan_version = 0", [])
            .unwrap();

        let (_, report) = update_library_root(&conn, &paths, &root).unwrap();

        assert_eq!(report.files_unchanged, 0);
        assert_eq!(report.tracks_stored, 1);
    }

    #[cfg(unix)]
    #[test]
    fn rescan_preserves_history_after_real_file_rename() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let old_path = root.join("old.wav");
        let new_path = root.join("new.wav");
        write_test_wav(&old_path);
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));
        add_library_root(&conn, &paths, &root).unwrap();
        let original = db::library_tracks(&conn).unwrap().remove(0);
        db::record_play(
            &conn,
            original.media_item_id,
            original.location_id,
            100,
            true,
        )
        .unwrap();
        fs::rename(&old_path, &new_path).unwrap();

        let (_, report) = update_library_root(&conn, &paths, &root).unwrap();

        assert_eq!(report.duplicate_tracks_merged, 1);
        let tracks = db::library_tracks(&conn).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].path, new_path.to_string_lossy());
        assert_eq!(tracks[0].play_count, 1);
    }

    #[test]
    fn artwork_cache_failure_is_reported_without_aborting_scan() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let audio_path = root.join("song.wav");
        write_test_wav(&audio_path);
        fs::write(root.join("cover.jpg"), b"cover").unwrap();
        let art_path = data_dir.path().join("art");
        fs::write(&art_path, b"not a directory").unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(art_path);

        let (_, report) = add_library_root(&conn, &paths, &root).unwrap();

        assert_eq!(report.tracks_stored, 1);
        assert_eq!(report.art_cached, 0);
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("caching cover art"));
        assert_eq!(db::library_tracks(&conn).unwrap().len(), 1);

        let (_, retry) = update_library_root(&conn, &paths, &root).unwrap();
        assert_eq!(retry.files_unchanged, 0);
        assert_eq!(retry.tracks_stored, 1);
        assert_eq!(retry.errors.len(), 1);
    }

    #[test]
    fn nested_filesystem_failure_marks_scan_partial_and_preserves_missing_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let missing_nested_path = root.join("missing").join("song.wav");
        let existing_path = root.join("existing.wav");
        write_test_wav(&existing_path);
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        db::upsert_track(&conn, &test_track_metadata(missing_nested_path.clone())).unwrap();
        let paths = test_paths(data_dir.path().join("art"));
        let mut report = ScanReport::default();
        let mut seen_paths = Vec::new();

        scan_inner(
            &conn,
            &paths,
            &missing_nested_path,
            &mut report,
            &mut seen_paths,
            true,
        )
        .unwrap();
        scan_inner(
            &conn,
            &paths,
            &existing_path,
            &mut report,
            &mut seen_paths,
            true,
        )
        .unwrap();
        if report.outcome == ScanOutcome::Complete {
            report.files_marked_missing =
                db::mark_locations_missing_under_root_except(&conn, &root, &seen_paths).unwrap();
        }

        assert_eq!(report.outcome, ScanOutcome::Partial);
        assert_eq!(report.tracks_stored, 1);
        assert_eq!(report.files_marked_missing, 0);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(db::library_tracks(&conn).unwrap().len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn unavailable_nested_symlink_target_makes_real_scan_partial_and_preserves_track() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let target_path = root.join("target.wav");
        let symlink_path = root.join("linked.wav");
        let sibling_path = root.join("sibling.wav");
        write_test_wav(&target_path);
        std::os::unix::fs::symlink(&target_path, &symlink_path).unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        let complete = reconcile_canonical_path(&conn, &paths, &root).unwrap();
        assert_eq!(complete.outcome, ScanOutcome::Complete);
        assert_eq!(db::library_tracks(&conn).unwrap().len(), 2);

        fs::remove_file(&target_path).unwrap();
        write_test_wav(&sibling_path);
        let partial = reconcile_canonical_path(&conn, &paths, &root).unwrap();

        assert_eq!(partial.outcome, ScanOutcome::Partial);
        assert_eq!(partial.files_marked_missing, 0);
        assert_eq!(partial.errors.len(), 1);
        assert!(partial.errors[0].contains("reading symlink target metadata"));
        let tracks = db::library_tracks(&conn).unwrap();
        assert_eq!(tracks.len(), 3);
        assert!(tracks
            .iter()
            .any(|track| track.path == symlink_path.to_string_lossy()));
        assert!(tracks
            .iter()
            .any(|track| track.path == sibling_path.to_string_lossy()));
    }

    #[test]
    fn root_filesystem_failure_reports_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let missing_root = dir.path().join("missing");
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        let mut report = ScanReport::default();
        let mut seen_paths = Vec::new();
        scan_inner(
            &conn,
            &paths,
            &missing_root,
            &mut report,
            &mut seen_paths,
            false,
        )
        .unwrap();

        assert_eq!(report.outcome, ScanOutcome::Unavailable);
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("reading filesystem metadata"));
    }

    #[test]
    fn deleted_single_file_root_is_unavailable_without_advancing_last_complete_scan() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("song.wav");
        write_test_wav(&audio_path);
        let root = audio_path.canonicalize().unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));
        add_library_root(&conn, &paths, &root).unwrap();
        set_last_scanned_at(&conn, &root, 123);
        fs::remove_file(&root).unwrap();

        let (reported_root, report) = update_library_root(&conn, &paths, &root).unwrap();

        assert_eq!(reported_root, root);
        assert_eq!(report.outcome, ScanOutcome::Unavailable);
        assert_eq!(report.files_marked_missing, 0);
        assert_eq!(last_scanned_at(&conn, &root), Some(123));
        assert_eq!(db::library_tracks(&conn).unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn partial_scan_does_not_advance_last_complete_scan() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let target_path = root.join("target.wav");
        let symlink_path = root.join("linked.wav");
        write_test_wav(&target_path);
        std::os::unix::fs::symlink(&target_path, &symlink_path).unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        let paths = test_paths(data_dir.path().join("art"));
        add_library_root(&conn, &paths, &root).unwrap();
        set_last_scanned_at(&conn, &root, 123);
        fs::remove_file(&target_path).unwrap();

        let (_, report) = update_library_root(&conn, &paths, &root).unwrap();

        assert_eq!(report.outcome, ScanOutcome::Partial);
        assert_eq!(last_scanned_at(&conn, &root), Some(123));
    }

    #[test]
    fn complete_scan_records_last_scanned_at() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        db::upsert_library_root(&conn, &root).unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        let (_, report) = update_library_root(&conn, &paths, &root).unwrap();

        assert_eq!(report.outcome, ScanOutcome::Complete);
        assert!(last_scanned_at(&conn, &root).is_some());
    }

    #[test]
    fn nested_database_failure_still_aborts_scan() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let audio_path = root.join("song.wav");
        write_test_wav(&audio_path);
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        conn.execute_batch(
            r#"
            CREATE TRIGGER fail_location_insert
            BEFORE INSERT ON locations
            BEGIN
                SELECT RAISE(FAIL, 'location insert failed');
            END;
            "#,
        )
        .unwrap();
        let paths = test_paths(data_dir.path().join("art"));

        let error = reconcile_canonical_path(&conn, &paths, &root).unwrap_err();

        assert!(error.to_string().contains("location insert failed"));
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

        let report = reconcile_canonical_path(&conn, &paths, &root).unwrap();

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
            modified_at_ns: Some(1_000_000_000),
            fs_device: None,
            fs_inode: None,
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

    fn last_scanned_at(conn: &Connection, _root: &Path) -> Option<i64> {
        conn.query_row("SELECT last_scanned_at FROM library_roots", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn set_last_scanned_at(conn: &Connection, _root: &Path, timestamp: i64) {
        conn.execute("UPDATE library_roots SET last_scanned_at = ?1", [timestamp])
            .unwrap();
    }

    fn write_test_wav(path: &Path) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&38_u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&8_000_u32.to_le_bytes());
        bytes.extend_from_slice(&16_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&0_i16.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }
}
