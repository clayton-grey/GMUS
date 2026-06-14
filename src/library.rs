use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::Connection;

use crate::config::AppPaths;
use crate::db;
use crate::scanner::{self, ScanOutcome, ScanReport};

#[derive(Debug, Clone)]
pub enum LibraryJob {
    AddRoot(PathBuf),
    UpdateRoot(PathBuf),
    UpdateAllRoots,
}

#[derive(Debug)]
pub enum LibraryJobResult {
    Root {
        action: &'static str,
        root: PathBuf,
        report: ScanReport,
    },
    AllRoots {
        roots: usize,
        attempted_roots: usize,
        report: ScanReport,
    },
    NoActiveRoots,
}

impl LibraryJobResult {
    pub fn refreshes_library(&self) -> bool {
        !matches!(self, Self::NoActiveRoots)
    }
}

pub fn add_root(conn: &Connection, paths: &AppPaths, root: &Path) -> Result<LibraryJobResult> {
    let (root, report) = scanner::add_library_root(conn, paths, root)?;
    Ok(LibraryJobResult::Root {
        action: "added",
        root,
        report,
    })
}

pub fn update_root(conn: &Connection, paths: &AppPaths, root: &Path) -> Result<LibraryJobResult> {
    let (root, report) = scanner::update_library_root(conn, paths, root)?;
    Ok(LibraryJobResult::Root {
        action: "updated",
        root,
        report,
    })
}

pub fn update_all_roots(conn: &Connection, paths: &AppPaths) -> Result<LibraryJobResult> {
    let roots = db::active_library_roots(conn)?;
    if roots.is_empty() {
        return Ok(LibraryJobResult::NoActiveRoots);
    }

    let mut report = ScanReport::default();
    let attempted_roots = roots.len();
    let mut successful_roots = 0;
    let mut complete_roots = 0;
    for root in &roots {
        let path = root.file_path.clone();
        let root_report = scanner::rescan_path_deferred_merge(conn, paths, &path)?;
        match root_report.outcome {
            ScanOutcome::Complete => {
                db::mark_library_root_scanned(conn, &path)?;
                successful_roots += 1;
                complete_roots += 1;
            }
            ScanOutcome::Partial => {
                successful_roots += 1;
            }
            ScanOutcome::Unavailable => {}
        }
        merge_reports(&mut report, root_report);
    }
    if successful_roots > 0 {
        report.duplicate_tracks_merged = db::reconcile_renamed_media_items(conn)?;
    }
    report.outcome = if successful_roots == 0 {
        ScanOutcome::Unavailable
    } else if complete_roots == attempted_roots {
        ScanOutcome::Complete
    } else {
        ScanOutcome::Partial
    };

    Ok(LibraryJobResult::AllRoots {
        roots: successful_roots,
        attempted_roots,
        report,
    })
}

pub fn run_job(conn: &Connection, paths: &AppPaths, job: LibraryJob) -> Result<LibraryJobResult> {
    match job {
        LibraryJob::AddRoot(root) => add_root(conn, paths, &root),
        LibraryJob::UpdateRoot(root) => update_root(conn, paths, &root),
        LibraryJob::UpdateAllRoots => update_all_roots(conn, paths),
    }
}

pub fn job_status(result: &LibraryJobResult) -> String {
    match result {
        LibraryJobResult::Root {
            action,
            root,
            report,
        } => scan_status(action, root, report),
        LibraryJobResult::AllRoots {
            roots,
            attempted_roots,
            report,
        } => format!(
            "updated {} of {} roots ({}), scanned {} files, stored {} tracks, cached {} covers, unchanged {}, skipped {}, missing {}, merged {}, errors {}",
            roots,
            attempted_roots,
            report.outcome.label(),
            report.files_seen,
            report.tracks_stored,
            report.art_cached,
            report.files_unchanged,
            report.files_skipped,
            report.files_marked_missing,
            report.duplicate_tracks_merged,
            report.errors.len()
        ),
        LibraryJobResult::NoActiveRoots => String::from("no active library roots; use :add PATH"),
    }
}

fn scan_status(action: &str, root: &Path, report: &ScanReport) -> String {
    let mut status = format!(
        "{action} {} ({}): stored {} tracks, cached {} covers, unchanged {}, skipped {}",
        root.display(),
        report.outcome.label(),
        report.tracks_stored,
        report.art_cached,
        report.files_unchanged,
        report.files_skipped
    );
    if !report.errors.is_empty() {
        status.push_str(&format!(", errors {}", report.errors.len()));
    }
    if report.files_marked_missing > 0 {
        status.push_str(&format!(", missing {}", report.files_marked_missing));
    }
    if report.duplicate_tracks_merged > 0 {
        status.push_str(&format!(", merged {}", report.duplicate_tracks_merged));
    }
    status
}

fn merge_reports(report: &mut ScanReport, root_report: ScanReport) {
    report.files_seen += root_report.files_seen;
    report.tracks_stored += root_report.tracks_stored;
    report.art_cached += root_report.art_cached;
    report.files_unchanged += root_report.files_unchanged;
    report.files_skipped += root_report.files_skipped;
    report.files_marked_missing += root_report.files_marked_missing;
    report.duplicate_tracks_merged += root_report.duplicate_tracks_merged;
    report.errors.extend(root_report.errors);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_all_reports_successful_and_attempted_roots() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let active_root = dir.path().canonicalize().unwrap();
        let missing_root = dir.path().join("missing");
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        db::upsert_library_root(&conn, &active_root).unwrap();
        db::upsert_library_root(&conn, &missing_root).unwrap();
        let paths = test_paths(data_dir.path());

        let result = update_all_roots(&conn, &paths).unwrap();

        let LibraryJobResult::AllRoots {
            roots,
            attempted_roots,
            report,
        } = &result
        else {
            panic!("expected all-roots result");
        };
        assert_eq!(*roots, 1);
        assert_eq!(*attempted_roots, 2);
        assert_eq!(report.outcome, ScanOutcome::Partial);
        assert_eq!(report.errors.len(), 1);
        assert!(job_status(&result).starts_with("updated 1 of 2 roots (partial)"));
    }

    #[test]
    fn statuses_surface_complete_and_partial_outcomes() {
        let root = Path::new("/music");
        let complete = ScanReport::default();
        let partial = ScanReport {
            outcome: ScanOutcome::Partial,
            ..ScanReport::default()
        };
        let unavailable = ScanReport {
            outcome: ScanOutcome::Unavailable,
            ..ScanReport::default()
        };

        assert!(scan_status("updated", root, &complete).contains("(complete)"));
        assert!(scan_status("updated", root, &partial).contains("(partial)"));
        assert!(scan_status("updated", root, &unavailable).contains("(unavailable)"));
    }

    #[test]
    fn update_all_reports_unavailable_when_no_roots_can_be_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let missing_root = dir.path().join("missing");
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        db::upsert_library_root(&conn, &missing_root).unwrap();
        conn.execute("UPDATE library_roots SET last_scanned_at = 123", [])
            .unwrap();
        let paths = test_paths(data_dir.path());

        let result = update_all_roots(&conn, &paths).unwrap();

        let LibraryJobResult::AllRoots {
            roots,
            attempted_roots,
            report,
        } = &result
        else {
            panic!("expected all-roots result");
        };
        assert_eq!(*roots, 0);
        assert_eq!(*attempted_roots, 1);
        assert_eq!(report.outcome, ScanOutcome::Unavailable);
        assert_eq!(report.errors.len(), 1);
        assert!(job_status(&result).starts_with("updated 0 of 1 roots (unavailable)"));
        let last_scanned_at: Option<i64> = conn
            .query_row("SELECT last_scanned_at FROM library_roots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(last_scanned_at, Some(123));
    }

    #[test]
    fn update_all_propagates_database_failures() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let audio_path = root.join("song.wav");
        write_test_wav(&audio_path);
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        db::upsert_library_root(&conn, &root).unwrap();
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
        let paths = test_paths(data_dir.path());

        let error = update_all_roots(&conn, &paths).unwrap_err();

        assert!(error.to_string().contains("location insert failed"));
    }

    fn test_paths(data_dir: &Path) -> AppPaths {
        AppPaths {
            data_dir: data_dir.to_path_buf(),
            db_path: data_dir.join("gmus.sqlite3"),
            art_dir: data_dir.join("art"),
        }
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
        std::fs::write(path, bytes).unwrap();
    }
}
