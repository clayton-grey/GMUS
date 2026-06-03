use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::Connection;

use crate::config::AppPaths;
use crate::db;
use crate::scanner::{self, ScanReport};

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
    for root in &roots {
        let path = PathBuf::from(&root.path);
        match scanner::rescan_path(conn, paths, &path) {
            Ok(root_report) => {
                merge_reports(&mut report, root_report);
                db::mark_library_root_scanned(conn, &path)?;
            }
            Err(error) => {
                report.errors.push(format!("{}: {error:#}", path.display()));
            }
        }
    }

    Ok(LibraryJobResult::AllRoots {
        roots: roots.len(),
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
        LibraryJobResult::AllRoots { roots, report } => format!(
            "updated {} roots, scanned {} files, stored {} tracks, cached {} covers, skipped {}, missing {}, merged {}, errors {}",
            roots,
            report.files_seen,
            report.tracks_stored,
            report.art_cached,
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
        "{action} {}: stored {} tracks, cached {} covers, skipped {}",
        root.display(),
        report.tracks_stored,
        report.art_cached,
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
    report.files_skipped += root_report.files_skipped;
    report.files_marked_missing += root_report.files_marked_missing;
    report.duplicate_tracks_merged += root_report.duplicate_tracks_merged;
    report.errors.extend(root_report.errors);
}
