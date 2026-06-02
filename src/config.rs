use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub art_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve(db_override: Option<PathBuf>) -> Result<Self> {
        let paths = Self::resolve_without_creating_dirs(db_override)?;
        paths.ensure_dirs()?;
        Ok(paths)
    }

    pub fn resolve_without_creating_dirs(db_override: Option<PathBuf>) -> Result<Self> {
        let data_dir = default_data_dir()?;
        let db_path = db_override.unwrap_or_else(|| data_dir.join("gmus.sqlite3"));
        let art_dir = db_path
            .parent()
            .map(|parent| parent.join("art"))
            .unwrap_or_else(|| data_dir.join("art"));

        Ok(Self {
            data_dir,
            db_path,
            art_dir,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        if let Some(parent) = self.db_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating database directory {}", parent.display()))?;
        }
        fs::create_dir_all(&self.art_dir)
            .with_context(|| format!("creating cover-art directory {}", self.art_dir.display()))?;
        Ok(())
    }
}

fn default_data_dir() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = home_dir()?;
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("GMUS"));
    }

    if let Some(xdg) = env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg).join("gmus"));
    }

    Ok(home_dir()?.join(".local").join("share").join("gmus"))
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set; use --db to choose an explicit database path")
}

#[cfg(test)]
mod tests {
    use super::AppPaths;

    #[test]
    fn pure_resolution_keeps_art_next_to_db_override_without_creating_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("nested").join("gmus.sqlite3");

        let paths = AppPaths::resolve_without_creating_dirs(Some(db_path.clone())).unwrap();

        assert_eq!(paths.db_path, db_path);
        assert_eq!(paths.art_dir, dir.path().join("nested").join("art"));
        assert!(!dir.path().join("nested").exists());
    }

    #[test]
    fn ensure_dirs_creates_db_parent_and_art_dir() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve_without_creating_dirs(Some(
            dir.path().join("nested").join("gmus.sqlite3"),
        ))
        .unwrap();

        paths.ensure_dirs().unwrap();

        assert!(paths.db_path.parent().unwrap().is_dir());
        assert!(paths.art_dir.is_dir());
    }
}
