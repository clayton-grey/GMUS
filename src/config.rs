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
        let data_dir = default_data_dir()?;
        let db_path = db_override.unwrap_or_else(|| data_dir.join("gmus.sqlite3"));
        let art_dir = db_path
            .parent()
            .map(|parent| parent.join("art"))
            .unwrap_or_else(|| data_dir.join("art"));

        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating database directory {}", parent.display()))?;
        }
        fs::create_dir_all(&art_dir)
            .with_context(|| format!("creating cover-art directory {}", art_dir.display()))?;

        Ok(Self {
            data_dir,
            db_path,
            art_dir,
        })
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
